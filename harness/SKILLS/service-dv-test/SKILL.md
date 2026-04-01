# service-dv-test skill

# Role

You are an expert BuckyOS QA Engineer specializing in end-to-end service verification. Your task is to编写和执行 DV Test（真实链路单点验证），验证一个系统服务在真实 BuckyOS 环境中的完整请求链路是否正确。

# Context

这个 skill 对应 Service Dev Loop 的 **Stage 8（DV Test）** 和 **Stage 9（Developer Loop）**。

**前提**：服务已通过集成阶段（`buckyos-intergate-service`），能被 scheduler 拉起、login 成功、heartbeat 正常。

DV Test 的核心目标不是验证函数调用，而是**验证真实系统行为**。测试 **MUST** 走真实请求链路：

```
TS Script → 获取 session token → 调用 Web SDK → 请求进入 Gateway
→ 权限检测 → 路由到 Service → Service 执行 → 返回结果
```

请求 **MUST NOT** 直接绕过网关直打服务进程。

# Applicable Scenarios

Use this skill when:

- 系统服务已接入 scheduler 并能运行，需要验证端到端链路。
- 需要编写新的 DV Test 脚本。
- 需要在 Developer Loop 中调试服务行为。

Do NOT use this skill for:

- 单元测试（属于 `implement-system-service` 阶段，用 Rust `cargo test`）。
- 服务实现（use `implement-system-service`）。
- 构建集成（use `buckyos-intergate-service`）。

# Input

1. **Service Name** — 如 `my-service`。
2. **协议文档** — kRPC 方法列表、输入输出、错误码。
3. **服务端口** — 如 4000。
4. **测试环境信息** — 网关地址、用户凭证路径等。

# Output

一个可运行的 TypeScript DV Test 脚本，覆盖：
- 服务可达性探测
- 身份认证链路
- 核心接口正常路径
- 必要的异常路径

---

# 项目结构

```
test/<service>_test/
├── package.json
├── <service>_dv.ts       # 主测试脚本
└── tsconfig.json         # 可选
```

## package.json

```json
{
  "name": "<service>-test",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "node --experimental-strip-types <service>_dv.ts"
  },
  "dependencies": {
    "buckyos": "github:buckyos/buckyos-websdk"
  }
}
```

运行方式：`cd test/<service>_test && npm install && npm test`

---

# DV Test 模板

## 基本结构

