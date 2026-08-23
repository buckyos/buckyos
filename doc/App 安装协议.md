# BuckyOS App 安装协议（beta 2.2）

本文是 beta 2.2 App 身份、AppDoc、PIKG、InstallPlan、Scheduler 执行事务和 Node 执行投影的权威协议。beta 2.2 是 breaking change，只支持从空 SystemConfig 初始化，不读取、不迁移 beta 2.1 及更早的 App 安装数据。

## 1. 身份模型

```text
AppDID -> AppId
AppId + owner_user_id -> AppInstanceId
AppInstanceId + node_id -> ReplicaKey
AppDoc ObjectId + spec_generation -> DeploymentIdentity
```

- `AppDID` 是产品身份，必须是 canonical lowercase hostname-form DID，不得含 path、port、fragment 或 percent encoding。
- `AppId` 是 `AppDID.to_raw_host_name()` 的严格可逆结果。解析后必须满足 `DID::from_str(AppId) == AppDID`；多 label 非 Web DID 使用 `<id>.<method>.did`。
- `AppInstanceId` 的 canonical 字符串是 `<app_id>@<owner_user_id>`，不含版本、Node、短域名或 AppIndex。
- `ReplicaKey` 是 Scheduler 内部结构化的 `(AppInstanceId, node_id)`，不是公共字符串身份。
- `DeploymentIdentity` 精确绑定 `app_instance_id/task_id/app_doc_object_id/spec_generation/pikg_digest?`。
- 非 `did:` 的系统服务名先归类为 `SystemServiceId`，不得先交给 DID parser。共享服务身份使用显式 `App | System` 分支。

`AppType::Agent` 只表示 App 产品提供 Agent runtime。Agent 本身使用独立的 AgentDID、AgentDocument、AgentId 和 AgentSpec；AgentSpec.binding 指向普通 `AppInstanceId + service_name`。多个 Agent 可以共享一个 runtime，删除 Agent binding 不卸载 runtime，卸载 runtime 前必须拒绝仍被引用的目标。

## 2. AppDoc v1

### 2.1 完整 JSON Schema

