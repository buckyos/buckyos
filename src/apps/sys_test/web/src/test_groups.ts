/**
 * Browser-side test group definitions for the sys_test panel.
 *
 * Each case mirrors a corresponding backend case in
 * `/sdk/appservice/selftest` (see ../../main.ts), so the user can compare
 * "in page" vs "in background service" results side by side.
 */
import { bns, bucket, buckyos, mailboxAddress, mailboxResource, ndm, ndm_proxy, parseSessionTokenClaims, sn } from 'buckyos'

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
  titleKey?: string
  cases: TestCase[]
  backendOnly?: boolean
}

function getKEventBaseUrl(sdk: Sdk): string {
  const baseUrl = sdk.getZoneServiceURL('kevent')
  return baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`
}

function getKEventRequestUrl(sdk: Sdk, path: 'publish' | 'stream'): string {
  return new URL(path, new URL(getKEventBaseUrl(sdk), window.location.origin)).toString()
}

async function readJsonResponse(response: Response): Promise<Record<string, unknown>> {
  const text = await response.text()
  try {
    return JSON.parse(text) as Record<string, unknown>
  } catch {
    throw new Error(`non-json response (${response.status}): ${text.slice(0, 200)}`)
  }
}

/** The app's own `smoke` bucket (`dapp_meta/app.json`: data / temp, 1 day). */
const SMOKE_BUCKET = 'smoke'

function smokeKey(kind: string): string {
  return `${kind}/${Date.now()}-${Math.random().toString(36).slice(2, 8)}.txt`
}

/** Reads a key through its data-plane URL with the page's cookies only, as an `<img src>` would. */
async function readObjectUrl(client: bucket.BucketClient, bucketId: string, key: string): Promise<{ url: string; text: string }> {
  const url = client.objectUrl(bucketId, key)
  const response = await fetch(url, { credentials: 'include' })
  if (!response.ok) throw new Error(`GET ${url} answered ${response.status}: ${(await response.text()).slice(0, 200)}`)
  return { url, text: await response.text() }
}

/** `bucket.delete`, then the key must be gone (`no_such_key` by code, never by message). */
async function deleteAndConfirm(client: bucket.BucketClient, bucketId: string, key: string): Promise<boolean> {
  const deleted = await client.delete(bucketId, key)
  try {
    await client.fetchObject(bucketId, key)
  } catch (error) {
    if (error instanceof bucket.BucketError && error.code === 'no_such_key') return deleted.deleted
    throw error
  }
  throw new Error(`${key} is still readable after bucket.delete`)
}

async function publishKEvent(sdk: Sdk, eventid: string, data: Record<string, unknown>): Promise<void> {
  const account = await sdk.getAccountInfo()
  if (!account?.session_token) {
    throw new Error('KEvent publish requires a browser session')
  }
  const response = await fetch(getKEventRequestUrl(sdk, 'publish'), {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${account.session_token}`,
    },
    credentials: 'include',
    body: JSON.stringify({ eventid, data }),
    signal: AbortSignal.timeout(5_000),
  })
  const payload = await readJsonResponse(response)
  if (!response.ok || payload.status !== 'ok') {
    throw new Error(String(payload.error ?? `kevent publish failed with status ${response.status}`))
  }
}