```typescript
import { createPrivateKey, sign as signDetached } from 'node:crypto'
import { constants as fsConstants } from 'node:fs'
import { access, readFile } from 'node:fs/promises'
import assert from 'node:assert/strict'

import { buckyos, RuntimeType } from 'buckyos/node'

// ─── 配置 ───────────────────────────────────────────
const SYSTEM_CONFIG_URL =
  getEnv('BUCKYOS_SYSTEM_CONFIG_URL') ??
  'http://127.0.0.1:3200/kapi/system_config'
const VERIFY_HUB_URL =
  getEnv('BUCKYOS_VERIFY_HUB_URL') ??
  'http://127.0.0.1:3300/kapi/verify-hub'
const SERVICE_URL =
  getEnv('MY_SERVICE_URL') ??
  'http://127.0.0.1:<port>/kapi/<service-name>'
const TEST_APP_ID =
  getEnv('BUCKYOS_TEST_APP_ID') ?? 'control-panel'
const TEST_USER_ID =
  getEnv('BUCKYOS_TEST_USER_ID') ?? 'devtest'

function getEnv(name: string): string | null {
  const value = process.env[name]
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

// ─── 服务探测 ───────────────────────────────────────
async function probeRpc(
  url: string,
  method: string,
  params: Record<string, unknown>,
): Promise<void> {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ method, params, sys: [1] }),
  })
  if (!response.ok) {
    throw new Error(
      `${url} probe failed: ${response.status} ${response.statusText}`,
    )
  }
}

// ─── 身份认证 ───────────────────────────────────────
// （完整实现参照 test/aicc_test/aicc_smoke.ts 中的
//   createOwnerSignedLoginJwt + loginWithAppClient 函数）

async function login(): Promise<string> {
  // 1. 初始化 SDK
  await buckyos.initBuckyOS(TEST_APP_ID, {
    appId: TEST_APP_ID,
    runtimeType: RuntimeType.AppClient,
    zoneHost: '',
    defaultProtocol: 'https://',
    systemConfigServiceUrl: SYSTEM_CONFIG_URL,
    privateKeySearchPaths: [
      '/opt/buckyos/etc/.buckycli',
      '/opt/buckyos/etc',
      `${process.env.HOME ?? ''}/.buckycli`,
    ],
  })

  // 2. 创建 owner-signed JWT（或 fallback 到交互式登录）
  const accountInfo = await buckyos.login()
  if (!accountInfo?.session_token) {
    throw new Error('login did not return session_token')
  }

  // 3. 通过 verify-hub 换取正式 token
  const verifyHubRpc = new buckyos.kRPCClient(VERIFY_HUB_URL)
  const tokenPair = await verifyHubRpc.call('login_by_jwt', {
    type: 'jwt',
    jwt: accountInfo.session_token,
  })
  if (!tokenPair?.session_token) {
    throw new Error('verify-hub login failed')
  }

  return tokenPair.session_token
}

// ─── 测试主体 ───────────────────────────────────────
async function main(): Promise<void> {
  // Phase 1: 探测服务可达
  console.log('[probe] checking services...')
  await probeRpc(SYSTEM_CONFIG_URL, 'sys_config_get', { key: 'boot/config' })
  await probeRpc(SERVICE_URL, '<any_method>', {})
  console.log('[probe] ok')

  // Phase 2: 认证
  console.log('[auth] logging in...')
  const sessionToken = await login()
  console.log('[auth] ok')

  // Phase 3: 创建 RPC client
  const rpc = new buckyos.kRPCClient(SERVICE_URL, sessionToken)

  // Phase 4: 核心接口测试
  // （见下方「测试覆盖要求」）

  // Phase 5: 清理
  buckyos.logout(false)
  console.log('[done] all tests passed')
}

main().catch((error) => {
  console.error('DV test failed:', error)
  process.exitCode = 1
})
```

---

# 测试覆盖要求

DV Test 不是完整测试套件，而是**单点验证真实链路是否通畅**。但必须覆盖以下几类场景：

## A. 正常路径（MUST）

针对协议文档中每个核心接口，至少验证一次完整的请求-响应：

```typescript
// 示例：创建 → 读取 → 列表 → 删除 round-trip
console.log('[test] create...')
const id = await rpc.call('create', { name: 'dv-test-item', payload: '{}' })
assert.ok(id, 'create should return id')

console.log('[test] get...')
const record = await rpc.call('get', { id })
assert.equal(record.name, 'dv-test-item')

console.log('[test] list...')
const list = await rpc.call('list', {})
assert.ok(Array.isArray(list))
assert.ok(list.some(r => r.id === id), 'created item should appear in list')

console.log('[test] delete...')
await rpc.call('delete', { id })
const deleted = await rpc.call('get', { id })
assert.equal(deleted, null, 'deleted item should not exist')
```

## B. 身份与权限异常（MUST）

### B1. 无 token 访问

```typescript
console.log('[test] no-token access...')
const anonRpc = new buckyos.kRPCClient(SERVICE_URL)  // 不传 token
try {
  await anonRpc.call('create', { name: 'should-fail' })
  assert.fail('should reject unauthenticated request')
} catch (e) {
  // 预期：返回认证错误，而不是 500 或静默成功
  assert.ok(
    /auth|token|unauthorized|permission|401/i.test(String(e)),
    `expected auth error, got: ${e}`,
  )
}
```

### B2. 过期/无效 token 访问

