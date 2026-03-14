import test, { after } from 'node:test'
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { createPrivateKey, randomBytes, sign as signDetached } from 'node:crypto'
import { cp, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

import { buckyos, RuntimeType, TaskManagerClient } from 'buckyos/node'

const execFileAsync = promisify(execFile)

const TEST_ROOT = path.dirname(fileURLToPath(import.meta.url))
const FIXTURES_ROOT = path.join(TEST_ROOT, 'fixtures')
const TEMPLATES_ROOT = path.join(FIXTURES_ROOT, 'templates')

const SYSTEM_CONFIG_URL =
  getEnv('BUCKYOS_SYSTEM_CONFIG_URL') ??
  'http://127.0.0.1:3200/kapi/system_config'
const CONTROL_PANEL_URL =
  getEnv('BUCKYOS_CONTROL_PANEL_URL') ??
  'http://127.0.0.1:4020/kapi/control-panel'
const VERIFY_HUB_URL =
  getEnv('BUCKYOS_VERIFY_HUB_URL') ??
  'http://127.0.0.1:3300/kapi/verify-hub'
const TASK_MANAGER_URL =
  getEnv('BUCKYOS_TASK_MANAGER_URL') ??
  'http://127.0.0.1:3380/kapi/task-manager'
const TEST_APP_ID = 'control-panel'
const TEST_USER_ID =
  getEnv('BUCKYOS_TEST_USER_ID') ??
  'devtest'
const OWNER_DID =
  getEnv('BUCKYOS_TEST_OWNER_DID') ??
  'did:bns:root'
const DOCKER_BASE_IMAGE =
  getEnv('BUCKYOS_TEST_DOCKER_BASE_IMAGE') ??
  'busybox:1.36.1'

const tempPaths = new Set()
const dockerImages = new Set()

let sdkContextPromise = null

function getEnv(name) {
  const value = process.env[name]
  if (typeof value !== 'string') {
    return null
  }
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

function createRunId(prefix) {
  return `${prefix}-${Date.now().toString(36)}-${randomBytes(3).toString('hex')}`
}

function buildVersion() {
  return `0.1.${Math.floor(Date.now() / 1000)}`
}

function isKeyNotFoundError(error) {
  const message = String(error?.message ?? error)
  return /key.?not.?found|not.?found|KeyNotFound/i.test(message)
}

function mapDockerArch() {
  switch (process.arch) {
    case 'x64':
      return 'amd64_docker_image'
    case 'arm64':
      return 'aarch64_docker_image'
    default:
      throw new Error(`Unsupported docker publish arch: ${process.arch}`)
  }
}

function buildMetaFields() {
  const now = Math.floor(Date.now() / 1000)
  return {
    create_time: now,
    last_update_time: now,
    exp: now + 30 * 24 * 60 * 60,
  }
}

function replacePlaceholders(value, tokens) {
  if (typeof value === 'string') {
    return Object.entries(tokens).reduce(
      (result, [key, tokenValue]) => result.replaceAll(`__${key}__`, String(tokenValue)),
      value,
    )
  }

  if (Array.isArray(value)) {
    return value.map((item) => replacePlaceholders(item, tokens))
  }

  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, replacePlaceholders(item, tokens)]),
    )
  }

  return value
}

async function loadTemplate(name, tokens) {
  const templatePath = path.join(TEMPLATES_ROOT, name)
  const raw = await readFile(templatePath, 'utf8')
  return replacePlaceholders(JSON.parse(raw), tokens)
}

async function ensureTempDir(prefix) {
  const dir = await mkdtemp(path.join(os.tmpdir(), `${prefix}-`))
  tempPaths.add(dir)
  return dir
}

async function cleanupTempDir(dir) {
  try {
    await rm(dir, { recursive: true, force: true })
  } finally {
    tempPaths.delete(dir)
  }
}

async function execQuiet(command, args, options = {}) {
  try {
    return await execFileAsync(command, args, options)
  } catch (error) {
    const stdout = error?.stdout ? `\nstdout:\n${error.stdout}` : ''
    const stderr = error?.stderr ? `\nstderr:\n${error.stderr}` : ''
    throw new Error(
      `Command failed: ${command} ${args.join(' ')}${stdout}${stderr}`,
    )
  }
}

