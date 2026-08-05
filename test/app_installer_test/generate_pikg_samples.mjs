import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { createHash, createPrivateKey, sign as signDetached } from 'node:crypto'
import {
  access,
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

import { buckyos } from 'buckyos/node'

const execFileAsync = promisify(execFile)
const TEST_ROOT = path.dirname(fileURLToPath(import.meta.url))
const FIXTURES_ROOT = path.join(TEST_ROOT, 'fixtures')
const TEMPLATES_ROOT = path.join(FIXTURES_ROOT, 'templates')
const OUTPUT_ROOT = path.resolve(
  process.env.BUCKYOS_PIKG_OUTPUT_DIR ?? path.join(TEST_ROOT, 'pikg_samples'),
)

const SYSTEM_CONFIG_URL =
  process.env.BUCKYOS_SYSTEM_CONFIG_URL ??
  'http://127.0.0.1:3200/kapi/system_config'
const CONTROL_PANEL_URL =
  process.env.BUCKYOS_CONTROL_PANEL_URL ??
  'http://127.0.0.1:4020/kapi/control-panel'
const VERIFY_HUB_URL =
  process.env.BUCKYOS_VERIFY_HUB_URL ??
  'http://127.0.0.1:3300/kapi/verify-hub'
const TEST_USER_ID = process.env.BUCKYOS_TEST_USER_ID ?? 'devtest'
const OWNER_DID = process.env.BUCKYOS_TEST_OWNER_DID ?? 'did:bns:root'
const NODE_KID = process.env.BUCKYOS_TEST_NODE_KID ?? 'ood1'
const DOCKER_BASE_IMAGE =
  process.env.BUCKYOS_TEST_DOCKER_BASE_IMAGE ?? 'busybox:1.36.1'
const VERSION = '0.1.0'

function appPackageNamespace(appId) {
  const ownerIdPart = OWNER_DID.split(':').pop()
  return `${ownerIdPart}_${appId}`
}

function packageEnvQualifier() {
  const osName =
    process.platform === 'darwin'
      ? 'apple'
      : process.platform === 'win32'
        ? 'windows'
        : process.platform
  const archName = process.arch === 'x64' ? 'amd64' : process.arch === 'arm64' ? 'aarch64' : process.arch
  return `nightly-${osName}-${archName}`
}

function appPackageName(appId, role, qualifier = packageEnvQualifier()) {
  return `${qualifier}.${appPackageNamespace(appId)}-${role}`
}

const execQuiet = async (command, args, options = {}) => {
  try {
    return await execFileAsync(command, args, {
      maxBuffer: 16 * 1024 * 1024,
      ...options,
    })
  } catch (error) {
    const stdout = error?.stdout ? `\nstdout:\n${error.stdout}` : ''
    const stderr = error?.stderr ? `\nstderr:\n${error.stderr}` : ''
    throw new Error(`Command failed: ${command} ${args.join(' ')}${stdout}${stderr}`)
  }
}

const fileExists = async (targetPath) => {
  try {
    await access(targetPath)
    return true
  } catch {
    return false
  }
}

const encodeJwtPart = (value) =>
  Buffer.from(JSON.stringify(value)).toString('base64url')

async function createOwnerSignedLoginJwt() {
  const nodeIdentity = JSON.parse(
    await readFile('/opt/buckyos/etc/node_identity.json', 'utf8'),
  )
  const deviceHost = String(nodeIdentity.device_did ?? '').replace(
    /^did:web:/,
    '',
  )
  const candidates = [
    { path: '/opt/buckyos/etc/.buckycli/user_private_key.pem', kid: 'root' },
    {
      path: `/opt/buckyos/security/${deviceHost}/authentication.private.pem`,
      kid: nodeIdentity.device_name ?? NODE_KID,
    },
  ]

  for (const candidate of candidates) {
    if (!(await fileExists(candidate.path))) {
      continue
    }
    const keyPem = (await readFile(candidate.path, 'utf8')).trim()
    if (!keyPem) {
      continue
    }

    const now = Math.floor(Date.now() / 1000)
    const header = { alg: 'EdDSA', kid: candidate.kid }
    const payload = {
      appid: 'control-panel',
      userid: TEST_USER_ID,
      sub: TEST_USER_ID,
      iss: candidate.kid,
      jti: String(now),
      session: now,
      exp: now + 5 * 60,
    }
    const signingInput = `${encodeJwtPart(header)}.${encodeJwtPart(payload)}`
    const signature = signDetached(
      null,
      Buffer.from(signingInput),
      createPrivateKey(keyPem),
    ).toString('base64url')
    return `${signingInput}.${signature}`
  }

  throw new Error('No local owner or node private key is available for app.publish')
}

async function createControlPanelClient() {
  const loginJwt = await createOwnerSignedLoginJwt()

  const verifyHub = new buckyos.kRPCClient(VERIFY_HUB_URL)
  const tokenPair = await verifyHub.call('login_by_jwt', {
    type: 'jwt',
    jwt: loginJwt,
  })
  assert.ok(tokenPair?.session_token, 'verify-hub did not return a session token')

  const systemConfig = new buckyos.kRPCClient(
    SYSTEM_CONFIG_URL,
    tokenPair.session_token,
  )
  const repoInfo = await systemConfig.call('sys_config_get', {
    key: 'services/repo-service/info',
  })
  assert.ok(repoInfo, 'app.publish requires a running repo-service')

  return new buckyos.kRPCClient(CONTROL_PANEL_URL, tokenPair.session_token)
}

function replacePlaceholders(value, tokens) {
  if (typeof value === 'string') {
    return Object.entries(tokens).reduce(
      (result, [key, tokenValue]) =>
        result.replaceAll(`__${key}__`, String(tokenValue)),
      value,
    )
  }
  if (Array.isArray(value)) {
    return value.map((item) => replacePlaceholders(item, tokens))
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        replacePlaceholders(key, tokens),
        replacePlaceholders(item, tokens),
      ]),
    )
  }
  return value
}