`AppDoc` flatten/inherit `BaseContentObject`，因此它仍是可以通过 Named Object/NDN 流转的内容对象；但它不再 inherit `PackageMeta`。以下 schema 冻结 AppDoc body。根对象、`SubPkgDesc`、selector、presentation、permission、endpoint、mount、bash env 和 instance volume 都拒绝未知字段；`service_config_tips` 为显式自定义配置保留 additional properties。

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://buckyos.ai/schema/appdoc-v1.json",
  "title": "BuckyOS AppDoc v1",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema_version", "doc_type", "did", "version", "app_type",
    "owner", "controller", "author", "create_time", "last_update_time",
    "exp", "pkg_list", "show_name", "selector_type", "service_config_tips"
  ],
  "properties": {
    "schema_version": { "const": 1 },
    "doc_type": { "const": "app" },
    "did": { "$ref": "#/$defs/did" },
    "name": { "type": "string", "minLength": 1 },
    "copyright": { "type": "string" },
    "tags": {
      "type": "array", "minItems": 1,
      "items": { "type": "string" }
    },
    "categories": {
      "type": "array", "minItems": 1,
      "items": { "type": "string" }
    },
    "base_on": { "type": "string", "pattern": "^[^:]+:[0-9a-f]+$" },
    "directory": {
      "type": "object", "minProperties": 1,
      "additionalProperties": { "type": "object" }
    },
    "references": {
      "type": "object", "minProperties": 1,
      "additionalProperties": { "type": "object" }
    },
    "version": { "type": "string", "minLength": 1 },
    "version_tag": { "type": "string" },
    "app_type": { "enum": ["service", "dapp", "web", "agent"] },
    "owner": { "$ref": "#/$defs/did" },
    "controller": { "$ref": "#/$defs/did" },
    "author": { "$ref": "#/$defs/did" },
    "create_time": { "type": "integer", "minimum": 0 },
    "last_update_time": { "type": "integer", "minimum": 0 },
    "exp": { "type": "integer", "minimum": 1 },
    "pkg_list": {
      "type": "object",
      "additionalProperties": { "$ref": "#/$defs/subPackage" }
    },
    "show_name": { "type": "string", "minLength": 1 },
    "presentation": { "$ref": "#/$defs/presentation" },
    "sdk_version": { "type": "string" },
    "req_capbilities": {
      "type": "object",
      "additionalProperties": { "type": "integer" },
      "default": {}
    },
    "permissions": {
      "type": "array",
      "items": { "$ref": "#/$defs/permission" },
      "default": []
    },
    "selector_type": {
      "oneOf": [
        { "enum": ["single", "static", "random", "by_event"] },
        { "type": "string", "minLength": 1 }
      ]
    },
    "service_config_tips": { "$ref": "#/$defs/serviceConfigTips" }
  },
  "allOf": [
    {
      "if": { "properties": { "app_type": { "not": { "const": "service" } } } },
      "then": { "properties": { "pkg_list": { "minProperties": 1 } } }
    }
  ],
  "$defs": {
    "did": { "type": "string", "pattern": "^did:[a-z0-9]+:[a-z0-9.-]+$" },
    "stringMap": {
      "type": "object",
      "additionalProperties": { "type": "string" }
    },
    "selector": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "os": { "type": "string" },
        "arch": { "type": "string" },
        "min_kernel_version": { "type": "string" }
      }
    },
    "subPackage": {
      "type": "object",
      "additionalProperties": false,
      "required": ["pkg_id", "pkg_objid"],
      "properties": {
        "pkg_id": { "type": "string", "minLength": 1 },
        "pkg_objid": { "type": "string", "pattern": "^pkg:[0-9a-f]{64}$" },
        "docker_image_name": { "type": "string", "minLength": 1 },
        "docker_image_digest": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
        "source_url": { "type": "string", "minLength": 1 },
        "selector": { "$ref": "#/$defs/selector" },
        "required": { "type": "boolean", "default": true }
      }
    },
    "permission": {
      "type": "object",
      "additionalProperties": false,
      "required": ["scope_path", "required"],
      "properties": {
        "scope_path": { "type": "string", "minLength": 1 },
        "required": { "type": "boolean" },
        "actions": { "type": "array", "items": { "type": "string" }, "default": [] },
        "exp": { "type": ["integer", "null"], "minimum": 0 }
      }
    },
    "presentation": {
      "type": "object",
      "additionalProperties": false,
      "required": ["title", "summary", "description", "icons", "links", "license"],
      "properties": {
        "title": { "$ref": "#/$defs/stringMap" },
        "summary": { "$ref": "#/$defs/stringMap" },
        "description": { "$ref": "#/$defs/stringMap" },
        "icons": { "type": "object", "additionalProperties": { "type": "string" } },
        "links": { "$ref": "#/$defs/stringMap" },
        "license": { "type": "string" }
      }
    },
    "exposeRoute": {
      "oneOf": [
        {
          "type": "object", "additionalProperties": false,
          "required": ["type"], "properties": { "type": { "const": "web" } }
        },
        {
          "type": "object", "additionalProperties": false,
          "required": ["type"],
          "properties": {
            "type": { "const": "port" },
            "preferred_port": { "type": "integer", "minimum": 1, "maximum": 65535 }
          }
        }
      ]
    },
    "expose": {
      "type": "object",
      "additionalProperties": false,
      "required": ["route"],
      "properties": {
        "route": { "$ref": "#/$defs/exposeRoute" },
        "scope": { "type": "string", "default": "" },
        "allow_guest": { "type": "boolean", "default": false }
      }
    },
    "endpoint": {
      "type": "object",
      "additionalProperties": false,
      "required": ["protocol", "inner_port"],
      "properties": {
        "protocol": { "enum": ["http", "https", "tcp", "udp"] },
        "inner_port": { "type": "integer", "minimum": 1, "maximum": 65535 },
        "required": { "type": "boolean", "default": false },
        "description": { "$ref": "#/$defs/stringMap" },
        "expose": { "$ref": "#/$defs/expose" }
      }
    },
    "mount": {
      "type": "object",
      "additionalProperties": false,
      "required": ["mount_point_name", "access", "reason"],
      "properties": {
        "mount_point_name": { "type": "string" },
        "access": { "enum": ["read_only", "read_write", "read_write_append"] },
        "reason": { "$ref": "#/$defs/stringMap" }
      }
    },
    "bashEnv": {
      "type": "object",
      "additionalProperties": false,
      "required": ["required"],
      "properties": {
        "required": { "type": "boolean" },
        "description": { "$ref": "#/$defs/stringMap" }
      }
    },
    "instanceVolume": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "mode": { "enum": ["required", "optional", "disabled"], "default": "required" },
        "quota_mib": { "type": "integer", "minimum": 0 },
        "ephemeral_contents": { "type": "array", "items": { "type": "string" }, "default": [] }
      }
    },
    "serviceConfigTips": {
      "type": "object",
      "properties": {
        "service_endpoints": { "type": "object", "additionalProperties": { "$ref": "#/$defs/endpoint" } },
        "data_mount_points": { "type": "object", "additionalProperties": { "oneOf": [{ "$ref": "#/$defs/mount" }, { "type": "null" }] } },
        "local_cache_mount_points": { "type": "object", "additionalProperties": { "oneOf": [{ "$ref": "#/$defs/mount" }, { "type": "null" }] } },
        "external_mount_points": { "type": "object", "additionalProperties": { "oneOf": [{ "$ref": "#/$defs/mount" }, { "type": "null" }] } },
        "rdb_instances": { "type": "object" },
        "instance_volume": { "$ref": "#/$defs/instanceVolume" },
        "bash_envs": { "type": "object", "additionalProperties": { "$ref": "#/$defs/bashEnv" } },
        "runtime_caps": { "$ref": "#/$defs/stringMap" },
        "container_param": { "type": "string" },
        "start_param": { "type": "string" }
      },
      "additionalProperties": true,
      "default": {}
    }
  }
}
```

Serde/default 规则：

- `did/author/owner/create_time/last_update_time/exp` 来自 flattened `BaseContentObject`，其中 AppDoc specialization 继续要求它们有效且必填。
- `name/copyright/tags/categories/base_on/directory/references` 是 `BaseContentObject` 的可选内容元数据；空 `name/tags/categories/directory/references` 序列化时省略。`name` 不是 App 身份，也不得用于推导 AppId。
- `version_tag/presentation/sdk_version` 缺省为 `null` 且序列化时省略。
- `req_capbilities/permissions` 缺省为空并省略。
- `SubPkgDesc.required` 缺省解释为 `true`；PIKG 的 canonical 输出必须显式写出该布尔值。
- selector 的 `os/arch/min_kernel_version` 均可省略；`amd64/x86_64/x64` 归一为 `x86_64`，`arm64/aarch64` 归一为 `aarch64`，`darwin/apple/osx` 归一为 `macos`。
- 已知 package key 的 selector：各平台 key 派生相应 OS/arch；`web/agent/agent_skills/agent_tools/script` 匹配所有平台；未知 key 必须显式提供 selector 才参与自动选择。
- 根对象不允许 schema 之外的扩展键；`service_config_tips` 的自定义键以及所有显式 BaseContentObject 元数据仍进入 AppDoc ObjectId。

### 2.2 App 类型约束

- `service`：只表示 `SystemBuiltin` 的发布元数据，允许空 `pkg_list`，不得有 web、docker 或 Agent package；它不进入普通 App 安装、AppRegistry 或用户 AppSpec。
- `dapp`：必须有 script 或 docker runtime，不得有 web/Agent package。
- `web`：必须有 web package，不得有 runtime/Agent package，`selector_type` 固定 `static`。
- `agent`：必须有 agent package，可含 agent_skills/agent_tools，不得有 web/native app。它仍是 runtime AppDoc，不是 AgentDocument。

### 2.3 权威、签名与 ObjectId

AppDoc body 从 `BaseContentObject` 继承的 `owner` 表示发布对象所有者，AppDoc 自有的 `controller` 决定可接受的 verification method，BaseContentObject 的 `author` 是本次签名者。冻结的 detached envelope 为：

```json
{
  "schema_version": 1,
  "app_doc_object_id": "appdoc:<64 lowercase hex>",
  "signer": "did:...",
  "controller": "did:...",
  "verification_method": "did:...#key-id",
  "algorithm": "EdDSA",
  "signature": "<base64url-no-pad Ed25519 signature>"
}
```

验证必须同时满足：schema 版本为 1；body 自身通过严格验证；`signer == AppDoc.author`；`controller == AppDoc.controller`；verification method 以 `<controller>#` 开头；算法为 `EdDSA`；签名验证 key 由 controller 的已验证 DID Document 提供。任一失败都 fail closed，不返回弱验证 snapshot。

