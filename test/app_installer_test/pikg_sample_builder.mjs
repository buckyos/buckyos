import { execFile } from 'node:child_process'
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)
const TEST_ROOT = path.dirname(fileURLToPath(import.meta.url))

export const PIKG_SAMPLES_ROOT = path.join(TEST_ROOT, 'pikg_samples')
const NPX = process.platform === 'win32' ? 'npx.cmd' : 'npx'

export function dockerTarget() {
  switch (process.arch) {
    case 'x64':
      return {
        key: 'amd64_docker_image',
        arch: 'x86_64',
        tagArch: 'amd64',
      }
    case 'arm64':
      return {
        key: 'aarch64_docker_image',
        arch: 'aarch64',
        tagArch: 'aarch64',
      }
    default:
      throw new Error(`Unsupported Docker sample architecture: ${process.arch}`)
  }
}

export async function runCommand(command, args, options = {}) {
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

async function runPikgTool(args) {
  const { stdout } = await runCommand(NPX, [
    'buckyos',
    '--non-interactive',
    ...args,
  ])
  let envelope
  try {
    envelope = JSON.parse(stdout)
  } catch (error) {
    throw new Error(`buckyos-tool returned invalid JSON: ${error.message}\n${stdout}`)
  }
  if (envelope?.ok !== true || !envelope.data) {
    throw new Error(`buckyos-tool failed: ${JSON.stringify(envelope)}`)
  }
  return envelope.data
}

export async function copyPikgSample(name, prefix = `pikg-${name}`) {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), `${prefix}-`))
  const projectDir = path.join(tempRoot, name)
  await cp(path.join(PIKG_SAMPLES_ROOT, name), projectDir, { recursive: true })
  return { tempRoot, projectDir }
}

export async function configurePikgSample(
  projectDir,
  { appId, version, ownerDid, dockerImage },
) {
  const metaDir = path.join(projectDir, 'dapp_meta')
  const appPath = path.join(metaDir, 'app.json')
  const pikgPath = path.join(metaDir, 'pikg.json')
  const app = JSON.parse(await readFile(appPath, 'utf8'))
  const pikg = JSON.parse(await readFile(pikgPath, 'utf8'))
  const ownerName = ownerDid.split(':').pop()

  Object.assign(app, {
    did: `did:bns:${appId}.${ownerName}`,
    name: appId,
    version,
    owner: ownerDid,
    author: ownerDid,
  })
  pikg.pikg_file = `${appId}-${version}.pikg`

  if (dockerImage) {
    const target = dockerTarget()
    pikg.sub_pkgs = {
      [target.key]: {
        selector: { os: 'linux', arch: target.arch },
        required: true,
        source: { type: 'docker-image', image: dockerImage },
      },
    }
  }

  await writeFile(appPath, `${JSON.stringify(app, null, 2)}\n`)
  await writeFile(pikgPath, `${JSON.stringify(pikg, null, 2)}\n`)

  if (app.categories[0] === 'agent') {
    const agentDocPath = path.join(
      projectDir,
      'agent',
      'dist',
      'agent_doc.json',
    )
    const agentDoc = JSON.parse(await readFile(agentDocPath, 'utf8'))
    agentDoc.id = `did:opendan:${appId}`
    agentDoc.name = appId
    await writeFile(agentDocPath, `${JSON.stringify(agentDoc, null, 2)}\n`)
  }
}

export async function buildPikgProject(projectDir) {
  const metaDir = path.join(projectDir, 'dapp_meta')
  const build = await runPikgTool(['pikg', 'build', metaDir])
  const pack = await runPikgTool(['pikg', 'pack', build.dist_dir])
  const info = await runPikgTool(['pikg', 'info', pack.pikg_path])
  const appDoc = JSON.parse(
    await readFile(path.join(build.dist_dir, 'APPDOC.json'), 'utf8'),
  )

  if (pack.validation !== 'passed' || info.valid !== true) {
    throw new Error(`PIKG validation failed: ${JSON.stringify({ pack, info })}`)
  }

  return { build, pack, info, appDoc }
}
