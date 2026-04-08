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
]
