/**
 * Browser-side test group definitions for the sys_test panel.
 *
 * Each case mirrors a corresponding backend case in
 * `/sdk/appservice/selftest` (see ../../main.ts), so the user can compare
 * "in page" vs "in background service" results side by side.
 */
import { buckyos, parseSessionTokenClaims } from 'buckyos'

type Sdk = typeof buckyos

// TaskStatus enum is not re-exported from the SDK barrel; mirror the
// Completed value here. Keep in sync with src/task_mgr_client.ts in
// buckyos-websdk and the systest backend's TASK_STATUS_COMPLETED.
const TASK_STATUS_COMPLETED = 'Completed'

export interface TestContext {
  sdk: Sdk
  userId: string
  appId: string
}

export interface TestCase {
  name: string
  run: (ctx: TestContext) => Promise<Record<string, unknown> | void>
}

export interface TestGroup {
  id: string
  title: string
  description: string
  cases: TestCase[]
}

type KEventStreamAckFrame = {
  type: 'ack'
  connection_id: string
  keepalive_ms: number
}

type KEventStreamEventFrame = {
  type: 'event'
  event: {
    eventid: string
    source_node: string
    ingress_node?: string | null
    data?: Record<string, unknown>
  }
}

type KEventStreamKeepaliveFrame = {
  type: 'keepalive'
  at_ms: number
}

type KEventStreamErrorFrame = {
  type: 'error'
  error: string
}

type KEventStreamFrame =
  | KEventStreamAckFrame
  | KEventStreamEventFrame
  | KEventStreamKeepaliveFrame
  | KEventStreamErrorFrame

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    promise.then(
      value => {
        window.clearTimeout(timer)
        resolve(value)
      },
      error => {
        window.clearTimeout(timer)
        reject(error)
      },
    )
  })
}