ObjectId material 是 AppDoc body 经 serde default/omit 规则正规化后的 JSON，再执行 RFC 8785 JCS。`ObjType` 固定 `appdoc`：

```text
AppDocObjectId = appdoc:lowercase_hex(SHA256(JCS(AppDoc body)))
signature bytes = Ed25519.sign(JCS(AppDoc body))
```

Detached signature envelope、resolver metadata、缓存时间和网络来源不进入 ObjectId。JSON 空白、对象 key 顺序和等价数字表示经 JCS 统一；发布者必须使用 PIKG canonical 输出，尤其不能混用缺省和显式 default 表达。

Golden fixture：

- [`fixtures/appdoc-v1.json`](fixtures/appdoc-v1.json)
- [`fixtures/appdoc-v1.jcs`](fixtures/appdoc-v1.jcs)
- [`fixtures/appdoc-v1.object-id`](fixtures/appdoc-v1.object-id)
- [`fixtures/appdoc-v1.signature-envelope.json`](fixtures/appdoc-v1.signature-envelope.json)
- [`fixtures/appdoc-v1.verifying-key`](fixtures/appdoc-v1.verifying-key)

冻结 ObjectId 是 `appdoc:39e65ed735f588cd1193d349a072f21fecde6df519402bbeb7967a61fe5d9685`。Rust serde/signature test 与 TypeScript PIKG test 必须共同消费这些文件。