async function loadTemplate(name, tokens) {
  const raw = await readFile(path.join(TEMPLATES_ROOT, name), 'utf8')
  return replacePlaceholders(JSON.parse(raw), tokens)
}

function deriveAppDid(appId) {
  return `did:bns:${appId}.${OWNER_DID.split(':').pop()}`
}

function addStableMeta(appDoc) {
  return {
    ...appDoc,
    create_time: 1_800_000_000,
    last_update_time: 1_800_000_000,
    exp: 0,
  }
}

function dockerArchKey() {
  if (process.arch === 'x64') {
    return 'amd64_docker_image'
  }
  if (process.arch === 'arm64') {
    return 'aarch64_docker_image'
  }
  throw new Error(`Unsupported Docker sample architecture: ${process.arch}`)
}

async function prepareSamples(tempRoot) {
  const staticAppId = 'pikg-static-web'
  const staticPackageName = appPackageName(staticAppId, 'web')
  const staticDoc = addStableMeta(
    await loadTemplate('static-web.app_doc.json', {
      APP_ID: staticAppId,
      APP_DID: deriveAppDid(staticAppId),
      VERSION,
      OWNER_DID,
      WEB_PKG_ID: `${staticPackageName}#${VERSION}`,
      WEB_PKG_NAME: staticPackageName,
    }),
  )
  staticDoc.show_name = 'PIKG Static Web Fixture'

  const scriptAppId = 'pikg-script-host'
  const scriptPackageName = appPackageName(scriptAppId, 'script')
  const scriptDoc = addStableMeta(
    await loadTemplate('script-host.app_doc.json', {
      APP_ID: scriptAppId,
      APP_DID: deriveAppDid(scriptAppId),
      VERSION,
      OWNER_DID,
      SCRIPT_PKG_ID: `${scriptPackageName}#${VERSION}`,
      SCRIPT_PKG_NAME: scriptPackageName,
    }),
  )

  const dockerAppId = 'pikg-docker'
  const dockerDir = path.join(tempRoot, 'docker')
  await cp(path.join(FIXTURES_ROOT, 'docker'), dockerDir, { recursive: true })
  const archKey = dockerArchKey()
  const imageName = `local/${dockerAppId}:${VERSION}-${process.arch}`
  await execQuiet('docker', [
    'build',
    '--build-arg',
    `BASE_IMAGE=${DOCKER_BASE_IMAGE}`,
    '-t',
    imageName,
    dockerDir,
  ])
  await execQuiet('docker', [
    'save',
    '-o',
    path.join(dockerDir, `${archKey}.tar`),
    imageName,
  ])

  const dockerDoc = addStableMeta(
    await loadTemplate('docker.app_doc.json', {
      APP_ID: dockerAppId,
      APP_DID: deriveAppDid(dockerAppId),
      VERSION,
      OWNER_DID,
    }),
  )
  dockerDoc.show_name = 'PIKG Docker Fixture'
  const dockerArchName = process.arch === 'x64' ? 'amd64' : 'aarch64'
  const dockerPackageName = appPackageName(
    dockerAppId,
    'image',
    `nightly-linux-${dockerArchName}`,
  )
  dockerDoc.pkg_list = {
    [archKey]: {
      pkg_id: `${dockerPackageName}#${VERSION}`,
      docker_image_name: imageName,
    },
  }
  dockerDoc.deps = { [dockerPackageName]: VERSION }

  return {
    imageName,
    samples: [
      {
        name: 'static-web',
        appType: 'web',
        localDir: path.join(FIXTURES_ROOT, 'static-web'),
        appDoc: staticDoc,
        packageKey: 'web',
      },
      {
        name: 'script-host',
        appType: 'dapp',
        localDir: path.join(FIXTURES_ROOT, 'script-host'),
        appDoc: scriptDoc,
        packageKey: 'script',
      },
      {
        name: 'docker',
        appType: 'dapp',
        localDir: dockerDir,
        appDoc: dockerDoc,
        packageKey: archKey,
      },
    ],
  }
}

