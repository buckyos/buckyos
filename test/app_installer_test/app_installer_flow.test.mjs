import test, { after } from 'node:test'
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { createHash, createPrivateKey, randomBytes, sign as signDetached } from 'node:crypto'
import { access, readFile, rm } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

import { buckyos, ndm_proxy, ndn, TaskManagerClient } from 'buckyos/node'
import {
  buildPikgProject,
  configurePikgSample,
  copyPikgSample,
  dockerTarget,
} from './pikg_sample_builder.mjs'

const execFileAsync = promisify(execFile)

const NODE_GATEWAY_URL =
  getEnv('BUCKYOS_NODE_GATEWAY_URL') ??
  'http://127.0.0.1:3180'
const SYSTEM_CONFIG_URL =
  getEnv('BUCKYOS_SYSTEM_CONFIG_URL') ??
  `${NODE_GATEWAY_URL}/kapi/system_config`
const CONTROL_PANEL_URL =
  getEnv('BUCKYOS_CONTROL_PANEL_URL') ??
  `${NODE_GATEWAY_URL}/kapi/control-panel`
const VERIFY_HUB_URL =
  getEnv('BUCKYOS_VERIFY_HUB_URL') ??
  `${NODE_GATEWAY_URL}/kapi/verify-hub`
const TASK_MANAGER_URL =
  getEnv('BUCKYOS_TASK_MANAGER_URL') ??
  `${NODE_GATEWAY_URL}/kapi/task-manager`
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
const INSTALL_EVIDENCE_TIMEOUT_MS = Number(
  getEnv('BUCKYOS_TEST_INSTALL_EVIDENCE_TIMEOUT_MS') ?? '120000',
)
const UNINSTALL_AFTER_INSTALL =
  getEnv('BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL') === '1'
const tempPaths = new Set()
const dockerImages = new Set()

let sdkContextPromise = null
let versionCounter = Math.floor(Date.now() / 1000) % 60000

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
  versionCounter = (versionCounter + 1) % 60000
  return `0.1.${versionCounter}`
}

function isKeyNotFoundError(error) {
  const message = String(error?.message ?? error)
  return /key.?not.?found|not.?found|KeyNotFound/i.test(message)
}

// v0.5: AppDoc requires `did` (App DID); derive via the frozen rule did:bns:{app_name}.{owner_id}.
function deriveAppDid(appId) {
  const ownerIdPart = OWNER_DID.split(':').pop()
  return `did:bns:${appId}.${ownerIdPart}`
}

function appIdFromName(appId) {
  const ownerIdPart = OWNER_DID.split(':').pop()
  return `${appId}.${ownerIdPart}.bns.did`
}

function appInstanceId(appId, ownerUserId) {
  return `${appIdFromName(appId)}@${ownerUserId}`
}

