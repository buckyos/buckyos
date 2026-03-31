import { createPrivateKey, randomBytes, sign as signDetached } from 'node:crypto'
import { constants as fsConstants } from 'node:fs'
import { access, readFile } from 'node:fs/promises'

import { buckyos, RuntimeType } from 'buckyos/node'

type JsonPrimitive = string | number | boolean | null
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue }

type AccountInfo = {
  session_token?: string
  refresh_token?: string
  user_id?: string
}

type AiccCompleteResponse = {
  task_id: string
  status: 'succeeded' | 'running' | 'failed'
  result?: JsonValue
  event_ref?: string | null
}

type TaskRecord = {
  id: number
  status: string
  message?: string | null
  updated_at?: number
  data?: {
    aicc?: {
      external_task_id?: string
      output?: JsonValue
      error?: JsonValue
      status?: string
    }
  }
}

const SYSTEM_CONFIG_URL =
  getEnv('BUCKYOS_SYSTEM_CONFIG_URL') ??
  'http://127.0.0.1:3200/kapi/system_config'
const VERIFY_HUB_URL =
  getEnv('BUCKYOS_VERIFY_HUB_URL') ??
  'http://127.0.0.1:3300/kapi/verify-hub'
const TASK_MANAGER_URL =
  getEnv('BUCKYOS_TASK_MANAGER_URL') ??
  'http://127.0.0.1:3380/kapi/task-manager'
const AICC_URL =
  getEnv('AICC_URL') ??
  'http://127.0.0.1:4040/kapi/aicc'
const TEST_APP_ID =
  getEnv('BUCKYOS_TEST_APP_ID') ??
  'control-panel'
const TEST_USER_ID =
  getEnv('BUCKYOS_TEST_USER_ID') ??
  'devtest'
const AICC_MODEL_ALIAS =
  getEnv('AICC_MODEL_ALIAS') ??
  'llm.default'
const AICC_TEST_INPUT =
  getEnv('AICC_TEST_INPUT') ??
  '今天天气如何，我在sanjose'
const AICC_WAIT_TIMEOUT_MS = Number(
  getEnv('AICC_WAIT_TIMEOUT_MS') ?? '90000',
)

function getEnv(name: string): string | null {
  const value = process.env[name]
  if (typeof value !== 'string') {
    return null
  }
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path, fsConstants.F_OK)
    return true
  } catch {
    return false
  }
}

