/**
 * Browser-side test group definitions for the sys_test panel.
 *
 * Each case mirrors a corresponding backend case in
 * `/sdk/appservice/selftest` (see ../../main.ts), so the user can compare
 * "in page" vs "in background service" results side by side.
 */
import { bns, buckyos, ndm_proxy, parseSessionTokenClaims, sn } from 'buckyos'

type Sdk = typeof buckyos

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

export const TEST_GROUPS: TestGroup[] = [
  {
    id: 'runtime',
    title: 'SDK Runtime',
    description: '运行时检测：确认 WebSDK 初始化后的 runtime、appId、Zone 与服务地址均可用。',
    cases: [
      {
        name: 'Runtime identity and service URL resolution',
        run: async ({ sdk, appId }) => {
          const actualAppId = sdk.getAppId()
          const zoneHost = sdk.getZoneHostName()
          if (actualAppId !== appId) {
            throw new Error(`expected appId ${appId}, got ${actualAppId ?? 'null'}`)
          }
          if (!zoneHost) {
            throw new Error('getZoneHostName() returned an empty value')
          }
          const services = ['system-config', 'task-manager', 'workflow', 'aicc', 'kmsg', 'msg-center', 'repo-service']
          const serviceUrls = Object.fromEntries(services.map(name => [name, sdk.getZoneServiceURL(name)]))
          return { runtimeType: sdk.getRuntimeType(), appId: actualAppId, zoneHost, serviceUrls }
        },
      },
    ],
  },
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
            schema_id: 'raw/v1',
            input: { createdBy: 'sys-test-panel', userId, appId },
            executor: { kind: 'SelfApp' },
            idempotency_key: `sys-test-${crypto.randomUUID()}`,
          })
          const taskId = created.task_id
          try {
            await client.runnerStart(taskId)
            await client.runnerProgress(taskId, { completed: 1, total: 2 })
            await client.runnerComplete(taskId, { ok: true })
            const fetched = await client.getTask(taskId)
            if (fetched.phase !== 'Terminal' || fetched.outcome !== 'Succeeded') {
              throw new Error(
                `expected task ${taskId} to succeed, got ${fetched.phase}/${fetched.outcome}`,
              )
            }
            const page = await client.listTasks({ root_id: created.root_id })
            if (!page.tasks.some((task) => task.task_id === taskId)) {
              throw new Error(`task ${taskId} missing from filtered list`)
            }
            return { taskId }
          } finally {
            try {
              const latest = await client.getTask(taskId)
              if (latest.phase === 'Terminal' && latest.archived_at === undefined) {
                await client.archiveTask({
                  task_id: taskId,
                  expected_revision: latest.revision,
                })
              }
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
    description: '事件检测：通过 WebSDK createEventReader 订阅唯一事件，再通过 publish 发布并确认页面端收到回环事件。',
    cases: [
      {
        name: 'KEvent stream/publish round trip on a unique eventid',
        run: async ({ sdk, userId, appId }) => {
          const eventid = `/users/${userId}/apps/${appId}/kevent/sys_test_${Date.now()}_${Math.random()
            .toString(36)
            .slice(2, 8)}`
          const marker = `page_${Date.now()}`
          const reader = await sdk.createEventReader(eventid, { keepaliveMs: 1_000 })
          try {
            await publishKEvent(sdk, eventid, {
              marker,
              origin: 'sys_test_web',
              userId,
              appId,
            })

            const event = await reader.pullEvent(5_000)
            if (!event) {
              throw new Error('timed out waiting for the published kevent')
            }
            const eventData = event.data && typeof event.data === 'object'
              ? event.data as Record<string, unknown>
              : {}
            if (event.eventid !== eventid) {
              throw new Error(`received mismatched eventid: ${event.eventid}`)
            }
            if (eventData.marker !== marker) {
              throw new Error(`received mismatched marker: ${JSON.stringify(eventData)}`)
            }

            return {
              eventid,
              sourceNode: event.source_node,
              sourcePid: event.source_pid,
              ingressNode: event.ingress_node ?? null,
              timestamp: event.timestamp,
            }
          } finally {
            await reader.close()
          }
        },
      },
    ],
  },
  {
    id: 'service_clients',
    title: 'Service Clients',
    description: '新版服务客户端检测：Workflow、AICC、MsgCenter、Repo 执行只读调用，MsgQueue 执行带清理的生命周期调用。',
    cases: [
      {
        name: 'WorkflowClient.listDefinitions',
        run: async ({ sdk, userId, appId }) => {
          const definitions = await sdk.getWorkflowClient().listDefinitions({
            owner: { user_id: userId, app_id: appId },
          })
          return { definitions: definitions.length }
        },
      },
      {
        name: 'AiccClient.queryQuota',
        run: async ({ sdk }) => {
          const { quota } = await sdk.getAiccClient().queryQuota({})
          if (typeof quota.state !== 'string' || quota.state.length === 0) {
            throw new Error('quota.query returned an invalid state')
          }
          return { quota }
        },
      },
      {
        name: 'MsgCenterClient.peekBox',
        run: async ({ sdk, userId }) => {
          const owner = bns.didBnsFromName(userId)
          const records = await sdk.getMsgCenterClient().peekBox({
            owner,
            box_kind: 'INBOX',
            limit: 1,
            with_object: false,
          })
          return { owner, records: records.length }
        },
      },
      {
        name: 'RepoClient.stat',
        run: async ({ sdk }) => {
          const stat = await sdk.getRepoClient().stat()
          return { stat }
        },
      },
      {
        name: 'MsgQueueClient create/post/stat/delete lifecycle',
        run: async ({ sdk, userId, appId }) => {
          const client = sdk.getMsgQueueClient()
          const owner = bns.didBnsFromName(userId)
          const queueName = `sys-test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
          const queueUrn = await client.createQueue(queueName, appId, owner)
          try {
            const msgIndex = await client.postMessage(queueUrn, {
              index: 0,
              created_at: Date.now(),
              payload: Array.from(new TextEncoder().encode('sys_test')),
              headers: { source: 'sys_test_web' },
            })
            const stats = await client.getQueueStats(queueUrn)
            if (stats.message_count < 1) {
              throw new Error(`expected at least one queued message, got ${stats.message_count}`)
            }
            return { queueUrn, msgIndex, stats }
          } finally {
            await client.deleteQueue(queueUrn)
          }
        },
      },
    ],
  },
  {
    id: 'sdk_utilities',
    title: 'BNS / SN Utilities',
    description: '无副作用的工具 API 检测：BNS DID 往返与 SN URL/region 标准化。',
    cases: [
      {
        name: 'BNS name/DID canonical round trip',
        run: async ({ userId }) => {
          const canonicalName = bns.canonicalBnsName(userId)
          const did = bns.didBnsFromName(canonicalName)
          const roundTripName = bns.nameFromDidBns(did)
          if (roundTripName !== canonicalName) {
            throw new Error(`BNS round trip mismatch: ${roundTripName}`)
          }
          return { canonicalName, did }
        },
      },
      {
        name: 'SN URL and region normalization',
        run: async () => {
          const authUrl = sn.normalizeSnUrl('https://sn.example', 'auth')
          const region = sn.normalizeSnRegionIdHint('  US__West / 2  ')
          if (authUrl !== 'https://sn.example/kapi/sn/auth' || region !== 'us-west-2') {
            throw new Error(`unexpected SN normalization: ${authUrl}, ${region}`)
          }
          return { authUrl, region }
        },
      },
    ],
  },
  {
    id: 'ndm_proxy',
    title: 'NDM Proxy',
    description: '新版 NDM proxy 检测：Browser 验证受信运行时保护，AppService 通过 kRPC 读取 outbox 计数。',
    cases: [
      {
        name: 'ndm_proxy.outboxCount',
        run: async ({ sdk }) => {
          try {
            await ndm_proxy.outboxCount()
          } catch (error) {
            if (
              sdk.getRuntimeType() === 'Browser'
              && error instanceof ndm_proxy.NdmProxyError
              && error.code === 'PROXY_API_NOT_SUPPORTED_IN_RUNTIME'
            ) {
              return { supported: false, errorCode: error.code }
            }
            throw error
          }
          throw new Error('ndm_proxy unexpectedly allowed access in Browser runtime')
        },
      },
    ],
  },
]