```typescript
console.log('[test] invalid-token access...')
const badRpc = new buckyos.kRPCClient(SERVICE_URL, 'invalid.jwt.token')
try {
  await badRpc.call('get', { id: 'any' })
  assert.fail('should reject invalid token')
} catch (e) {
  assert.ok(
    /auth|token|unauthorized|invalid|401/i.test(String(e)),
    `expected auth error, got: ${e}`,
  )
}
```

### B3. 越权访问（若服务有用户隔离）

```typescript
// 用户 A 创建的资源，用户 B 不应能访问
// （需要两个不同身份的 session token）
```

## C. 输入异常（SHOULD）

### C1. 缺少必填字段

```typescript
console.log('[test] missing required field...')
try {
  await rpc.call('create', {})  // 缺少 name
  assert.fail('should reject missing required fields')
} catch (e) {
  // 预期：返回明确的参数错误，而不是 500
  assert.ok(
    /param|parse|required|missing|invalid/i.test(String(e)),
    `expected param error, got: ${e}`,
  )
}
```

### C2. 字段类型错误

```typescript
console.log('[test] wrong field type...')
try {
  await rpc.call('create', { name: 12345 })  // name 应为 string
  assert.fail('should reject wrong type')
} catch (e) {
  assert.ok(
    /param|parse|type|invalid/i.test(String(e)),
    `expected parse error, got: ${e}`,
  )
}
```

### C3. 资源不存在

```typescript
console.log('[test] not-found...')
try {
  await rpc.call('get', { id: 'nonexistent-id-12345' })
  // 可能返回 null 或抛错，视协议定义
} catch (e) {
  assert.ok(
    /not.?found|no.?such|does.?not.?exist/i.test(String(e)),
    `expected not-found error, got: ${e}`,
  )
}
```

## D. 幂等性验证（SHOULD，若协议声明幂等）

```typescript
console.log('[test] idempotency...')
const result1 = await rpc.call('create', {
  name: 'idempotent-test',
  idempotency_key: 'dv-idem-001',
})
const result2 = await rpc.call('create', {
  name: 'idempotent-test',
  idempotency_key: 'dv-idem-001',
})
assert.deepEqual(result1, result2, 'idempotent calls should return same result')
```

## E. 数据持久性验证（SHOULD）

验证数据在服务重启后是否保留（适用于 Developer Loop 阶段）：

```
1. 创建数据
2. stop.py → buckyos build → start.py（覆盖安装，数据不删）
3. 再次读取，验证数据仍在
```

## F. 并发基本验证（MAY）

```typescript
console.log('[test] concurrent writes...')
const promises = Array.from({ length: 5 }, (_, i) =>
  rpc.call('create', { name: `concurrent-${i}` }),
)
const results = await Promise.all(promises)
const ids = new Set(results.map(r => r.id ?? r))
assert.equal(ids.size, 5, 'concurrent creates should produce unique records')
```

---

# Developer Loop 集成

DV Test 脚本写好后，进入 Developer Loop 的标准循环：

```
npm test（运行 DV Test）
→ 判断结果是否正确
→ 若失败则读服务日志定位问题
→ 修改服务代码
→ stop.py
→ buckyos build
→ start.py          （覆盖安装，保留数据）
  或 start.py --all （全量重装，清空数据）
→ 再次 npm test
```

**何时用 `start.py`**: 只改了代码逻辑，数据格式不变。

**何时用 `start.py --all`**: 改了数据库 schema 或持久数据格式，且服务尚未上线（无需兼容旧数据）。

---

# 常见异常覆盖检查表

编写 DV Test 时，对照此表确认覆盖情况：

