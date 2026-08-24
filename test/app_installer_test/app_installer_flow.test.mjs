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
const SSO_ZONE_BASE_URL = getEnv('BUCKYOS_TEST_ZONE_BASE_URL')
const SSO_PASSWORD = getEnv('BUCKYOS_TEST_ADMIN_PASSWORD')
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

function hashPassword(username, password, nonce) {
  const original = createHash('sha256')
    .update(`${password}${username}.buckyos`, 'utf8')
    .digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

function decodeJwtPayload(token) {
  const segments = token.split('.')
  assert.equal(segments.length, 3, 'SSO session token must be a JWT')
  return JSON.parse(Buffer.from(segments[1], 'base64url').toString('utf8'))
}

function responseSetCookies(response) {
  if (typeof response.headers.getSetCookie === 'function') {
    return response.headers.getSetCookie()
  }
  const combined = response.headers.get('set-cookie')
  return combined ? [combined] : []
}

function cookieValue(setCookies, name) {
  const prefix = `${name}=`
  const header = setCookies.find((value) => value.startsWith(prefix))
  if (!header) return null
  return header.slice(prefix.length).split(';', 1)[0] || null
}

function appOriginForHost(appHostName) {
  assert.ok(SSO_ZONE_BASE_URL, 'App SSO DV requires a zone URL')
  const appUrl = new URL(SSO_ZONE_BASE_URL)
  appUrl.hostname = `${appHostName}.${appUrl.hostname}`
  return appUrl.origin
}

function buildVersion() {
  versionCounter = (versionCounter + 1) % 60000
  return `0.1.${versionCounter}`
}

function isKeyNotFoundError(error) {
  const message = String(error?.message ?? error)
  return /key.?not.?found|not.?found|KeyNotFound|returned null/i.test(message)
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

function runtimeContainerName(appHostName) {
  return `buckyos-app-${appHostName}`
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
    target: { kind: 'system', service_id: 'control-panel' },
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
  return ['new', 'deployed', 'running', 'stopped'].includes(String(state ?? '').toLowerCase())
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

async function loginStaticAppThroughSso({ appHostName, appId, appInstanceId, ownerUserId }) {
  assert.ok(SSO_ZONE_BASE_URL && SSO_PASSWORD, 'App SSO DV requires zone URL and password')
  const ctx = await getSdkContext()
  const appUrl = new URL(appOriginForHost(appHostName))
  appUrl.pathname = '/fixture'
  appUrl.search = '?source=app-sso-dv'
  const nonce = Date.now()
  const login = await callControlPanel('auth.login', {
    username: ctx.accountInfo.user_id,
    password: hashPassword(ctx.accountInfo.user_id, SSO_PASSWORD, nonce),
    appid: appId,
    redirect_url: appUrl.toString(),
    login_nonce: nonce,
  })
  assert.equal(typeof login.sso_nonce, 'number')

  const callbackUrl = new URL('/sso_callback', appUrl.origin)
  callbackUrl.searchParams.set('nonce', String(login.sso_nonce))
  callbackUrl.searchParams.set('redirect_url', appUrl.toString())
  const callback = await fetch(callbackUrl, { redirect: 'manual' })
  assert.equal(callback.status, 302)
  assert.equal(callback.headers.get('location'), appUrl.toString())
  const setCookies = responseSetCookies(callback)
  const sessionToken = cookieValue(setCookies, 'buckyos_session_token')
  const refreshToken = cookieValue(setCookies, 'buckyos_refresh_token')
  assert.ok(sessionToken, 'App SSO callback omitted session cookie')
  assert.ok(refreshToken, 'App SSO callback omitted refresh cookie')
  const claims = decodeJwtPayload(sessionToken)
  assert.deepEqual(
    {
      iss: claims.iss,
      sub: claims.sub,
      principal_kind: claims.principal_kind,
      token_use: claims.token_use,
      target_kind: claims.target_kind,
      appid: claims.appid,
      app_instance_id: claims.app_instance_id,
      app_owner_user_id: claims.app_owner_user_id,
    },
    {
      iss: 'verify-hub',
      sub: ctx.accountInfo.user_id,
      principal_kind: 'user',
      token_use: 'session',
      target_kind: 'app',
      appid: appId,
      app_instance_id: appInstanceId,
      app_owner_user_id: ownerUserId,
    },
  )
  assert.notEqual(claims.sudo, true, 'ordinary App SSO session must not be sudo')
  return { appOrigin: appUrl.origin, sessionToken, refreshToken }
}

async function verifyCrossOwnerRefreshRejected(source, destinationOrigin) {
  const response = await fetch(new URL('/sso_refresh', destinationOrigin), {
    method: 'POST',
    headers: {
      Cookie: `buckyos_session_token=${source.sessionToken}; buckyos_refresh_token=${source.refreshToken}`,
    },
    redirect: 'manual',
  })
  assert.equal(response.status, 401)
  const clearCookies = responseSetCookies(response)
  assert.ok(
    clearCookies.some((value) => value.startsWith('buckyos_session_token=;')),
    'cross-owner refresh must clear the session cookie',
  )
  assert.ok(
    clearCookies.some((value) => value.startsWith('buckyos_refresh_token=;')),
    'cross-owner refresh must clear the refresh cookie',
  )
}

async function logoutStaticAppSso(session) {
  await fetch(new URL('/sso_logout', session.appOrigin), {
    method: 'POST',
    headers: {
      Cookie: `buckyos_session_token=${session.sessionToken}; buckyos_refresh_token=${session.refreshToken}`,
    },
  })
}

async function waitForGatewayAppRoute(appHostName, appInstanceId) {
  const ready = await waitForCondition(async () => {
    try {
      const gatewayInfo = JSON.parse(
        await readFile('/opt/buckyos/etc/node_gateway_info.json', 'utf8'),
      )
      return gatewayInfo.app_info?.[appHostName]?.app_instance_id === appInstanceId
    } catch {
      return false
    }
  }, { timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS })
  assert.equal(ready, true, `Gateway route ${appHostName} did not bind ${appInstanceId}`)
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
// Verify Hub 签发的 system-config sudo session；boot LoginAssertion 不能在系统
// 启动完成后作为通用配置写凭证。
let seedRpcClient = null
async function getSeedRpcClient() {
  if (seedRpcClient) {
    return seedRpcClient
  }
  if (!SSO_PASSWORD) {
    throw new Error('BUCKYOS_TEST_ADMIN_PASSWORD is required for resolver fixture writes')
  }
  const ctx = await getSdkContext()
  const nonce = Date.now() + 1
  const verifyHubRpc = new buckyos.kRPCClient(VERIFY_HUB_URL)
  const sudo = await verifyHubRpc.call('sudo_by_password', {
    username: ctx.accountInfo.user_id,
    password: hashPassword(ctx.accountInfo.user_id, SSO_PASSWORD, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
    login_nonce: nonce,
  })
  if (!sudo?.session_token) {
    throw new Error('verify-hub sudo_by_password did not return a session token')
  }
  seedRpcClient = new buckyos.kRPCClient(SYSTEM_CONFIG_URL, sudo.session_token)
  return seedRpcClient
}

async function readConfigJsonAsRoot(key) {
  const rpc = await getSeedRpcClient()
  const response = await rpc.call('sys_config_get', { key })
  const normalized = normalizeConfigValue(response)
  if (normalized == null) {
    throw new Error(`system_config key \`${key}\` returned null`)
  }
  return typeof normalized === 'string' ? JSON.parse(normalized) : normalized
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
async function installPikgToCompletion({
  stagingHandle,
  expectOfflineReady = true,
  installParams = null,
  ownerUserId = null,
  action = null,
  onInspection = null,
}) {
  const source = { kind: 'local_pikg', staging_handle: stagingHandle }
  const options = {
    policy: 'NORMAL',
    ...(installParams ? { install_params: installParams } : {}),
  }
  const inspection = await callControlPanel('apps.inspect', {
    source,
    options,
    ...(ownerUserId ? { owner_user_id: ownerUserId } : {}),
    ...(action ? { action } : {}),
  })
  const plan = inspection.plan
  assert.ok(plan, 'inspect must return an install plan')
  if (onInspection) {
    await onInspection({ inspection, plan })
  }
  if (expectOfflineReady) {
    assert.equal(
      inspection.status?.readiness?.install,
      'OFFLINE_READY',
      `pikg install should be offline ready, got ${JSON.stringify(inspection.status?.readiness)}`,
    )
  }

  const submitParams = {
    source,
    owner_user_id: plan.owner_user_id,
    target: plan.target,
    install_params: plan.install_params,
    options,
    plan,
    approved_plan_fingerprint: plan.plan_fingerprint,
    idempotency_key: createRunId('install'),
  }
  const startResult = await callControlPanel('apps.submit', submitParams)
  assert.ok(startResult.task_id, 'apps.submit should return task_id')
  const taskId = startResult.task_id

  const task = await waitForTask(taskId)
  assert.ok(
    task.data?.result?.completed_at,
    'completed install task must carry a structured result',
  )
  const replay = await callControlPanel('apps.submit', submitParams)
  assert.equal(replay.action, 'replay')
  assert.equal(replay.task_id, taskId)
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
    binPath: () => path.join('/opt/buckyos/bin', `all.web.${appIdFromName(appId)}`),
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
    containerName: (appHostName) => runtimeContainerName(appHostName),
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

async function cleanupStaticWebTestApps() {
  let gatewayInfo
  try {
    gatewayInfo = JSON.parse(
      await readFile('/opt/buckyos/etc/node_gateway_info.json', 'utf8'),
    )
  } catch {
    return
  }
  const instances = new Map()
  for (const entry of Object.values(gatewayInfo.app_info ?? {})) {
    if (!entry || typeof entry !== 'object') continue
    if (!String(entry.app_id ?? '').startsWith('cp-web-')) continue
    if (!entry.app_instance_id || !entry.app_owner_user_id) continue
    instances.set(entry.app_instance_id, entry.app_owner_user_id)
  }
  for (const [appInstanceId, userId] of instances) {
    try {
      await uninstallApp({ appInstanceId, userId, removeData: false })
    } catch {
      // Best-effort cleanup is retried by the next DV run.
    }
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
    await cleanupStaticWebTestApps()
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

  await t.test(
    'static web app PIKG build + install',
    { skip: getEnv('BUCKYOS_TEST_SKIP_STATIC') === '1' },
    async () => {
      const fixture = await stageStaticWebFixture()

    try {
      const published = await buildAndStagePikg(fixture.projectDir)
      assert.equal(published.app_did, deriveAppDid(fixture.appId))

      // v0.5: 显式种 resolver 证据 -> 本地 pikg 安装 -> 确认 -> 严格等完成。
      await seedResolverCache(published.app_did, published.app_doc)
      const { task: installTask, plan } = await installPikgToCompletion({
        stagingHandle: published.pikg_handle,
        onInspection: async ({ plan: inspectedPlan }) => {
          assert.equal(await readConfigJsonOrNull(fixture.specPath(userId)), null)
          const registry = await readConfigJson('system/app_registry')
          assert.equal(registry.apps[appIdFromName(fixture.appId)], undefined)
          assert.equal(registry.instances[inspectedPlan.app_instance_id], undefined)
        },
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

      const otherUserId = createRunId('cp-user')
      const otherPublished = await buildAndStagePikg(fixture.projectDir)
      await seedResolverCache(otherPublished.app_did, otherPublished.app_doc)
      const { plan: otherPlan } = await installPikgToCompletion({
        stagingHandle: otherPublished.pikg_handle,
        ownerUserId: otherUserId,
      })
      const otherSpec = await readConfigJsonAsRoot(fixture.specPath(otherUserId))
      assert.equal(otherPlan.plan_use, 'FRESH_INSTALL')
      assert.equal(otherSpec.app_instance_id, fixture.specId(otherUserId))
      assert.equal(otherSpec.app_doc.version, fixture.version)
      assert.notEqual(otherSpec.app_instance_id, spec.app_instance_id)
      assert.equal(otherSpec.app_did, spec.app_did)
      assert.equal(otherSpec.app_name, spec.app_name)
      assert.notEqual(otherSpec.app_host_name, spec.app_host_name)
      assert.notEqual(otherSpec.app_index, spec.app_index)

      if (SSO_ZONE_BASE_URL && SSO_PASSWORD) {
        await waitForGatewayAppRoute(spec.app_host_name, installedAppInstanceId)
        await waitForGatewayAppRoute(otherSpec.app_host_name, otherSpec.app_instance_id)
        const firstSso = await loginStaticAppThroughSso({
          appHostName: spec.app_host_name,
          appId: appIdFromName(fixture.appId),
          appInstanceId: installedAppInstanceId,
          ownerUserId: userId,
        })
        try {
          await verifyCrossOwnerRefreshRejected(
            firstSso,
            appOriginForHost(otherSpec.app_host_name),
          )
        } finally {
          await logoutStaticAppSso(firstSso)
        }
      }

      const registryBeforeUpgrade = await readConfigJson('system/app_registry')
      const appAllocationBeforeUpgrade = structuredClone(
        registryBeforeUpgrade.apps[appIdFromName(fixture.appId)],
      )
      const instanceAllocationBeforeUpgrade = structuredClone(
        registryBeforeUpgrade.instances[installedAppInstanceId],
      )
      const otherInstanceAllocationBeforeUpgrade = structuredClone(
        registryBeforeUpgrade.instances[otherSpec.app_instance_id],
      )
      assert.ok(appAllocationBeforeUpgrade)
      assert.ok(instanceAllocationBeforeUpgrade)
      assert.ok(otherInstanceAllocationBeforeUpgrade)

      const upgradeVersion = buildVersion()
      await configurePikgSample(fixture.projectDir, {
        appId: fixture.appId,
        version: upgradeVersion,
        ownerDid: OWNER_DID,
      })
      const upgradedPublished = await buildAndStagePikg(fixture.projectDir)
      await seedResolverCache(upgradedPublished.app_did, upgradedPublished.app_doc)
      const { task: upgradeTask, plan: upgradePlan } = await installPikgToCompletion({
        stagingHandle: upgradedPublished.pikg_handle,
        ownerUserId: userId,
        action: 'upgrade',
      })
      assert.equal(upgradePlan.plan_use, 'UPGRADE')
      assert.equal(upgradeTask.id, upgradePlan.task_id)
      assert.equal(upgradePlan.app_instance_id, installedAppInstanceId)

      const upgradedSpec = await readConfigJson(fixture.specPath(userId))
      assert.equal(upgradedSpec.app_instance_id, installedAppInstanceId)
      assert.equal(upgradedSpec.app_doc.version, upgradeVersion)
      assert.equal(upgradedSpec.app_name, spec.app_name)
      assert.equal(upgradedSpec.app_host_name, spec.app_host_name)
      assert.equal(upgradedSpec.app_index, spec.app_index)

      const registryAfterUpgrade = await readConfigJson('system/app_registry')
      assert.deepEqual(
        registryAfterUpgrade.apps[appIdFromName(fixture.appId)],
        appAllocationBeforeUpgrade,
      )
      assert.deepEqual(
        registryAfterUpgrade.instances[installedAppInstanceId],
        instanceAllocationBeforeUpgrade,
      )
      const otherSpecAfterUpgrade = await readConfigJsonAsRoot(
        fixture.specPath(otherUserId),
      )
      assert.equal(otherSpecAfterUpgrade.app_doc.version, fixture.version)
      assert.deepEqual(otherSpecAfterUpgrade.deployment, otherSpec.deployment)

      if (UNINSTALL_AFTER_INSTALL) {
        await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })
        await uninstallApp({
          appInstanceId: otherSpec.app_instance_id,
          userId: otherUserId,
          removeData: false,
        })

        const deletedSpec = await readConfigJson(fixture.specPath(userId))
        assert.equal(deletedSpec.state, 'deleted')
        const deletedOtherSpec = await readConfigJsonAsRoot(fixture.specPath(otherUserId))
        assert.equal(deletedOtherSpec.state, 'deleted')
        const registryAfterUninstall = await readConfigJson('system/app_registry')
        assert.deepEqual(
          registryAfterUninstall.apps[appIdFromName(fixture.appId)],
          appAllocationBeforeUpgrade,
        )
        assert.deepEqual(
          registryAfterUninstall.instances[installedAppInstanceId],
          instanceAllocationBeforeUpgrade,
        )
        assert.deepEqual(
          registryAfterUninstall.instances[otherSpec.app_instance_id],
          otherInstanceAllocationBeforeUpgrade,
        )
      }
      } finally {
        await cleanupTempDir(fixture.tempRoot)
      }
    },
  )

  await t.test(
    'agent runtime app PIKG build + install without AgentDID binding',
    { skip: getEnv('BUCKYOS_TEST_SKIP_AGENT') === '1' },
    async () => {
      const fixture = await stageAgentFixture()

      try {
        const published = await buildAndStagePikg(fixture.projectDir)

        await seedResolverCache(published.app_did, published.app_doc)
        const { plan } = await installPikgToCompletion({
          stagingHandle: published.pikg_handle,
          installParams: {
            auto_start: false,
            expected_instance_count: 1,
            service_settings: { services: {} },
          },
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
        assert.equal(spec.enable, false)

        const instances = await listServiceInstances(fixture.specId(userId))
        assert.equal(
          instances.length,
          0,
          'an Agent runtime App must not create an Agent principal without AgentSpec binding',
        )
        if (UNINSTALL_AFTER_INSTALL) {
          await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })

          const deletedSpec = await readConfigJson(fixture.specPath(userId))
          assert.equal(deletedSpec.state, 'deleted')
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
        assert.equal(
          await waitForCondition(
            () => isContainerRunning(fixture.containerName(spec.app_host_name)),
            { timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS },
          ),
          true,
        )

        if (UNINSTALL_AFTER_INSTALL) {
          await uninstallApp({ appInstanceId: installedAppInstanceId, userId, removeData: false })

          const deletedSpec = await readConfigJson(fixture.specPath(userId))
          assert.equal(deletedSpec.state, 'deleted')
          assert.equal(
            await waitForCondition(
              () =>
                isContainerRunning(fixture.containerName(spec.app_host_name)).then(
                  (running) => !running,
                ),
              { timeoutMs: INSTALL_EVIDENCE_TIMEOUT_MS },
            ),
            true,
          )
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