async function probeRpc(url, method, params = {}) {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      method,
      params,
      sys: [1],
    }),
  })

  if (!response.ok) {
    throw new Error(`${url} probe failed: ${response.status} ${response.statusText}`)
  }
}

async function getSdkContext() {
  if (!sdkContextPromise) {
    sdkContextPromise = initSdkContext()
  }
  return sdkContextPromise
}

async function initSdkContext() {
  await probeRpc(SYSTEM_CONFIG_URL, 'sys_config_get', { key: 'boot/config' })
  await probeRpc(CONTROL_PANEL_URL, 'auth.logout', {})
  const accountInfo = await loginWithAppClient()

  if (!accountInfo?.session_token) {
    throw new Error('login did not return a session_token')
  }

  const sessionToken = accountInfo.session_token
  const controlPanelRpc = new buckyos.kRPCClient(CONTROL_PANEL_URL, sessionToken)
  const systemConfigRpc = new buckyos.kRPCClient(SYSTEM_CONFIG_URL, sessionToken)
  const taskManagerRpc = new buckyos.kRPCClient(TASK_MANAGER_URL, sessionToken)
  const taskManager = new TaskManagerClient(taskManagerRpc)
  await ensurePublishDependencies(systemConfigRpc)

  return {
    accountInfo,
    sessionToken,
    controlPanelRpc,
    systemConfigRpc,
    taskManager,
  }
}

function encodeJwtPart(value) {
  return Buffer.from(JSON.stringify(value)).toString('base64url')
}