| 类别 | 场景 | 级别 | 说明 |
|------|------|------|------|
| **认证** | 无 token | MUST | 验证不会返回 500 或静默成功 |
| **认证** | 无效 token | MUST | 验证返回明确的认证错误 |
| **认证** | 过期 token | SHOULD | 验证 token 过期后被正确拒绝 |
| **权限** | 越权访问 | SHOULD | 用户 A 资源对用户 B 不可见（若有隔离） |
| **输入** | 缺少必填字段 | SHOULD | 验证返回参数错误而非 500 |
| **输入** | 字段类型错误 | SHOULD | 验证解析错误而非 panic |
| **输入** | 超长/超大输入 | MAY | 验证不会 OOM 或无限等待 |
| **资源** | 访问不存在的资源 | MUST | 验证返回 not-found 或 null |
| **资源** | 重复创建 | SHOULD | 验证幂等行为或明确冲突错误 |
| **资源** | 删除后再访问 | MUST | 验证删除生效 |
| **状态** | 非法状态转换 | SHOULD | 如对已完成的任务再次完成 |
| **并发** | 并发写入 | MAY | 验证不会丢数据或死锁 |
| **持久** | 覆盖安装后数据保留 | SHOULD | Developer Loop 中验证 |
| **链路** | 请求经过 gateway | MUST | 不直接打服务端口 |
| **Unknown** | 调用不存在的方法 | SHOULD | 验证返回 UnknownMethod 错误 |

---

# Common Failure Modes

## 1. 绕过 gateway 直打服务端口

**症状**: 测试在开发机通过，但通过 gateway 走就失败。
**原因**: 直接用 `127.0.0.1:<service-port>` 访问，绕过了 gateway 的认证和路由。
**修复**: DV Test 的 URL **MUST** 经过 gateway。在本地开发环境中，如果 gateway 尚未完全配置好路由到新服务，可以暂时直打端口做冒烟测试，但最终 DV Test **MUST** 验证完整链路。

## 2. 认证异常返回 500 而非明确错误

**症状**: 无 token 或错误 token 时服务返回 500 Internal Server Error。
**原因**: 服务代码中未处理认证失败场景，直接 unwrap token 导致 panic。
**修复**: Handler 中检查 `RPCContext` 的身份信息，认证失败时返回明确的 `RPCErrors`。

## 3. 缺少必填字段时服务 panic

**症状**: 发送不完整参数时服务进程崩溃重启。
**原因**: `from_json()` 解析失败后 unwrap 而非返回 `ParseRequestError`。
**修复**: 所有 `from_json` 必须正确 map_err 到 `RPCErrors::ParseRequestError`。

## 4. 测试数据污染

**症状**: 测试首次通过，再跑一次就失败（如唯一键冲突）。
**原因**: 测试未清理创建的数据，或使用了固定 ID。
**修复**: 使用随机前缀或时间戳生成测试 ID，测试结束后清理创建的资源。

## 5. 依赖服务未启动

**症状**: 测试脚本启动后立即报 `ECONNREFUSED`。
**原因**: 被测服务或其依赖（system-config、verify-hub）未运行。
**修复**: 在测试开头 `probeRpc` 探测关键服务的可达性，给出明确的前置条件失败提示。

## 6. token 获取失败

**症状**: `login()` 阶段报错，无法获取 session token。
**原因**: owner private key 路径不对，或 verify-hub 未就绪。
**修复**: 检查 `privateKeySearchPaths` 是否包含当前环境的实际路径；确认 verify-hub 已启动且 login_by_jwt 可用。

## 7. 异步任务超时

**症状**: 涉及长任务的测试超时退出。
**原因**: 轮询间隔太长、超时设置不合理、或任务本身卡住。
**修复**: 设置合理的超时时间（通过环境变量可配置），轮询间隔 1-2 秒，超时后输出最后已知的任务状态帮助诊断。

---

# Pass Criteria

DV Test 通过需同时满足：

- [ ] TS SDK 可用（`npm install` 成功）
- [ ] 测试脚本能运行（`npm test` 不报语法/导入错误）
- [ ] 能获取 session token（login 链路正常）
- [ ] 请求通过 gateway 到达服务
- [ ] 核心接口正常路径全部通过
- [ ] 无 token / 无效 token 被正确拒绝（不返回 500）
- [ ] 不存在的资源返回 not-found 而非 500
- [ ] 测试可重复运行（不会因残留数据失败）