### 2.4 Resolver 校验顺序

解析 `(AppDID, doc_type=app)` 后按顺序执行：

1. 验证请求 AppDID 的 hostname profile 和 raw-hostname round-trip。
2. 验证 resolver 返回的 DID Document 权威链、状态、版本和 controller key。
3. 严格反序列化 AppDoc，接收已冻结的 BaseContentObject 字段，拒绝缺字段、未知字段、unknown schema 和旧 PackageMeta flatten。
4. 验证 `AppDoc.did == requested AppDID`。
5. 对 body 做 JCS，重算 AppDoc ObjectId，并与 resolver snapshot 的 ObjectId 相等。
6. 验证 detached envelope 的 signer/controller/verification method 和 Ed25519 签名。
7. 对每个自有 SubPkgDesc 验证 package namespace、版本和 PackageMeta ObjectId。
8. 读取 PackageMeta body，重算 `pkg:<sha256>`，验证 name/version 与 AppDoc PackageId，最后验证 selector/target 与 payload content identity。

任何一步失败都不得进入 InstallPlan。拒绝测试至少覆盖：缺 required、unknown field、错误 signer/controller、AppDID 不一致、AppDoc ObjectId 不一致、PackageMeta ObjectId/name/version 不一致和旧 `size/content/deps/meta` PackageMeta flatten 格式。`BaseContentObject.name` 是合法的非身份元数据。

## 3. Package namespace 与 exact 内容

App 自有 namespace 固定为 `AppId`。允许的 unique name：

```text
<app_id>
<single-safe-subpackage-label>.<app_id>
```