async function createOwnerSignedLoginJwt(userId) {
  const ownerKeyPath = '/opt/buckyos/etc/.buckycli/user_private_key.pem'
  if (!(await fileExists(ownerKeyPath))) {
    return null
  }

  const keyPem = (await readFile(ownerKeyPath, 'utf8')).trim()
  if (!keyPem) {
    return null
  }

  const now = Math.floor(Date.now() / 1000)
  const header = {
    alg: 'EdDSA',
    kid: 'root',
  }
  const payload = {
    appid: TEST_APP_ID,
    userid: userId,
    sub: userId,
    iss: 'root',
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

async function loginWithAppClient() {
  const ownerSignedJwt = await createOwnerSignedLoginJwt(TEST_USER_ID)

  await buckyos.initBuckyOS(TEST_APP_ID, {
    appId: TEST_APP_ID,
    runtimeType: RuntimeType.AppClient,
    zoneHost: '',
    defaultProtocol: 'https://',
    systemConfigServiceUrl: SYSTEM_CONFIG_URL,
    privateKeySearchPaths: [
      '/opt/buckyos/etc/.buckycli',
      '/opt/buckyos/etc',
      '/opt/buckyos',
      `${process.env.HOME ?? ''}/.buckycli`,
      `${process.env.HOME ?? ''}/.buckyos`,
    ],
  })

  const accountInfo = ownerSignedJwt
    ? {
        session_token: ownerSignedJwt,
        user_id: TEST_USER_ID,
      }
    : await buckyos.login()

  if (!accountInfo?.session_token) {
    throw new Error('AppClient login did not return a session_token')
  }

  const verifyHubRpc = new buckyos.kRPCClient(VERIFY_HUB_URL)
  const tokenPair = await verifyHubRpc.call('login_by_jwt', {
    type: 'jwt',
    jwt: accountInfo.session_token,
  })

  if (!tokenPair?.session_token) {
    throw new Error('verify-hub login_by_jwt did not return a session_token')
  }

  return {
    ...accountInfo,
    user_id: ownerSignedJwt ? TEST_USER_ID : accountInfo.user_id,
    session_token: tokenPair.session_token,
    refresh_token: tokenPair.refresh_token,
  }
}

async function waitForTaskResult(taskId) {
  const ctx = await getSdkContext()
  const numericTaskId = Number(taskId)
  const status = await ctx.taskManager.waitForTaskEnd(numericTaskId)
  const task = await ctx.taskManager.getTask(numericTaskId)
  return { status, task, numericTaskId }
}

async function waitForTask(taskId) {
  const { status, task, numericTaskId } = await waitForTaskResult(taskId)
  assert.equal(
    status,
    'Completed',
    `Task ${numericTaskId} failed with status=${status}, message=${task.message ?? '<none>'}`,
  )
  return task
}

async function ensurePublishDependencies(systemConfigRpc) {
  try {
    const result = await systemConfigRpc.call('sys_config_get', {
      key: 'services/repo-service/info',
    })

    if (typeof result === 'string' && result.trim()) {
      return
    }

    if (result && typeof result.value === 'string' && result.value.trim()) {
      return
    }
  } catch (error) {
    if (!isKeyNotFoundError(error)) {
      throw error
    }
  }

  throw new Error(
    [
      'app.publish requires repo-service, but `services/repo-service/info` is missing in system_config.',
      'Provision and start repo-service before running this test suite.',
    ].join(' '),
  )
}

function normalizeConfigValue(response) {
  if (response == null) {
    return null
  }

  if (typeof response === 'string') {
    return response
  }

  if (typeof response.value === 'string') {
    return response.value
  }

  if (typeof response === 'object') {
    return response
  }

  return null
}

async function readConfigJson(key) {
  const ctx = await getSdkContext()
  const response = await ctx.systemConfigRpc.call('sys_config_get', { key })
  const normalized = normalizeConfigValue(response)

  if (normalized == null) {
    throw new Error(`system_config key \`${key}\` returned null`)
  }

  if (typeof normalized === 'string') {
    return JSON.parse(normalized)
  }

  return normalized
}

async function readConfigJsonOrNull(key) {
  try {
    return await readConfigJson(key)
  } catch (error) {
    if (isKeyNotFoundError(error)) {
      return null
    }
    throw error
  }
}

function isInstalledSpecState(state) {
  return ['new', 'deployed', 'running'].includes(String(state ?? '').toLowerCase())
}

async function listConfigChildren(key) {
  const ctx = await getSdkContext()
  try {
    const response = await ctx.systemConfigRpc.call('sys_config_list', { key })
    return Array.isArray(response) ? response : []
  } catch (error) {
    if (isKeyNotFoundError(error)) {
      return []
    }
    throw error
  }
}

async function listServiceInstances(specId) {
  const baseKey = `services/${specId}/instances`
  const nodeIds = await listConfigChildren(baseKey)
  const instances = []

  for (const nodeId of nodeIds) {
    const instance = await readConfigJsonOrNull(`${baseKey}/${nodeId}`)
    if (instance) {
      instances.push(instance)
    }
  }

  return instances
}

function hasActiveInstance(instances) {
  return instances.some((instance) =>
    instance?.state === 'started' || instance?.state === 'deploying',
  )
}

async function callControlPanel(method, params) {
  const ctx = await getSdkContext()
  return ctx.controlPanelRpc.call(method, params)
}

async function publishApp({ appType, localDir, appDoc }) {
  const result = await callControlPanel('app.publish', {
    app_type: appType,
    local_dir: localDir,
    app_doc: appDoc,
  })

  assert.equal(result.ok, true)
  assert.ok(result.obj_id, 'publish should return obj_id')
  return result
}

async function installApp({ appId, version }) {
  const result = await callControlPanel('apps.install', {
    app_id: appId,
    version,
  })

  assert.ok(result.task_id, 'install should return task_id')
  return waitForTask(result.task_id)
}

async function installAppAllowFailure({ appId, version }) {
  const result = await callControlPanel('apps.install', {
    app_id: appId,
    version,
  })

  assert.ok(result.task_id, 'install should return task_id')
  return waitForTaskResult(result.task_id)
}

async function uninstallApp({ appId, removeData = false }) {
  const result = await callControlPanel('apps.uninstall', {
    app_id: appId,
    remove_data: removeData,
  })

  assert.ok(result.task_id, 'uninstall should return task_id')
  return waitForTask(result.task_id)
}

async function safeUninstall(appId) {
  try {
    await uninstallApp({ appId, removeData: false })
  } catch (_error) {
    // ignore cleanup failures for partially-installed cases
  }
}

async function stageStaticWebFixture() {
  const appId = createRunId('cp-web')
  const version = buildVersion()
  const localDir = await ensureTempDir('cp-web')
  await cp(path.join(FIXTURES_ROOT, 'static-web'), localDir, { recursive: true })

  const appDoc = await loadTemplate('static-web.app_doc.json', {
    APP_ID: appId,
    VERSION: version,
    OWNER_DID,
    WEB_PKG_ID: `${appId}-web#${version}`,
  })

  Object.assign(appDoc, buildMetaFields())

  return {
    appId,
    version,
    localDir,
    appDoc,
    specPath: (userId) => `users/${userId}/apps/${appId}/spec`,
    specId: (userId) => `${appId}@${userId}`,
  }
}

async function stageAgentFixture() {
  const appId = createRunId('cp-agent')
  const version = buildVersion()
  const localDir = await ensureTempDir('cp-agent')
  await cp(path.join(FIXTURES_ROOT, 'agent'), localDir, { recursive: true })

  const agentDoc = replacePlaceholders(
    JSON.parse(await readFile(path.join(TEMPLATES_ROOT, 'agent_doc.json'), 'utf8')),
    {
      APP_ID: appId,
    },
  )
  await writeFile(
    path.join(localDir, 'agent_doc.json'),
    `${JSON.stringify(agentDoc, null, 2)}\n`,
  )

  const appDoc = await loadTemplate('agent.app_doc.json', {
    APP_ID: appId,
    VERSION: version,
    OWNER_DID,
    AGENT_PKG_ID: `${appId}-agent#${version}`,
  })

  Object.assign(appDoc, buildMetaFields())

  return {
    appId,
    version,
    localDir,
    appDoc,
    specPath: (userId) => `users/${userId}/agents/${appId}/spec`,
    specId: (userId) => `${appId}@${userId}`,
    pidFile: (userId) =>
      path.join('/opt/buckyos/data/home', userId, '.local', 'share', appId, '.opendan.pid'),
  }
}

async function stageDockerFixture() {
  const appId = createRunId('cp-docker')
  const version = buildVersion()
  const localDir = await ensureTempDir('cp-docker')
  await cp(path.join(FIXTURES_ROOT, 'docker'), localDir, { recursive: true })

  const dockerArchKey = mapDockerArch()
  const imageName = `local/${appId}:e2e`
  const imageTarPath = path.join(localDir, `${dockerArchKey}.tar`)

  await execQuiet(
    'docker',
    [
      'build',
      '--build-arg',
      `BASE_IMAGE=${DOCKER_BASE_IMAGE}`,
      '-t',
      imageName,
      localDir,
    ],
  )
  dockerImages.add(imageName)
  await execQuiet('docker', ['save', '-o', imageTarPath, imageName])

  const appDoc = await loadTemplate('docker.app_doc.json', {
    APP_ID: appId,
    VERSION: version,
    OWNER_DID,
  })

  Object.assign(appDoc, buildMetaFields())
  appDoc.pkg_list = {
    [dockerArchKey]: {
      pkg_id: `${appId}-img#${version}`,
      docker_image_name: imageName,
    },
  }
  appDoc.deps = {
    [`${appId}-img`]: version,
  }

  return {
    appId,
    version,
    localDir,
    appDoc,
    imageName,
    specPath: (userId) => `users/${userId}/apps/${appId}/spec`,
    specId: (userId) => `${appId}@${userId}`,
    containerName: (userId) => `${userId}-${appId}`,
  }
}

async function isDockerAvailable() {
  try {
    await execQuiet('docker', ['version', '--format', '{{.Server.Version}}'])
    return true
  } catch (_error) {
    return false
  }
}

async function isContainerRunning(containerName) {
  const { stdout } = await execQuiet('docker', [
    'ps',
    '--filter',
    `name=^/${containerName}$`,
    '--format',
    '{{.Names}}',
  ])
  return stdout
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .includes(containerName)
}

async function removeDockerImage(imageName) {
  try {
    await execQuiet('docker', ['image', 'rm', '-f', imageName])
  } catch (_error) {
    // ignore cleanup failures
  } finally {
    dockerImages.delete(imageName)
  }
}

async function fileExists(targetPath) {
  try {
    await readFile(targetPath)
    return true
  } catch (_error) {
    return false
  }
}

after(async () => {
  if (sdkContextPromise) {
    buckyos.logout(false)
  }

  for (const imageName of [...dockerImages]) {
    await removeDockerImage(imageName)
  }

  for (const dir of [...tempPaths]) {
    await cleanupTempDir(dir)
  }
})

test('app_installer local publish lifecycle', async (t) => {
  const ctx = await getSdkContext()
  const userId = ctx.accountInfo.user_id

  await t.test('static web app publish + install + uninstall', async () => {
    const fixture = await stageStaticWebFixture()
    let installed = false

    try {
      await publishApp({
        appType: 'web',
        localDir: fixture.localDir,
        appDoc: fixture.appDoc,
      })

      const installTask = await installApp({
        appId: fixture.appId,
        version: fixture.version,
      })
      installed = true

      const spec = await readConfigJson(fixture.specPath(userId))
      assert.equal(spec.app_doc.name, fixture.appId)
      assert.equal(spec.app_doc.version, fixture.version)
      assert.equal(spec.app_doc.selector_type, 'static')
      assert.equal(spec.state, 'new')
      assert.deepEqual(installTask.data?.instance ?? null, null)
      assert.deepEqual(
        spec.install_config.expose_config.www?.sub_hostname ?? [],
        [fixture.appId],
      )

      await uninstallApp({ appId: fixture.appId, removeData: false })

      const deletedSpec = await readConfigJson(fixture.specPath(userId))
      assert.equal(deletedSpec.state, 'deleted')
      installed = false
    } finally {
      if (installed) {
        await safeUninstall(fixture.appId)
      }
      await cleanupTempDir(fixture.localDir)
    }
  })

  await t.test('agent app publish + install + uninstall', async () => {
    const fixture = await stageAgentFixture()
    let installed = false

    try {
      await publishApp({
        appType: 'agent',
        localDir: fixture.localDir,
        appDoc: fixture.appDoc,
      })

      const installTask = await installApp({
        appId: fixture.appId,
        version: fixture.version,
      })
      installed = true

      const spec = await readConfigJson(fixture.specPath(userId))
      assert.equal(spec.app_doc.name, fixture.appId)
      assert.equal(spec.app_doc.version, fixture.version)
      assert.ok(
        isInstalledSpecState(spec.state),
        `agent spec should be in an installed state, got ${spec.state}`,
      )
      assert.equal(spec.app_doc.categories[0], 'agent')

      const instances = await listServiceInstances(fixture.specId(userId))
      assert.ok(instances.length >= 1, 'agent install should create a started instance')
      assert.ok(
        installTask.data?.instance?.state === 'started',
        'agent install task should report a started instance',
      )
      assert.equal(installTask.data?.instance?.state, 'started')
      assert.equal(await fileExists(fixture.pidFile(userId)), true)

      await uninstallApp({ appId: fixture.appId, removeData: false })

      const deletedSpec = await readConfigJson(fixture.specPath(userId))
      assert.equal(deletedSpec.state, 'deleted')
      assert.equal(await fileExists(fixture.pidFile(userId)), false)
      installed = false
    } finally {
      if (installed) {
        await safeUninstall(fixture.appId)
      }
      await cleanupTempDir(fixture.localDir)
    }
  })

  await t.test(
    'docker app publish + install + uninstall',
    { skip: !(await isDockerAvailable()) },
    async () => {
      const fixture = await stageDockerFixture()
      let installed = false

      try {
        await publishApp({
          appType: 'dapp',
          localDir: fixture.localDir,
          appDoc: fixture.appDoc,
        })

        const installTask = await installAppAllowFailure({
          appId: fixture.appId,
          version: fixture.version,
        })

        const spec = await readConfigJson(fixture.specPath(userId))
        assert.equal(spec.app_doc.name, fixture.appId)
        assert.equal(spec.app_doc.version, fixture.version)
        assert.ok(
          isInstalledSpecState(spec.state),
          `docker spec should be in an installed state, got ${spec.state}`,
        )
        assert.equal(spec.app_doc.categories[0], 'dapp')
        assert.equal(await isContainerRunning(fixture.containerName(userId)), true)
        assert.ok(
          installTask.status === 'Completed' ||
            (installTask.status === 'Failed' &&
              `${installTask.task.message ?? ''}`.includes('Timed out waiting for app')),
          `docker install should either complete or only fail on control-plane readiness timeout, got status=${installTask.status}, message=${installTask.task.message ?? '<none>'}`,
        )
        installed = true

        await uninstallApp({ appId: fixture.appId, removeData: false })

        const deletedSpec = await readConfigJson(fixture.specPath(userId))
        assert.equal(deletedSpec.state, 'deleted')
        assert.equal(await isContainerRunning(fixture.containerName(userId)), false)
        installed = false
      } finally {
        if (installed) {
          await safeUninstall(fixture.appId)
        }
        await removeDockerImage(fixture.imageName)
        await cleanupTempDir(fixture.localDir)
      }
    },
  )
})