function getKEventBaseUrl(sdk: Sdk): string {
  const baseUrl = sdk.getZoneServiceURL('kevent')
  return baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`
}

function getKEventRequestUrl(sdk: Sdk, path: 'publish' | 'stream'): string {
  return new URL(path, getKEventBaseUrl(sdk)).toString()
}

async function readJsonResponse(response: Response): Promise<Record<string, unknown>> {
  const text = await response.text()
  try {
    return JSON.parse(text) as Record<string, unknown>
  } catch {
    throw new Error(`non-json response (${response.status}): ${text.slice(0, 200)}`)
  }
}

async function publishKEvent(sdk: Sdk, eventid: string, data: Record<string, unknown>): Promise<void> {
  const response = await fetch(getKEventRequestUrl(sdk, 'publish'), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    credentials: 'include',
    body: JSON.stringify({ eventid, data }),
  })
  const payload = await readJsonResponse(response)
  if (!response.ok || payload.status !== 'ok') {
    throw new Error(String(payload.error ?? `kevent publish failed with status ${response.status}`))
  }
}

async function openKEventStream(
  sdk: Sdk,
  patterns: string[],
): Promise<{
  next: (timeoutMs: number) => Promise<KEventStreamFrame>
  close: () => Promise<void>
}> {
  const controller = new AbortController()
  const response = await fetch(getKEventRequestUrl(sdk, 'stream'), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    credentials: 'include',
    signal: controller.signal,
    body: JSON.stringify({ patterns, keepalive_ms: 1_000 }),
  })
  if (!response.ok) {
    const payload = await readJsonResponse(response)
    throw new Error(String(payload.error ?? `kevent stream failed with status ${response.status}`))
  }
  if (!response.body) {
    throw new Error('kevent stream response has no body')
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  const readLine = async (timeoutMs: number): Promise<string> => {
    while (true) {
      const newlineIndex = buffer.indexOf('\n')
      if (newlineIndex >= 0) {
        const line = buffer.slice(0, newlineIndex).trim()
        buffer = buffer.slice(newlineIndex + 1)
        if (line.length > 0) {
          return line
        }
        continue
      }

      const chunk = await withTimeout(
        reader.read(),
        timeoutMs,
        'waiting for kevent stream frame',
      )
      if (chunk.done) {
        throw new Error('kevent stream closed unexpectedly')
      }
      buffer += decoder.decode(chunk.value, { stream: true })
    }
  }

  return {
    next: async (timeoutMs: number) => {
      const line = await readLine(timeoutMs)
      return JSON.parse(line) as KEventStreamFrame
    },
    close: async () => {
      try {
        await reader.cancel()
      } catch {
        // ignore close failures
      }
      controller.abort()
    },
  }
}

export const TEST_GROUPS: TestGroup[] = [
  {
    id: 'system_config',
    title: 'SystemConfigClient',
    description:
      '系统配置读写检测：读取 boot/config，并在 users/${userId}/apps/${appId}/info 下完成一次写入与回读。',
    cases: [
      {
        name: 'SystemConfigClient.get(boot/config)',
        run: async ({ sdk }) => {
          const bootConfig = await sdk.getSystemConfigClient().get('boot/config')
          const parsed = JSON.parse(bootConfig.value) as Record<string, unknown>
          if (!parsed || typeof parsed !== 'object') {
            throw new Error('boot/config did not decode into an object')
          }
          if (Object.keys(parsed).length === 0) {
            throw new Error('boot/config decoded into an empty object')
          }
          return { version: bootConfig.version, keys: Object.keys(parsed).length }
        },
      },
      {
        name: 'SystemConfigClient writes and reads back a namespaced key',
        run: async ({ sdk, userId, appId }) => {
          const key = `users/${userId}/apps/${appId}/info`
          const value = JSON.stringify({ ok: true, key, ts: Date.now() })
          await sdk.getSystemConfigClient().set(key, value)
          const read = await sdk.getSystemConfigClient().get(key)
          if (read.value !== value) {
            throw new Error(`value mismatch at ${key}`)
          }
          return { key }
        },
      },
    ],
  },
  {
    id: 'app_settings',
    title: 'AppSettings',
    description:
      '应用设置读写检测：getAppSetting / setAppSetting 在测试键上完成一次往返。',
    cases: [
      {
        name: 'getAppSetting/setAppSetting round trip on namespaced key',
        run: async ({ sdk }) => {
          const settingPath = `test_settings.websdk_${Date.now()}`
          await sdk.setAppSetting(settingPath, '"roundtrip"')
          const read = await sdk.getAppSetting(settingPath)
          if (read !== 'roundtrip') {
            throw new Error(`settings round trip mismatch, got ${JSON.stringify(read)}`)
          }
          return { settingPath }
        },
      },
    ],
  },
  {
    id: 'task_manager',
    title: 'TaskManagerClient',
    description: '任务管理器生命周期检测：创建 → 更新进度/状态 → 查询 → 删除。',
    cases: [
      {
        name: 'TaskManagerClient creates/updates/queries/deletes a namespaced task',
        run: async ({ sdk, userId, appId }) => {
          const client = sdk.getTaskManagerClient()
          const name = `test-websdk-${Date.now()}`
          const created = await client.createTask({
            name,
            taskType: 'test',
            data: { createdBy: 'sys-test-panel' },
            userId,
            appId,
          })
          try {
            await client.updateTaskProgress(created.id, 1, 2)
            // The TaskStatus enum is not in the SDK barrel; pass the string
            // value directly. The runtime check below verifies the round trip.
            await client.updateTaskStatus(created.id, TASK_STATUS_COMPLETED as any)
            const fetched = await client.getTask(created.id)
            if (fetched.status !== TASK_STATUS_COMPLETED) {
              throw new Error(
                `expected task ${created.id} to be Completed, got ${fetched.status}`,
              )
            }
            const filtered = await client.listTasks({
              filter: { root_id: String(created.id) },
            })
            if (!filtered.some((task) => task.id === created.id)) {
              throw new Error(`task ${created.id} missing from filtered list`)
            }
            return { taskId: created.id }
          } finally {
            try {
              await client.deleteTask(created.id)
            } catch {
              // best-effort cleanup, ignore
            }
          }
        },
      },
    ],
  },
  {
    id: 'verify_hub',
    title: 'VerifyHub / Session',
    description: '会话身份检测：读取当前 accountInfo，并解析 session token 中的 claims。',
    cases: [
      {
        name: 'getAccountInfo + parseSessionTokenClaims',
        run: async ({ sdk }) => {
          const accountInfo = await sdk.getAccountInfo()
          if (!accountInfo) {
            throw new Error('not logged in: getAccountInfo() returned null')
          }
          const claims = parseSessionTokenClaims(accountInfo.session_token ?? null)
          if (!claims) {
            throw new Error('failed to parse session token claims')
          }
          return {
            userId: accountInfo.user_id,
            userType: accountInfo.user_type,
            appId: claims.appid ?? null,
            exp: claims.exp ?? null,
          }
        },
      },
    ],
  },
  {
    id: 'kevent',
    title: 'KEvent',
    description: '事件检测：通过 kevent HTTP stream 订阅唯一事件，再通过 publish 发布并确认页面端收到回环事件。',
    cases: [
      {
        name: 'KEvent stream/publish round trip on a unique eventid',
        run: async ({ sdk, userId, appId }) => {
          const eventid = `/users/${userId}/apps/${appId}/kevent/sys_test_${Date.now()}_${Math.random()
            .toString(36)
            .slice(2, 8)}`
          const marker = `page_${Date.now()}`
          const stream = await openKEventStream(sdk, [eventid])
          try {
            const ack = await stream.next(2_000)
            if (ack.type !== 'ack') {
              throw new Error(`expected kevent ack frame, got ${ack.type}`)
            }

            await publishKEvent(sdk, eventid, {
              marker,
              origin: 'sys_test_web',
              userId,
              appId,
            })

            while (true) {
              const frame = await stream.next(5_000)
              if (frame.type === 'keepalive') {
                continue
              }
              if (frame.type === 'error') {
                throw new Error(frame.error)
              }
              if (frame.type !== 'event') {
                throw new Error(`unexpected kevent frame type: ${frame.type}`)
              }

              const eventData = frame.event.data ?? {}
              if (frame.event.eventid !== eventid) {
                throw new Error(`received mismatched eventid: ${frame.event.eventid}`)
              }
              if (eventData.marker !== marker) {
                throw new Error(`received mismatched marker: ${JSON.stringify(eventData)}`)
              }

              return {
                eventid,
                connectionId: ack.connection_id,
                sourceNode: frame.event.source_node,
                ingressNode: frame.event.ingress_node ?? null,
              }
            }
          } finally {
            await stream.close()
          }
        },
      },
    ],
  },
]