前面可以有 PackageEnv 认可的 qualifier，例如 `all.`、`nightly-linux-amd64.`。判断 namespace 时先用统一 PackageId parser 去除认可的 qualifier，再对完整 AppId 做 suffix 匹配；不得按固定 dot 数或最后一段判断。

AppDoc 中使用语义版本 selector：

```text
all.web.app.example#1.0.0
```

进入 Deploy 的 `SelectedPackage/DeploymentPackage/NodeExecutionSpec` 必须转换为 exact PackageId：

```text
all.web.app.example#pkg:<64 lowercase hex>
```

exact PackageId 的 ObjectId 必须与 `pkg_objid/package_meta_id` 相等。PackageMeta body 重算 ObjectId 后，其 `name` 必须等于带 qualifier 的 PackageId name，`version` 必须等于 AppDoc 声明版本。node-daemon 只按 exact PackageId 安装和加载严格目录，不通过 mutable friendly/latest alias 选择运行内容。

## 4. SystemConfig 真相与投影

```text
system/app_registry
users/{owner}/apps/{app_id}/spec
users/{owner}/apps/{app_id}/install
users/{owner}/agents/{agent_id}/spec
users/{owner}/agents/{agent_id}/install
nodes/{node_id}/config
services/{app_instance_id}/info
services/{app_instance_id}/instances/{node_id}
```

`system/app_registry` schema v1 是唯一分配真相：

- `apps[AppId] = {app_did, app_name, allocated_at}`
- `instances[AppInstanceId] = {app_id, owner_user_id, app_host_name, app_index, allocated_at}`
- `next_app_index` 从 1 单调增加，最大可分配值 3470；不因卸载回收。
- Registry 整体 JSON 只由 Scheduler 通过 SystemConfig CAS 更新。unknown schema、损坏引用或投影不一致时 fail closed。
- Zone Owner 来自 `system/zone_owner_user_id`；Owner 的默认 host 可省略 owner suffix，普通用户使用 DNS-safe owner suffix。
- `_`、`www`、`sys`、已分配 app/instance hostname 和 shortcut 共用同一冲突域。

`AppSpec.app_name/app_host_name/app_index` 与 `NodeExecutionSpec` 同名字段都是 `read_only, derived_from=system/app_registry` 的 Scheduler 投影，调用者不能提供或回写。

`NodeConfig.apps` 的 key 是 AppInstanceId，不拼 NodeId。每个 value 携带一个完整 `NodeExecutionSpec v1`：AppInstanceId、AppDID、AppDoc ObjectId、spec generation、AppType、已选 exact package map、权限、ServiceSpecConfig 和 Registry 投影。它不嵌入完整 AppDoc/AppSpec。node-daemon 只读同一 NodeConfig revision 即可执行，禁止回读 AppSpec、AppDoc 或 Registry 来补字段。

beta 2.2 bootstrap 先创建空 Registry，再把预装 App/runtime 作为 `InstallPlanExecutionRecord(state=pending)` 交给 Scheduler。builder 不直接创建普通 AppSpec 或 AgentSpec。Registry 缺失或 schema 不支持时 Scheduler 拒绝运行并要求从空 SystemConfig 重建。

## 5. InstallPlan execution protocol

Control Panel 负责 Resolve、Inspect、用户批准并生成 immutable InstallPlan；它不得直接写 Registry、AppSpec、NodeConfig 或 gateway 派生配置。Scheduler kRPC 冻结四个 plan 方法：

```text
submit_install_plan({ plan }) -> InstallPlanExecutionRecord
get_install_plan_status({ key }) -> InstallPlanExecutionRecord
cancel_install_plan({ key }) -> InstallPlanExecutionRecord
retry_install_plan({ key }) -> InstallPlanExecutionRecord
```

`InstallPlanExecutionKey = (app_instance_id, task_id, plan_fingerprint)`；持久 claim key 是三者 canonical 字符串的 SHA-256：

```text
system/scheduler/install_plan_executions/{64 lowercase hex}
```