function runtimeContainerName(appInstance) {
  const runtimeKey = createHash('sha256').update(appInstance, 'utf8').digest('hex')
  return `buckyos-app-${runtimeKey}`
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

function getSdkContext() {
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

async function getNodeSigningCredential() {
  const identity = JSON.parse(
    await readFile('/opt/buckyos/etc/node_identity.json', 'utf8'),
  )
  const deviceHost = String(identity.device_did ?? '').replace(/^did:web:/, '')
  return {
    path: path.join(
      '/opt/buckyos/security',
      deviceHost,
      'authentication.private.pem',
    ),
    kid: identity.device_name ?? getEnv('BUCKYOS_TEST_NODE_KID') ?? 'ood1',
  }
}

async function createOwnerSignedLoginJwt(userId) {
  // 本地可用的 zone owner 或当前 device 信任凭证。
  const candidates = [
    { path: '/opt/buckyos/etc/.buckycli/user_private_key.pem', kid: 'root' },
    await getNodeSigningCredential(),
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
    const header = {
      alg: 'EdDSA',
      kid: candidate.kid,
    }
    const payload = {
      appid: TEST_APP_ID,
      userid: userId,
      sub: userId,
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
  return null
}

async function loginWithAppClient() {
  const ownerSignedJwt = await createOwnerSignedLoginJwt(TEST_USER_ID)
  if (!ownerSignedJwt) {
    throw new Error('No local owner or device signing credential is available')
  }
  const accountInfo = {
    session_token: ownerSignedJwt,
    user_id: TEST_USER_ID,
  }

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
  const task = await waitForTaskStatus(taskId, [
    'Completed',
    'Failed',
    'Canceled',
  ])
  return { status: task.status, task, taskId: task.id }
}

async function waitForTask(taskId) {
  const { status, task, taskId: completedTaskId } = await waitForTaskResult(taskId)
  assert.equal(
    status,
    'Completed',
    `Task ${completedTaskId} failed with status=${status}, message=${task.message ?? '<none>'}`,
  )
  return task
}

async function sleep(ms) {
  if (ms <= 0) {
    return
  }
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForCondition(check, { timeoutMs = 30000, intervalMs = 1000 } = {}) {
  const deadline = Date.now() + timeoutMs

  while (Date.now() <= deadline) {
    if (await check()) {
      return true
    }
    await sleep(intervalMs)
  }

  return await check()
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

async function callControlPanel(method, params) {
  const ctx = await getSdkContext()
  return ctx.controlPanelRpc.call(method, params)
}

async function buildAndStagePikg(projectDir) {
  const result = await buildPikgProject(projectDir)
  const digest = result.pack.pikg_digest.replace(/^sha256:/, '')
  assert.match(digest, /^[0-9a-f]{64}$/)
  assert.equal(result.info.app.app_doc_object_id, result.pack.app_doc_object_id)
  assert.equal(result.info.app.did, result.appDoc.did)

  const ctx = await getSdkContext()
  const pikgBytes = await readFile(result.pack.pikg_path)
  const sourceObjId = ndn.ChunkId.fromMix256Result(
    pikgBytes.byteLength,
    ndn.sha256Bytes(pikgBytes),
  ).toString()
  const ndmClient = ndm_proxy.createNdmProxyClient({
    endpoint: NODE_GATEWAY_URL,
    sessionToken: ctx.sessionToken,
    fetcher: (request, init) => {
      const target = typeof request === 'string'
        ? request.replaceAll('%3A', ':').replaceAll('%3a', ':')
        : request
      return fetch(target, init)
    },
  })
  await ndmClient.putChunk(sourceObjId, pikgBytes)
  const staging = await callControlPanel('apps.staging.finalize', {
    source_obj_id: sourceObjId,
    purpose: 'install',
  })
  assert.equal(staging.pikg_digest, digest)
  assert.equal(staging.size, pikgBytes.byteLength)

  return {
    app_did: result.appDoc.did,
    app_doc_id: result.pack.app_doc_object_id,
    app_doc: result.appDoc,
    pikg_handle: staging.handle,
    pikg_digest: digest,
  }
}

function escapeResolverSegment(raw) {
  return `${raw}`.replaceAll('%', '%25').replaceAll('/', '%2F')
}

// resolver/cache/* 的写入受 RBAC 限制（kernel/root 级）。fixture 种入使用
// 本机 node/device key 铸 root 会话（等价于 DV 管理注入，Installer 自身
// 永不写这些 key）。
let seedRpcClient = null
async function getSeedRpcClient() {
  if (seedRpcClient) {
    return seedRpcClient
  }
  const credential = await getNodeSigningCredential()
  const keyPem = (await readFile(credential.path, 'utf8')).trim()
  const now = Math.floor(Date.now() / 1000)
  const header = { alg: 'EdDSA', kid: credential.kid }
  const payload = {
    appid: 'node-daemon',
    userid: 'root',
    sub: 'root',
    iss: credential.kid,
    jti: String(now),
    session: now,
    exp: now + 3600,
  }
  const input = `${encodeJwtPart(header)}.${encodeJwtPart(payload)}`
  const signature = signDetached(
    null,
    Buffer.from(input),
    createPrivateKey(keyPem),
  ).toString('base64url')
  seedRpcClient = new buckyos.kRPCClient(SYSTEM_CONFIG_URL, `${input}.${signature}`)
  return seedRpcClient
}

async function writeConfig(key, value) {
  const rpc = await getSeedRpcClient()
  await rpc.call('sys_config_set', { key, value })
}

// 测试环境通过 zone resolver 数据面（RBAC 管控的 KV）显式种入
// `(App DID, "app")` 解析证据；Installer 只消费 resolver 结果。
async function seedResolverCache(appDid, appDocJson) {
  const documentVersion = appDocJson.exp - 5 * 365 * 24 * 60 * 60
  assert.ok(Number.isSafeInteger(documentVersion) && documentVersion > 0)
  const base = `resolver/cache/${escapeResolverSegment(appDid)}/app`
  await writeConfig(`${base}/doc`, JSON.stringify(appDocJson))
  await writeConfig(
    `${base}/state`,
    JSON.stringify({
      document_status: 'active',
      document_version: documentVersion,
      updated_by: 'app_installer_test',
    }),
  )
}

function taskStatus(task) {
  if (task.phase === 'Terminal') {
    if (task.outcome === 'Succeeded') {
      return 'Completed'
    }
    if (task.outcome === 'Canceled') {
      return 'Canceled'
    }
    return 'Failed'
  }
  if (task.phase === 'Waiting') {
    return task.wait_reason?.kind === 'Authorization'
      ? 'WaitingForApproval'
      : 'Paused'
  }
  if (task.phase === 'Paused') {
    return 'Paused'
  }
  if (task.phase === 'Running') {
    return 'Running'
  }
  return 'Pending'
}

function installTaskView(task) {
  return {
    ...task,
    id: task.task_id,
    status: taskStatus(task),
    data: task.result ?? task.progress ?? task.input,
  }
}

async function waitForTaskStatus(taskId, statuses, { timeoutMs = 120000, intervalMs = 1000 } = {}) {
  const ctx = await getSdkContext()
  const stableTaskId = String(taskId)
  const deadline = Date.now() + timeoutMs
  let task = null
  while (Date.now() <= deadline) {
    task = installTaskView(await ctx.taskManager.getTask(stableTaskId))
    if (statuses.includes(task.status)) {
      return task
    }
    if (['Failed', 'Canceled'].includes(task.status)) {
      return task
    }
    await sleep(intervalMs)
  }
  throw new Error(
    `task ${stableTaskId} did not reach ${statuses.join('/')} in time, last=${task?.status}, message=${task?.message ?? '<none>'}`,
  )
}

// v4 安装闭环：apps.inspect -> fingerprint-bound apps.submit -> Completed。
async function installPikgToCompletion({ stagingHandle, expectOfflineReady = true }) {
  const source = { kind: 'local_pikg', staging_handle: stagingHandle }
  const options = { policy: 'NORMAL' }
  const inspection = await callControlPanel('apps.inspect', {
    source,
    options,
  })
  const plan = inspection.plan
  assert.ok(plan, 'inspect must return an install plan')
  if (expectOfflineReady) {
    assert.equal(
      inspection.status?.readiness?.install,
      'OFFLINE_READY',
      `pikg install should be offline ready, got ${JSON.stringify(inspection.status?.readiness)}`,
    )
  }

  const startResult = await callControlPanel('apps.submit', {
    source,
    owner_user_id: plan.owner_user_id,
    target: plan.target,
    install_params: plan.install_params,
    options,
    plan,
    approved_plan_fingerprint: plan.plan_fingerprint,
    idempotency_key: createRunId('install'),
  })
  assert.ok(startResult.task_id, 'apps.submit should return task_id')
  const taskId = startResult.task_id

  const task = await waitForTask(taskId)
  assert.ok(
    task.data?.result?.completed_at,
    'completed install task must carry a structured result',
  )
  return { task, plan }
}

async function uninstallApp({ appInstanceId, userId, removeData = false }) {
  const result = await callControlPanel('apps.uninstall', {
    selector: appInstanceId,
    owner_user_id: userId,
    data_disposition: removeData ? 'delete' : 'retain',
    idempotency_key: createRunId('uninstall'),
  })

  assert.ok(result.task_id, 'uninstall should return task_id')
  return waitForTask(result.task_id)
}

async function stageStaticWebFixture() {
  const appId = createRunId('cp-web')
  const version = buildVersion()
  const { tempRoot, projectDir } = await copyPikgSample('static-web', 'cp-web')
  tempPaths.add(tempRoot)
  await configurePikgSample(projectDir, {
    appId,
    version,
    ownerDid: OWNER_DID,
  })

  return {
    appId,
    version,
    projectDir,
    tempRoot,
    specPath: (userId) =>
      `users/${userId}/apps/${appIdFromName(appId)}/spec`,
    specId: (userId) => appInstanceId(appId, userId),
    binPath: () => path.join('/opt/buckyos/bin', `web.${appIdFromName(appId)}`),
  }
}

async function stageAgentFixture() {
  const appId = createRunId('cp-agent')
  const version = buildVersion()
  const { tempRoot, projectDir } = await copyPikgSample('agent', 'cp-agent')
  tempPaths.add(tempRoot)
  await configurePikgSample(projectDir, {
    appId,
    version,
    ownerDid: OWNER_DID,
  })

  return {
    appId,
    version,
    projectDir,
    tempRoot,
    specPath: (userId) =>
      `users/${userId}/apps/${appIdFromName(appId)}/spec`,
    specId: (userId) => appInstanceId(appId, userId),
    pidFile: (userId) =>
      path.join('/opt/buckyos/data/home', userId, '.local', 'share', appIdFromName(appId), '.opendan.pid'),
  }
}

async function stageDockerFixture() {
  const appId = createRunId('cp-docker')
  const version = buildVersion()
  const { tempRoot, projectDir } = await copyPikgSample('docker', 'cp-docker')
  tempPaths.add(tempRoot)
  const imageName = `local/${appId}:${version}-${dockerTarget().tagArch}`

  await execQuiet(
    'docker',
    [
      'build',
      '--build-arg',
      `BASE_IMAGE=${DOCKER_BASE_IMAGE}`,
      '-t',
      imageName,
      path.join(projectDir, 'image'),
    ],
  )
  dockerImages.add(imageName)
  await configurePikgSample(projectDir, {
    appId,
    version,
    ownerDid: OWNER_DID,
    dockerImage: imageName,
  })

  return {
    appId,
    version,
    projectDir,
    tempRoot,
    imageName,
    specPath: (userId) =>
      `users/${userId}/apps/${appIdFromName(appId)}/spec`,
    specId: (userId) => appInstanceId(appId, userId),
    containerName: (userId) => runtimeContainerName(appInstanceId(appId, userId)),
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
    await access(targetPath)
    return true
  } catch (_error) {
    return false
  }
}

after(async () => {
  if (UNINSTALL_AFTER_INSTALL) {
    for (const imageName of [...dockerImages]) {
      await removeDockerImage(imageName)
    }
  }

  for (const dir of [...tempPaths]) {
    await cleanupTempDir(dir)
  }
})

test('app_installer local PIKG lifecycle', async (t) => {
  const ctx = await getSdkContext()
  const userId = ctx.accountInfo.user_id

  await t.test('static web app PIKG build + install', async () => {
    const fixture = await stageStaticWebFixture()

    try {
      const published = await buildAndStagePikg(fixture.projectDir)
      assert.equal(published.app_did, deriveAppDid(fixture.appId))

      // v0.5: 显式种 resolver 证据 -> 本地 pikg 安装 -> 确认 -> 严格等完成。
      await seedResolverCache(published.app_did, published.app_doc)
      const { task: installTask, plan } = await installPikgToCompletion({
        stagingHandle: published.pikg_handle,
      })
      const installedAppInstanceId = plan.app_instance_id

      const spec = await readConfigJson(fixture.specPath(userId))
      assert.equal(spec.app_instance_id, installedAppInstanceId)
      assert.equal(spec.app_doc.version, fixture.version)
      assert.equal(spec.app_doc.did, published.app_did)
      assert.equal(spec.app_doc.selector_type, 'static')
      assert.ok(
        isInstalledSpecState(spec.state),
        `static web spec should be in an installed state, got ${spec.state}`,
      )
      assert.deepEqual(
        spec.spec_config.expose_config.www?.route?.sub_hostname ?? [],
        [fixture.appId],
      )

      // install_record（D3）与 proof 顺序：完成后 record=installed 且
      // task result 带 record key；proof id（Repo 可用时）回填进 record。
      const installRecordKey = `users/${userId}/apps/${appIdFromName(fixture.appId)}/install_record`
      const installRecord = await readConfigJson(installRecordKey)
      assert.equal(installRecord.state, 'installed')
      assert.equal(installRecord.task_id, installTask.id)
      assert.equal(installRecord.app.did, published.app_did)
      assert.equal(
        installTask.data?.result?.install_record_key,
        installRecordKey,
      )
      if (installTask.data?.result?.proof_id) {
        assert.equal(installRecord.proof_id, installTask.data.result.proof_id)
      }

      assert.equal(
        await waitForCondition(() => fileExists(fixture.binPath()), {
          timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS,
        }),
        true,
      )

      if (UNINSTALL_AFTER_INSTALL) {
        await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })

        const deletedSpec = await readConfigJson(fixture.specPath(userId))
        assert.equal(deletedSpec.state, 'deleted')
        assert.equal(
          await waitForCondition(
            () => fileExists(fixture.binPath()).then((exists) => !exists),
            { timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS },
          ),
          true,
        )
      }
    } finally {
      await cleanupTempDir(fixture.tempRoot)
    }
  })

  await t.test(
    'agent app PIKG build + install',
    {
      skip:
        getEnv('BUCKYOS_TEST_SKIP_DOCKER') === '1' || !(await isDockerAvailable()),
    },
    async () => {
      const fixture = await stageAgentFixture()

      try {
        const published = await buildAndStagePikg(fixture.projectDir)

        await seedResolverCache(published.app_did, published.app_doc)
        const { plan } = await installPikgToCompletion({
          stagingHandle: published.pikg_handle,
        })
        const installedAppInstanceId = plan.app_instance_id

        const spec = await readConfigJson(fixture.specPath(userId))
        assert.equal(spec.app_instance_id, installedAppInstanceId)
        assert.equal(spec.app_doc.version, fixture.version)
        assert.ok(
          isInstalledSpecState(spec.state),
          `agent spec should be in an installed state, got ${spec.state}`,
        )
        assert.equal(spec.app_doc.app_type, 'agent')

        const instances = await listServiceInstances(fixture.specId(userId))
        assert.ok(instances.length >= 1, 'agent install should create a started instance')
        assert.equal(
          await waitForCondition(() => fileExists(fixture.pidFile(userId)), {
            timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS,
          }),
          true,
        )

        if (UNINSTALL_AFTER_INSTALL) {
          await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })

          const deletedSpec = await readConfigJson(fixture.specPath(userId))
          assert.equal(deletedSpec.state, 'deleted')
          assert.equal(
            await waitForCondition(
              () => fileExists(fixture.pidFile(userId)).then((exists) => !exists),
              { timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS },
            ),
            true,
          )
        }
      } finally {
        await cleanupTempDir(fixture.tempRoot)
      }
    },
  )

  await t.test(
    'docker app PIKG build + install',
    {
      skip:
        getEnv('BUCKYOS_TEST_SKIP_DOCKER') === '1' || !(await isDockerAvailable()),
    },
    async () => {
      const fixture = await stageDockerFixture()

      try {
        const published = await buildAndStagePikg(fixture.projectDir)

        await seedResolverCache(published.app_did, published.app_doc)
        const { plan } = await installPikgToCompletion({
          stagingHandle: published.pikg_handle,
        })
        const installedAppInstanceId = plan.app_instance_id

        const spec = await readConfigJson(fixture.specPath(userId))
        assert.equal(spec.app_instance_id, installedAppInstanceId)
        assert.equal(spec.app_doc.version, fixture.version)
        assert.ok(
          isInstalledSpecState(spec.state),
          `docker spec should be in an installed state, got ${spec.state}`,
        )
        assert.equal(spec.app_doc.app_type, 'dapp')
        assert.equal(await isContainerRunning(fixture.containerName(userId)), true)

        if (UNINSTALL_AFTER_INSTALL) {
          await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })

          const deletedSpec = await readConfigJson(fixture.specPath(userId))
          assert.equal(deletedSpec.state, 'deleted')
          assert.equal(await isContainerRunning(fixture.containerName(userId)), false)
        }
      } finally {
        if (UNINSTALL_AFTER_INSTALL) {
          await removeDockerImage(fixture.imageName)
        }
        await cleanupTempDir(fixture.tempRoot)
      }
    },
  )
})