async function probeRpc(
  url: string,
  method: string,
  params: Record<string, unknown>,
): Promise<void> {
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

function encodeJwtPart(value: Record<string, unknown>): string {
  return Buffer.from(JSON.stringify(value)).toString('base64url')
}

async function createOwnerSignedLoginJwt(userId: string): Promise<string | null> {
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

async function loginWithAppClient(): Promise<AccountInfo> {
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
    : ((await buckyos.login()) as AccountInfo)

  if (!accountInfo?.session_token) {
    throw new Error('AppClient login did not return a session_token')
  }

  const verifyHubRpc = new buckyos.kRPCClient(VERIFY_HUB_URL)
  const tokenPair = await verifyHubRpc.call('login_by_jwt', {
    type: 'jwt',
    jwt: accountInfo.session_token,
  }) as AccountInfo

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

function normalizeTaskList(result: unknown): TaskRecord[] {
  if (Array.isArray(result)) {
    return result as TaskRecord[]
  }
  if (result && typeof result === 'object' && Array.isArray((result as { tasks?: unknown }).tasks)) {
    return (result as { tasks: TaskRecord[] }).tasks
  }
  return []
}

function normalizeTask(result: unknown): TaskRecord {
  if (result && typeof result === 'object' && 'task' in result) {
    return (result as { task: TaskRecord }).task
  }
  return result as TaskRecord
}

function renderSummary(summary: unknown): string {
  if (!summary || typeof summary !== 'object') {
    return '<empty>'
  }

  const text = (summary as { text?: unknown }).text
  if (typeof text === 'string' && text.trim()) {
    return text.trim()
  }

  return JSON.stringify(summary, null, 2)
}

function extractTaskSummary(task: TaskRecord): unknown {
  return task.data?.aicc?.output ?? null
}

function extractTaskError(task: TaskRecord): unknown {
  return task.data?.aicc?.error ?? task.message ?? null
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function findTaskByExternalId(
  taskManagerRpc: InstanceType<typeof buckyos.kRPCClient>,
  externalTaskId: string,
  deadlineMs: number,
): Promise<TaskRecord | null> {
  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call('list_tasks', {
      app_id: TEST_APP_ID,
      task_type: 'aicc.compute',
      source_user_id: TEST_USER_ID,
      source_app_id: TEST_APP_ID,
    })

    const tasks = normalizeTaskList(result).sort(
      (left, right) => (right.updated_at ?? 0) - (left.updated_at ?? 0),
    )
    const matched = tasks.find(
      (task) => task.data?.aicc?.external_task_id === externalTaskId,
    )
    if (matched) {
      return matched
    }

    await sleep(1000)
  }

  return null
}

async function waitForFinalTaskResult(
  taskManagerRpc: InstanceType<typeof buckyos.kRPCClient>,
  externalTaskId: string,
): Promise<TaskRecord> {
  const deadlineMs = Date.now() + AICC_WAIT_TIMEOUT_MS
  const task = await findTaskByExternalId(taskManagerRpc, externalTaskId, deadlineMs)

  if (!task) {
    throw new Error(`Timed out while locating AICC task for external_task_id=${externalTaskId}`)
  }

  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call('get_task', { id: task.id })
    const latest = normalizeTask(result)
    if (['Completed', 'Failed', 'Canceled'].includes(latest.status)) {
      return latest
    }
    await sleep(1000)
  }

  throw new Error(`Timed out while waiting for AICC task ${task.id} to finish`)
}

async function main(): Promise<void> {
  const runId = `aicc-smoke-${Date.now().toString(36)}-${randomBytes(3).toString('hex')}`

  await probeRpc(SYSTEM_CONFIG_URL, 'sys_config_get', { key: 'boot/config' })
  await probeRpc(AICC_URL, 'cancel', { task_id: runId })

  const accountInfo = await loginWithAppClient()
  const sessionToken = accountInfo.session_token
  if (!sessionToken) {
    throw new Error('missing session token after login')
  }

  const aiccRpc = new buckyos.kRPCClient(AICC_URL, sessionToken)
  const taskManagerRpc = new buckyos.kRPCClient(TASK_MANAGER_URL, sessionToken)

  const response = await aiccRpc.call('complete', {
    capability: 'llm_router',
    model: {
      alias: AICC_MODEL_ALIAS,
    },
    requirements: {
      must_features: [],
    },
    payload: {
      messages: [
        {
          role: 'user',
          content: AICC_TEST_INPUT,
        },
      ],
      options: {
        temperature: 0.2,
        max_tokens: 2560,
        session_id: runId,
        rootid: runId,
      },
    },
    idempotency_key: runId,
  }) as AiccCompleteResponse

  if (!response?.task_id || !response?.status) {
    throw new Error(`invalid AICC response: ${JSON.stringify(response, null, 2)}`)
  }

  let summary = response.result ?? null
  let terminalStatus = response.status

  if (response.status === 'failed') {
    throw new Error(`AICC complete failed: ${JSON.stringify(response, null, 2)}`)
  }

  if (response.status === 'running' && !summary) {
    const finalTask = await waitForFinalTaskResult(taskManagerRpc, response.task_id)
    terminalStatus = finalTask.status.toLowerCase()

    if (finalTask.status !== 'Completed') {
      throw new Error(
        `AICC task ${finalTask.id} ended with ${finalTask.status}: ${JSON.stringify(extractTaskError(finalTask), null, 2)}`,
      )
    }

    summary = extractTaskSummary(finalTask)
  }

  console.log('=== AICC Smoke Test ===')
  console.log(`App ID: ${TEST_APP_ID}`)
  console.log(`User ID: ${accountInfo.user_id ?? TEST_USER_ID}`)
  console.log(`Model Alias: ${AICC_MODEL_ALIAS}`)
  console.log(`Task ID: ${response.task_id}`)
  console.log(`Status: ${terminalStatus}`)
  console.log('Input:')
  console.log(AICC_TEST_INPUT)
  console.log('Output:')
  console.log(renderSummary(summary))

  buckyos.logout(false)
}

main().catch((error) => {
  console.error('AICC smoke test failed')
  console.error(error)
  process.exitCode = 1
})