plan fingerprint 使用 schema、AppDoc snapshot/ObjectId、resolver snapshot、target、安装参数、最终 ServiceSpecConfig、selected exact packages 和 required contents 的 JCS identity。它不包含 Scheduler 分配的 AppName、AppHostName、AppIndex 或 `www` instance port。

状态：`pending -> claimed -> committed -> scheduled -> completed`，失败为 `failed`，claim 前可 `canceled`。commit point：`before_claim -> claimed -> desired_state_committed -> node_config_published`。

- 相同 key 重放返回同一 record，不重复分配 hostname/index。
- 同一 AppInstanceId 的不同 fingerprint 通过 Registry/AppSpec CAS 串行；冲突重试上限后返回明确冲突。
- `desired_state_committed` 之前失败可以 retry 或 cancel；之后 Registry/AppSpec 已是 durable truth，cancel 必须拒绝，retry 只恢复后续调度。
- Scheduler 重启扫描 pending/claimed/desired-state-committed/failed records；commit 前重新 claim，commit 后继续 schedule，不重新分配。
- AppSpec、InstallRecord 和 Registry 在同一 SystemConfig transaction 内提交；NodeConfig 可在后续调度轮次从 desired state 完整重建。

Shortcut 也提交 Scheduler：

```text
mutate_shortcut({ plan }) -> SchedulerShortcutMutationRecord
```

它与 Registry 默认 hostname 在同一串行协调域中校验，再 CAS 写 gateway settings。目标必须是精确 AppInstanceId；系统目标使用显式 System 分支。

## 6. Agent 投影

`AgentSpec v1` 保存 AgentId、AgentDID、AgentDocument snapshot/ObjectId、generation 和 `AgentServiceBinding v1`。binding 重复保存并校验 AgentDID/ObjectId，且必须包含目标 AppInstanceId、service_name 和 generation。

Scheduler 将 Agent gateway/service endpoint 投影到 binding 的目标 runtime service；RBAC group 和 verify-hub token principal 使用 canonical AgentDID。node-daemon 只运行普通 runtime AppInstance，不创建 Agent 容器身份。runtime 卸载前 Control Panel 扫描所有用户的 AgentSpec；任何 binding 引用都会拒绝卸载。

## 7. Runtime 与鉴权

固定环境变量：

```text
BUCKYOS_APP_DID
BUCKYOS_APP_ID
BUCKYOS_APP_INSTANCE_ID
BUCKYOS_OWNER_USER_ID
BUCKYOS_DATA_DIR
```

数据目录按 `(owner_user_id, AppId)` 隔离且跨升级稳定。Docker 容器名是 `buckyos-app-{AppHostName}`，其中 AppHostName 必须直接取自 NodeExecutionSpec 的 Registry 投影；instance volume/systemd 使用的 RuntimeKey 是 `lowercase_hex(SHA256(canonical AppInstanceId bytes))` 的完整 64 hex。Docker label 同时保存完整 AppDID、AppInstanceId、owner、exact PackageId/PackageMeta ObjectId、AppDoc ObjectId 和 generation。回收前必须用 label 复核完整身份。

普通 App token 的 `appid` 是 AppId，并必须另带、精确比较 AppInstanceId；系统 principal 的兼容 appid 字段只有在 principal kind 为 System 时才解释为 SystemServiceId。Agent principal kind 使用 AgentDID。

## 8. Golden/rejection gate

合入或修改协议时必须通过：

- Rust AppDoc serde/JCS/ObjectId/signature golden test。
- TypeScript PIKG 对同一 fixture 的 schema 与 ObjectId golden test。
- AppDID Web/BNS 多 label round-trip、path/fragment/保留 `.did` rejection。
- AppRegistry owner 隔离、hostname 冲突、index 上限、unknown schema rejection。
- exact PackageId、PackageMeta hash/name/version、namespace rejection。
- NodeExecutionSpec 与 DeploymentIdentity generation 一致性。
- InstallPlan fingerprint/idempotency/retry/cancel/recovery测试。
- 多 Agent 共享 runtime 与卸载引用保护测试。