export const TEST_GROUPS: TestGroup[] = [
  {
    id: 'xllm',
    title: 'xllm subprocess identity (#640)',
    titleKey: 'tests.xllm.title',
    backendOnly: true,
    cases: [],
  },
  {
    id: 'runtime',
    title: 'SDK runtime',
    titleKey: 'tests.runtime.title',
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
    cases: [
      {
        name: 'SystemConfigClient round trip on user app settings',
        run: async ({ sdk, userId, appId }) => {
          const client = sdk.getSystemConfigClient()
          const key = `users/${userId}/apps/${appId}/settings`
          let previous: string | null = null
          try {
            previous = (await client.get(key)).value
          } catch (error) {
            if (!(error instanceof Error) || error.message !== `system_config key not found: ${key}`) throw error
          }
          const settingPath = `websdk_${crypto.randomUUID().replaceAll('-', '')}`
          const settings = previous === null ? {} : JSON.parse(previous)
          try {
            await client.set(key, JSON.stringify({ ...settings, [settingPath]: 'roundtrip' }))
            const read = JSON.parse((await client.get(key)).value)
            if (read[settingPath] !== 'roundtrip') {
              throw new Error(`settings round trip mismatch, got ${JSON.stringify(read[settingPath])}`)
            }
            return { key, settingPath }
          } finally {
            if (previous === null) await client.delete(key)
            else await client.set(key, previous)
          }
        },
      },
    ],
  },
  {
    id: 'task_manager',
    title: 'TaskManagerClient',
    cases: [
      {
        name: 'TaskManagerClient creates/runs/queries/archives a namespaced task',
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
    title: 'VerifyHub session',
    titleKey: 'tests.verify_hub.title',
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
    cases: [
      {
        name: 'KEvent stream/publish round trip on a unique eventid',
        run: async ({ sdk, userId, appId }) => {
          const eventid = `/users/${userId}/apps/${appId}/kevent/sys_test_${Date.now()}_${Math.random()
            .toString(36)
            .slice(2, 8)}`
          const marker = `page_${Date.now()}`
          const reader = await sdk.createEventReader(eventid, {
            keepaliveMs: 1_000,
            signal: AbortSignal.timeout(10_000),
          })
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
    title: 'Service clients',
    titleKey: 'tests.service_clients.title',
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
        name: 'AiccClient.policyStatus',
        run: async ({ sdk }) => {
          const status = await sdk.getAiccClient().policyStatus({})
          if (typeof status.user_id !== 'string' || !Array.isArray(status.limits)) {
            throw new Error('policy.status returned no user or limit list')
          }
          return { user_id: status.user_id, limits: status.limits.length }
        },
      },
      {
        name: 'MsgCenterClient rejects an ungranted owner inbox read',
        run: async ({ sdk, userId }) => {
          const mailbox = mailboxAddress(bns.didBnsFromName(userId))
          const resource = mailboxResource(mailbox, 'INBOX')
          try {
            await sdk.getMsgCenterClient().peekBox({
              mailbox,
              box_kind: 'INBOX',
              limit: 1,
              with_object: false,
            })
          } catch (error) {
            if (
              error instanceof Error
              && error.message.startsWith('RPC call error: No permission:')
              && error.message.includes(`resource:${resource},action:read`)
            ) {
              return { mailbox, resource, access: 'denied' }
            }
            throw error
          }
          throw new Error('sys_test has no MsgCenter grant but could read the owner inbox')
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
    title: 'BNS and SN utilities',
    titleKey: 'tests.sdk_utilities.title',
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
    id: 'bucket',
    title: 'App object bucket smoke test',
    titleKey: 'tests.bucket.title',
    cases: [
      {
        name: 'putBytes → objectUrl → bucket.delete',
        run: async ({ sdk, userId, appId }) => {
          const client = sdk.getBucketClient()
          const bucketId = bucket.userBucketId(appId, userId, SMOKE_BUCKET)
          const key = smokeKey('put')
          const body = `sys_test putBytes ${key}`
          const put = await client.putBytes(bucketId, key, body, { contentType: 'text/plain', if_none_match: '*' })
          try {
            const read = await readObjectUrl(client, bucketId, key)
            if (read.text !== body) throw new Error(`objectUrl returned ${JSON.stringify(read.text.slice(0, 80))}`)
            return { bucketId, key, objId: put.obj_id, version: put.version, url: read.url, deleted: await deleteAndConfirm(client, bucketId, key) }
          } catch (error) {
            await client.delete(bucketId, key).catch(() => undefined)
            throw error
          }
        },
      },
      {
        name: 'ndm.importFiles + ensureUploaded({ bind }) → objectUrl → bucket.delete',
        run: async ({ sdk, userId, appId }) => {
          const client = sdk.getBucketClient()
          const bucketId = bucket.userBucketId(appId, userId, SMOKE_BUCKET)
          const key = smokeKey('upload')
          const body = `sys_test upload direct to bucket ${key}`
          const snapshot = await ndm.importFiles([new File([body], key.split('/').pop()!, { type: 'text/plain' })], { mode: 'single_file' })
          try {
            const status = await ndm.ensureUploaded(snapshot.sessionId, { bind: { bucketId, key, contentType: 'text/plain', ifNoneMatch: '*' } })
            if (!status.binding || status.binding.key !== key) throw new Error(`finalize did not bind ${key}: ${JSON.stringify(status.binding)}`)
            const read = await readObjectUrl(client, bucketId, key)
            if (read.text !== body) throw new Error(`objectUrl returned ${JSON.stringify(read.text.slice(0, 80))}`)
            return { bucketId, key, binding: status.binding, url: read.url, deleted: await deleteAndConfirm(client, bucketId, key) }
          } catch (error) {
            await client.delete(bucketId, key).catch(() => undefined)
            throw error
          } finally {
            ndm.releaseImportSession(snapshot.sessionId)
          }
        },
      },
    ],
  },
  {
    id: 'ndm_proxy',
    title: 'NDM proxy',
    titleKey: 'tests.ndm_proxy.title',
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