async function publishSample(client, sample) {
  const result = await client.call('app.publish', {
    app_type: sample.appType,
    local_dir: sample.localDir,
    app_doc: sample.appDoc,
  })
  assert.equal(result?.ok, true)
  assert.equal(result?.app_did, sample.appDoc.did)
  assert.ok(result?.pikg_path)
  assert.ok(result?.pikg_digest)
  return result
}

async function sha256File(filePath) {
  const bytes = await readFile(filePath)
  return createHash('sha256').update(bytes).digest('hex')
}

async function readZipJson(filePath, entry) {
  const { stdout } = await execQuiet('unzip', ['-p', filePath, entry])
  return JSON.parse(stdout)
}

async function verifyOutput(filePath, sample, published) {
  await execQuiet('unzip', ['-tqq', filePath])
  const digest = await sha256File(filePath)
  assert.equal(digest, published.pikg_digest)

  const appDoc = await readZipJson(filePath, 'APPDOC.json')
  const packageMeta = await readZipJson(filePath, 'PACKAGE_META.json')
  assert.equal(appDoc.did, sample.appDoc.did)
  assert.equal(appDoc.name, sample.appDoc.name)
  assert.equal(packageMeta['@schema'], 'buckyos.pikg.package-meta.v1')
  assert.ok(appDoc.pkg_list[sample.packageKey]?.pkg_objid)
  const contentEntries = Object.entries(packageMeta.content_index).filter(
    ([, entry]) => entry.sub_pkg_name === sample.packageKey,
  )
  assert.equal(contentEntries.length, 1)
  const [[contentId, contentEntry]] = contentEntries
  const { stdout: contentBytes } = await execQuiet(
    'unzip',
    ['-p', filePath, contentEntry.path],
    { encoding: 'buffer' },
  )
  assert.equal(contentBytes.length, contentEntry.size)
  assert.equal(
    contentId,
    `sha256:${createHash('sha256').update(contentBytes).digest('hex')}`,
  )

  return {
    file: path.basename(filePath),
    bytes: (await stat(filePath)).size,
    sha256: digest,
    pikg_handle: published.pikg_handle,
    app_did: published.app_did,
    app_doc_id: published.app_doc_id,
    app_type: sample.appType,
    package_key: sample.packageKey,
  }
}

async function main() {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'pikg-samples-'))
  let imageName = null

  try {
    const prepared = await prepareSamples(tempRoot)
    imageName = prepared.imageName
    const client = await createControlPanelClient()
    await mkdir(OUTPUT_ROOT, { recursive: true })

    const manifest = {
      schema: 'buckyos.pikg.samples.v1',
      generated_at: new Date().toISOString(),
      samples: [],
    }

    for (const sample of prepared.samples) {
      const published = await publishSample(client, sample)
      const outputPath = path.join(OUTPUT_ROOT, `${sample.name}.pikg`)
      await copyFile(published.pikg_path, outputPath)
      manifest.samples.push(await verifyOutput(outputPath, sample, published))
      process.stdout.write(
        `${sample.name}: ${outputPath} (sha256:${published.pikg_digest})\n`,
      )
    }

    await writeFile(
      path.join(OUTPUT_ROOT, 'manifest.json'),
      `${JSON.stringify(manifest, null, 2)}\n`,
    )
  } finally {
    if (imageName) {
      await execQuiet('docker', ['image', 'rm', '-f', imageName]).catch(() => {})
    }
    await rm(tempRoot, { recursive: true, force: true })
  }
}

await main()
