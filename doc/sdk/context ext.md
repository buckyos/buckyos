# BuckyOS 内容扩展机制（Content Extension）

> 文档状态：Draft v0.1
> 日期：2026-09-04
> 上游需求：`product/bucky_file/BuckyOS Preview App-Component PRD.md`（Preview Pipeline 扩展注册与匹配、Full App、Open With）、`src/frame/desktop/src/app/canvas/BuckyOS AI Canvas PRD.md`（表格内 AI 单元格、自定义交互块）、`product/bucky_file/filebrowser_PRD.md`（图标视图、右侧 Meta 面板）。
> 关联协议：`doc/App 安装协议.md`（AppDoc v1、`system/app_registry`、真相与投影）、`doc/key url.md`（`cyfs://` / `obj://` / `buckyos://`）。

---

## 1. 目标

BuckyOS 的内容体系围绕 Named Data Object 展开：传统文件、CYFS 路径、Named Object 以及应用内部的结构化数据都是“内容”。系统需要一个统一的机制回答一个问题：

> **对于某一种类型的内容，在某个使用场景（意图）下，应该由谁、以什么方式来处理？**

本文定义 **Content Extension** 机制：

1. 统一的 **内容类型（Content Type）** 描述方式：非结构化内容用 MIME，结构化内容用 Named Object Type 或 Schema URL；
2. 统一的 **意图（Intent）** 命名空间：`preview`、`open`、`cell.view`、`cell.edit` 是首批意图，架构允许持续扩展；
3. 一张 **内容处理注册表（Content Registry）**：App 通过 AppDoc 声明自己能处理哪些内容类型的哪些意图，安装后由系统投影到注册表；
4. 一个 **解析（Resolve）算法** 和 **调用（Invoke）模型**：宿主 App 或系统组件只描述“内容 + 意图”，由系统选出处理者并在合适的时机以合适的方式调用 App。

核心结构可以概括为：

```text
ContentType  ──▶  Intent(op)  ──▶  Handler(App / Component / Function / Builtin)
   选择器            意图             注册表条目 + 调用方式
```

## 2. 非目标

- 不规定任何具体格式的转换算法、渲染实现或编辑器实现；
- 不替代 AppDoc 权限模型（`permissions[].scope_path`）和 RBAC，Handler 只能在 App 自身权限内工作；
- 不在本文定义扩展市场、格式包分发；
- 不定义 Preview Component 内部 UI（见 Preview PRD 第 10 章）和 Canvas 编辑器内部状态模型（见 Canvas PRD 第 12 章）。

## 3. 三档需求与共同结构

| 档位 | 场景 | 意图 | 调用方式 | 输入 | 输出 |
|---|---|---|---|---|---|
| 1. Preview | File Browser 图标 / 缩略图、Preview Component 预览、IM / Mail 内嵌预览 | `icon`、`preview` | 声明式（icon）；`invoke` 转换器或 `embed` 渲染器（preview） | 内容引用 + 目标 Profile | 图标资源 / 标准 Preview Result |
| 2. Open with… | 双击、右键“打开方式”、Preview 中“使用专用应用打开” | `open` | `launch` 拉起 App 窗口/页面 | Source + Session Context | 无（App 接管） |
| 3. View Cell / Edit Cell | AI Canvas 表格单元格、结果组中的自定义块、未来 IM 卡片 | `cell.view`、`cell.edit` | `embed` 沙箱组件 | 结构化值 + 类型 | 渲染 / 新值与修订 |

三档看起来差异很大，但共享同一套结构：**内容类型选择器 → 意图 → 注册表条目 → 调用契约**。只有“调用契约”按意图不同而不同。后续新增意图（例如 `convert`、`enrich`、`create`、`share`、`print`）只需要补一份调用契约，不需要改注册表结构。

---

## 4. 内容类型（Content Type）

### 4.1 内容描述符（ContentDescriptor）

系统在解析前先把任何内容引用归一化为一个 **ContentDescriptor**。它是匹配的唯一依据，宿主传入的扩展名和 MIME 只是 hint。

```ts
interface ContentDescriptor {
  /** 原始引用，仅用于回显、鉴权与错误定位 */
  source: ContentRef;               // { kind: "cyfs-path" | "object-id" | "inline" | ..., ... }
  /** 不可变内容身份；path 必须先锚定为 ObjId（Preview PRD §9.3） */
  objectId?: string;                // 形如 "cyfile:<hex>"
  /** Named Object 类型，来自 ObjId 前缀：cyfile / cydir / cypack / appdoc / buckyos.group_doc ... */
  objType?: string;
  /** 非结构化内容的媒体类型，RFC 6838，含可选参数与结构化后缀，如 image/png、application/ld+json */
  mime?: string;
  /** 结构化内容的 Schema URL（JSON Schema `$id` 或系统约定的 schema 地址） */
  schema?: string;
  /** 仅作 hint：扩展名（小写、含点）、文件名、大小、版本 */
  ext?: string;
  name?: string;
  size?: number;
  version?: string;
  /** 有界内容探测结果（magic bytes、容器签名等），由系统 Sniffer 填写 */
  sniff?: Record<string, unknown>;
}
```

归一化规则：

| 内容来源 | objType | mime | schema |
|---|---|---|---|
| `cyfs:///...` 指向的文件 | `cyfile` | FileObject 元数据中的 `mime`，缺失时由 Sniffer 从内容前缀探测，扩展名只作 fallback hint | 若 `mime` 为 `application/json` 等结构化类型且内容含 `$schema`，可提取 |
| `cyfs:///...` 指向的目录、ZIP 内目录 | `cydir` / 容器对象类型 | 无 | 无 |
| `obj://` Named Object | ObjId 前缀 | 对象声明的 `content_type`（若有） | 对象声明的 `schema`（若有） |
| App 内部结构化值（Canvas 单元格、IM 卡片 payload） | 无 | 通常 `application/json` | **必填**，例如 `https://buckyos.ai/schema/canvas-chart-v1.json` |

> 规则：**非结构化内容用 MIME 表达，结构化内容用 Named Object Type 或 Schema URL 表达。** 两者可以同时存在（一个 `cyfile` 同时有 `mime`），匹配时都参与。安全敏感格式不得只按扩展名判定（Preview PRD §9.5）。

### 4.2 内容类型选择器（ContentSelector）

注册表中 App 用 **选择器** 声明自己关心的内容。选择器是一组条件，全部满足才匹配；数组字段表示“任一”。

```ts
interface ContentSelector {
  objType?: string | string[];        // "cyfile" | ["cyfile","cypack"]
  mime?: string | string[];           // "image/png" | "image/*" | "*/*" | "application/*+json"
  schema?: string | string[];         // 精确 URL；允许尾部 "*" 表示版本族，如 ".../canvas-chart-v*"
  ext?: string[];                     // 仅 hint，只能提升排序，不能独立命中
  sniff?: Record<string, unknown>;    // 可选内容特征约束，由系统 Sniffer 插件解释
  maxSize?: number;                   // 超过则不匹配（避免大文件交给不适合的 Handler）
}
```

**紧凑字符串形式**（用于文档、CLI、日志），可与对象形式互转：

```text
mime:image/png
mime:image/*
obj:cyfile;mime:application/pdf
obj:cydir
schema:https://buckyos.ai/schema/canvas-chart-v1.json
```

### 4.3 特异性（Specificity）

同一意图下多个条目命中时，先比较特异性，再比较优先级（§7）。特异性由高到低：

1. `schema` 精确命中；
2. `objType` 精确命中且 `mime` 精确命中（含参数）；
3. `mime` 精确命中（忽略参数）；
4. `mime` 结构化后缀命中（`application/*+json`）；
5. `mime` 子类型通配（`image/*`）；
6. 仅 `objType` 命中；
7. `mime: */*` 或仅 `ext` hint 命中（只允许系统内建条目和用户显式指定的条目在此档位胜出）。

---

## 5. 意图（Intent）

意图是“对内容做什么”的稳定命名。命名规则：小写、点分层级、由系统冻结语义；第三方可注册私有意图，必须使用反向域名前缀（如 `com.example.annotate`），系统不对私有意图做 UI 集成。

### 5.1 首批系统意图

| 意图 | 语义 | 调用方式 | 宿主 | 状态 |
|---|---|---|---|---|
| `icon` | 某类内容在列表 / 图标视图中的静态图标，不读取内容 | 声明式，不调用 | File Browser、Preview 错误态、Open With 菜单 | P0 |
| `preview` | 生成 Preview Component 可消费的标准结果（含 `purpose: preview | thumbnail`） | `invoke`（转换器）或 `embed`（渲染器） | Preview Component / Preview App、File Browser 缩略图 | P0 |
| `open` | 以完整语义打开内容，App 接管窗口与生命周期 | `launch` | 桌面 / 窗口管理器 | P0 |
| `cell.view` | 在宿主提供的固定区域内只读渲染一个结构化值 | `embed` | AI Canvas、IM 卡片、Mail 附件卡片 | P1 |
| `cell.edit` | 在宿主区域内编辑一个结构化值并回传新值 | `embed` | AI Canvas | P1 |

### 5.2 预留意图（架构占位，本文不定义契约）

| 意图 | 用途 |
|---|---|
| `convert` | 通用格式转换（导出、另存为），是 `preview` 转换器的一般化 |
| `enrich` | 为 File Browser Meta 面板提取语义标签、摘要、EXIF 等（filebrowser PRD §9.6） |
| `create` | “新建某类内容”菜单，Handler 负责创建初始内容 |
| `enumerate` | 把容器类内容（ZIP、相册对象）展开为 Session Items（Preview PRD §11.3） |
| `share` / `print` | 分享、打印 |

新增意图的要求：在本文追加一节“调用契约”，定义 **调用方式、输入、输出、宿主义务、错误分类**；不得改变注册表条目结构。

---

## 6. 注册表（Content Registry）

### 6.1 真相与投影

沿用 `doc/App 安装协议.md` §4 的“真相与投影”模型：

```text
AppDoc.content_handlers            # 开发者声明（签名、随版本冻结、进入 AppDoc ObjectId）
        │ install / upgrade / uninstall
        ▼
system/content_registry            # Zone 级投影，Installer/Scheduler 通过 SystemConfig CAS 更新
        │ + users/{user}/content_defaults   # 用户偏好（默认打开方式等），用户可写
        ▼
ContentRegistry.resolve()          # 运行时只读查询，输出固定到版本的 HandlerPlan
```

- **AppDoc 是声明真相**。App 不能在运行时动态注册；换句话说注册表内容一定可以由“已安装 App 的 AppDoc 集合”重新推导出来，损坏时可以重建。
- **`system/content_registry` 是唯一投影真相**，只由 Installer / Scheduler 写入，unknown schema 时 fail closed。
- **用户偏好单独存放**，不混入投影，卸载 App 时投影条目移除，指向它的偏好自动失效（解析时忽略并回落）。
- 系统内建 Handler（内建 Renderer、Preview App、系统图标集）以 `provider: "system"` 形式存在于同一张表，与第三方条目走同一解析算法，避免两套逻辑。

### 6.2 AppDoc 中的声明

AppDoc v1 根对象 `additionalProperties: false`，因此需要 **显式增加可选字段 `content_handlers`**（schema 修订，字段进入 AppDoc ObjectId）。声明结构：

```json
{
  "content_handlers": [
    {
      "handler_id": "md-preview",
      "version": 2,
      "selectors": [
        { "mime": ["text/markdown", "text/x-markdown"], "ext": [".md", ".markdown"] }
      ],
      "intents": {
        "icon": {
          "icons": { "16": "icons/md-16.svg", "64": "icons/md-64.svg" }
        },
        "preview": {
          "kind": "converter",
          "entry": { "type": "rpc", "endpoint": "api", "method": "content.preview" },
          "outputs": ["text/html;profile=safe", "text/plain"],
          "features": ["streaming"],
          "quality": { "fidelity": "high", "latency": "fast", "cost": "low" },
          "sandbox": { "network": false, "fs": "input-only" },
          "priority": 50
        },
        "open": {
          "entry": { "type": "web", "path": "/edit?src={source}&session={session}" },
          "modes": ["view", "edit"],
          "fidelity": "full",
          "priority": 50
        },
        "cell.edit": {
          "entry": { "type": "embed", "path": "/cell/markdown.html" },
          "sandbox": { "network": false },
          "priority": 50
        }
      },
      "permissions": ["cyfs:///*:read"]
    }
  ]
}
```

字段语义：

| 字段 | 说明 |
|---|---|
| `handler_id` | App 内唯一，稳定；与 `app_id` 组合成全局 `HandlerRef = <app_id>#<handler_id>` |
| `version` | Handler 自身版本，结果缓存键的一部分（对应 Preview PRD 的 `pipelineVersion`） |
| `selectors` | §4.2，任一命中即匹配 |
| `intents` | 意图 → 该意图的调用契约参数；一个 Handler 可同时提供多个意图，共享选择器 |
| `entry` | 调用入口，§8；`icon` 没有入口 |
| `priority` | 0–100，开发者自评；第三方条目上限 80，80–100 保留给系统内建与用户显式指定 |
| `permissions` | 该 Handler 需要的内容访问范围，必须是 AppDoc `permissions[]` 的子集 |

### 6.3 `system/content_registry` 投影结构

```ts
interface ContentRegistry {
  schema_version: 1;
  updated_at: number;
  handlers: Record<HandlerKey, ContentHandlerEntry>;   // HandlerKey = `${app_instance_id}#${handler_id}`
}

interface ContentHandlerEntry {
  provider: "system" | "app";
  app_instance_id?: string;          // `<app_id>@<owner_user_id>`，system 条目为空
  app_doc_object_id?: string;        // 固定版本，用于确定性 Plan 与缓存键
  app_version?: string;
  handler_id: string;
  handler_version: number;
  selectors: ContentSelector[];
  intents: Record<IntentId, IntentBinding>;   // 与 AppDoc 声明一致，附加系统校验后的派生字段
  permissions: string[];
  enabled: boolean;                  // 用户或管理员可禁用，禁用不删除
  registered_at: number;
}
```

投影时系统校验：`entry` 引用的 endpoint / web host 在 AppDoc 中存在；`permissions` 是 AppDoc 权限子集；第三方 `priority ≤ 80`；`mime: */*` 或仅 `ext` 的选择器只允许绑定 `open` 与 `icon`，不允许绑定 `preview` / `cell.*`（防止劫持所有内容的预览）。

### 6.4 用户偏好 `users/{user}/content_defaults`

```json
{
  "schema_version": 1,
  "defaults": {
    "open":    { "mime:image/*": "photos.did.web@alice#viewer", "mime:text/markdown": "mdapp.did.web@alice#editor" },
    "preview": { "mime:application/pdf": "system#pdf-iframe" }
  },
  "disabled": ["thirdparty.did.web@alice#everything"]
}
```

- key 是紧凑选择器字符串（§4.2），value 是 `HandlerKey`；
- 用户通过“打开方式 → 始终使用此应用”写入；
- 解析时若 value 已不存在或被禁用，忽略该项并按普通排序回落。

---

## 7. 解析（Resolve）

### 7.1 接口

```ts
interface ResolveRequest {
  content: ContentDescriptor | ContentRef;   // 传 ContentRef 时由系统先归一化
  intent: IntentId;
  host?: {
    formFactor?: "desktop" | "mobile" | "tablet";
    runtime?: { acceptOutputs?: string[] };  // Preview Component 当前可消费的结果类型
    embed?: boolean;                          // 宿主是否能承载 embed 类 Handler
  };
  caller: { userId: string; appInstanceId?: string };
  limit?: number;
}

interface HandlerPlan {
  handlerKey: HandlerKey;
  handlerRef: HandlerRef;            // `<app_id>#<handler_id>`
  appDocObjectId?: string;           // 固定版本
  handlerVersion: number;
  intent: IntentId;
  binding: IntentBinding;            // entry / outputs / features / sandbox ...
  matched: { selector: ContentSelector; specificity: number };
  reason: "user-default" | "system-default" | "ranked";
}

interface ContentRegistryClient {
  resolve(req: ResolveRequest): Promise<HandlerPlan[]>;      // 按排序返回候选，[0] 为默认
  listHandlers(filter?: { intent?: IntentId; appInstanceId?: string }): Promise<ContentHandlerEntry[]>;
  setDefault(intent: IntentId, selector: string, handlerKey: HandlerKey | null): Promise<void>;
  setEnabled(handlerKey: HandlerKey, enabled: boolean): Promise<void>;
  onChanged(cb: () => void): () => void;     // 注册表或偏好变化，宿主据此刷新菜单/图标
}
```

### 7.2 算法

```mermaid
flowchart TD
  A[ContentRef] --> B[归一化为 ContentDescriptor<br/>objType / mime(sniff) / schema]
  B --> C[按 intent 过滤注册表条目]
  C --> D[按 selector 匹配，计算特异性]
  D --> E[按 host 约束过滤<br/>formFactor / acceptOutputs / embed]
  E --> F[按 caller 可见性过滤<br/>app availability + enabled]
  F --> G{用户偏好命中?}
  G -->|是| H[置顶 user-default]
  G -->|否| I[排序: 特异性 > priority > provider=system > 注册时间]
  H --> J[HandlerPlan 列表]
  I --> J
```

排序细则：

1. 用户偏好（`content_defaults`）命中且条目可用 → 第一位，`reason = user-default`；
2. 特异性高者优先（§4.3）；
3. 同特异性按 `priority` 降序；
4. 仍相同时 `provider: system` 优先，然后按 `registered_at` 早者优先（结果稳定，不随安装顺序抖动）；
5. `preview` 意图额外规则（Preview PRD §8.1）：Runtime 可直接渲染的类型，内建 `renderer` 条目胜过任何 `converter`；converter 之间再按 `quality.fidelity` → `latency` → `cost` 比较。

解析结果是 **确定性的**：同一注册表版本、同一偏好、同一 Descriptor 得到同一顺序。宿主不得自行指定“用哪个 pipeline”，只能提供 host 约束（Preview PRD §9.5）。

### 7.3 支持级别的推导

Preview PRD §8 的支持级别直接由解析结果推导，不再各自维护表格：

| 条件 | 级别 |
|---|---|
| `resolve(preview)` 为空且 `resolve(open)` 为空 | Level 0 Unsupported |
| `resolve(preview)` 非空 | Level 1 Preview Supported |
| `resolve(open)` 中存在 `fidelity: "full"` 的条目 | Level 2 Full App Supported |

---

## 8. 调用模型（Invoke）

系统通过三种方式调用 App，`entry.type` 决定使用哪种。

| `entry.type` | 方式 | 适用意图 | 说明 |
|---|---|---|---|
| `web` | **launch**：拉起 App 页面 | `open`、`create` | `path` 相对 App 的 Web host（`https://$appid.$zonehost/`），支持占位符 `{source}` `{session}` `{mode}`；桌面内 App 用 `buckyos://app/<app_id>?...` 深链接等价表达 |
| `embed` | **embed**：宿主在沙箱 iframe 中加载 | `preview`（renderer）、`cell.view`、`cell.edit` | `path` 相对 App Web host；宿主与组件只通过 `postMessage` 协议通信（§9） |
| `rpc` | **invoke**：调用 App Service 端点 | `preview`（converter）、`convert`、`enrich`、`enumerate` | `endpoint` 是 AppDoc `service_endpoints` 的 key，`method` 是 kRPC 方法名；长任务走 task-manager，可取消 |

通用调用契约：

- **鉴权**：系统为每次调用签发短期、最小范围的读取 Capability（Preview PRD §9.3、§16），随请求传递；不传长期凭证；Handler 只能访问 `permissions` 声明范围内的内容。
- **版本固定**：调用携带 `HandlerPlan.appDocObjectId + handlerVersion`，App 升级期间旧任务按旧版本完成或失败重试，不混用。
- **错误分类**：统一为 `unsupported | no-handler | corrupted | permission-denied | timeout | handler-error | resource-exceeded | incompatible-output | cancelled`，宿主 UI 只展示分类，不展示内部 Handler 名称。
- **取消**：宿主切换内容时必须能取消仅有自己一个消费者的调用；共享 `workKey` 的任务不因单个宿主退出而取消。

---

## 9. 各意图的调用契约

### 9.1 `icon`（声明式）

```ts
interface IconBinding {
  icons: Record<string, string>;   // size(px) -> App 包内相对路径，SVG 优先
  badge?: string;                  // 可选角标，用于“由 X 应用提供”
}
```

- File Browser 图标视图按 `resolve(icon)` 取第一位；没有任何条目时使用系统按 `mime` 大类的默认图标；
- 图标由系统在安装时复制到图标缓存，运行时不访问 App 服务；
- 缩略图不属于 `icon`，走 `preview` 且 `purpose: thumbnail`。

### 9.2 `preview`

对应 Preview PRD 第 9 章。注册字段与 PRD §9.5 的映射：

| PRD §9.5 要求 | 本文字段 |
|---|---|
| `pipelineId` / 可读名称 / `pipelineVersion` / 实现身份 | `handler_id` / AppDoc `presentation.title` / `handler_version` / `app_doc_object_id` |
| 可识别的输入格式、MIME、对象类型、内容特征 | `selectors[]` |
| 可输出的标准结果类型 | `intents.preview.outputs[]` |
| 成本、延迟、资源、保真等级 | `intents.preview.quality` |
| 流式、分页、缩略图、局部解码 | `intents.preview.features[]`：`streaming | paged | thumbnail | partial | progressive` |
| 沙箱与权限需求 | `intents.preview.sandbox`、`permissions` |
| 版本和优先级 | `handler_version`、`priority` |

```ts
interface PreviewBinding {
  kind: "converter" | "renderer";
  entry: RpcEntry | EmbedEntry;      // converter 用 rpc，renderer 用 embed
  outputs: string[];                 // converter：产出的标准结果类型（PRD §9.2 结果族的 MIME 或 "preview-manifest+json"）
  features?: string[];
  quality?: { fidelity: "low" | "medium" | "high"; latency: "fast" | "normal" | "slow"; cost: "low" | "medium" | "high" };
  sandbox?: { network: boolean; fs: "input-only" | "none"; gpu?: boolean };
  maxSteps?: number;                 // 参与管线拼接时允许的深度，默认 1
  priority: number;
}
```

- **converter** 的调用输入 / 输出即 PRD §9.3 / §9.4 的 Pipeline 请求 / 结果；`purpose`、目标 Profile、页码、时间段等作为参数传入，进入 `canonicalTransformParams`；
- **renderer** 是嵌入式渲染器：当 Runtime 不能直接展示但 App 能在沙箱中自行渲染源内容（例如 3D 模型查看器），宿主按 §9.4 的嵌入协议加载它，`capabilities` 由组件上报；
- Preview Planner 基于 `resolve(preview)` 的候选列表做管线拼接（PRD §9.6）：若单个 converter 的 `outputs` 与 Runtime 不兼容，可再次 `resolve` 中间类型，深度受 `maxSteps` 与系统上限约束，须做环路检测；
- 缓存键 `workKey` 中的 `pipelinePlanDigest` 由每一步的 `HandlerKey + handler_version + app_doc_object_id` 构成。

### 9.3 `open`

```ts
interface OpenBinding {
  entry: WebEntry;
  modes: Array<"view" | "edit">;
  fidelity: "full" | "partial";      // "full" 使内容进入 Level 2
  multiSource?: boolean;             // 是否接受一次打开多个 Source
  priority: number;
}

interface OpenRequest {               // 展开到 entry.path 的占位符，或以 buckyos:// 深链接传递
  source: ContentRef;
  session?: PreviewSessionContext;    // 与 Preview PRD §12 同构，App 可忽略
  mode?: "view" | "edit";
  origin?: { appInstanceId?: string; windowId?: string };
}
```

- 桌面 / 窗口管理器负责窗口复用与数量策略（Preview PRD §13），`open` Handler 只负责接管内容；
- “打开方式…”菜单 = `resolve(open)` 全部候选 + 系统 Preview App（Preview App 自身是 `provider: system` 的 `open` 条目，选择器 `mime:*/*`，`fidelity: partial`，兜底）；
- 双击 = 候选列表第一位；用户勾选“始终使用”即调用 `setDefault("open", selector, handlerKey)`；
- App 收到 `open` 后必须在自身权限内重新读取内容，不能假设系统已替它读好；无法处理时返回可见错误并允许用户回到 Preview。

### 9.4 `cell.view` / `cell.edit`

面向 AI Canvas 表格单元格、结果组中的自定义块，以及未来 IM / Mail 中的卡片。内容通常是结构化值，选择器以 `schema` 为主。

```ts
interface CellBinding {
  entry: EmbedEntry;
  sandbox?: { network: boolean };     // 默认 iframe sandbox="allow-scripts"，不启用 allow-same-origin
  sizing?: { minHeight?: number; maxHeight?: number; resizable?: boolean };
  priority: number;
}
```

**宿主 ↔ 组件协议**（`postMessage`，是 Canvas PRD §11.11 `CanvasWidgetSDK` 的一般化）：

```ts
// 宿主 -> 组件
type CellHostMessage =
  | { type: "cell.init"; value: unknown; content: ContentDescriptor; mode: "view" | "edit"; revision: string; theme: "light" | "dark"; locale: string; readonly: boolean }
  | { type: "cell.update"; value: unknown; revision: string }        // 上游数据变化（Canvas binding stale → 刷新）
  | { type: "cell.resize"; width: number; height: number }
  | { type: "cell.commit-request" }                                   // 宿主请求组件提交当前编辑
  | { type: "cell.destroy" };

// 组件 -> 宿主
type CellWidgetMessage =
  | { type: "cell.ready"; capabilities: string[] }                    // 例如 ["edit","resize","copy"]
  | { type: "cell.change"; value: unknown; baseRevision: string }     // 仅 edit 模式；宿主校验 schema 后写入并生成新 revision
  | { type: "cell.request-resize"; height: number }
  | { type: "cell.request-open"; source?: ContentRef }                // 请求以 open 意图升级到完整 App
  | { type: "cell.error"; code: string; message: string };
```

规则：

- 组件 **只能通过消息** 与宿主交互，不继承宿主 Cookie / 存储，不能直接 fetch；需要内容读取时由宿主代为解析并把值随 `cell.init` 传入，或传入短期 Capability；
- `cell.change` 的值必须通过宿主的 Schema 校验才写入（Canvas PRD §14 “不要让输出绕过 Schema 写入状态”）；`baseRevision` 不匹配时宿主拒绝并回发 `cell.update`；
- Canvas 内部标准块（`text` / `table` / `chart` / `metric`）是 `provider: system` 的 `cell.*` 条目，与第三方组件同表；Agent 输出 `interactive` 块时，其 `schema` 决定由哪个 Handler 渲染；
- 组件崩溃或超时只影响该单元格，宿主显示错误占位并提供“用完整应用打开”（走 `cell.request-open` → `open`）；
- 用户可在设置中禁用某个 `cell.*` Handler（`setEnabled`），禁用后该类型回落到只读 JSON 视图。

---

## 10. 与现有系统的接触点

| 模块 | 变更 |
|---|---|
| AppDoc v1 schema（`doc/App 安装协议.md` §2.1） | 新增可选根字段 `content_handlers`（本文 §6.2），进入 ObjectId；PIKG canonical 输出必须显式序列化 |
| Installer / Scheduler | 安装、升级、卸载时 CAS 更新 `system/content_registry`；执行 §6.3 的校验；投影失败 fail closed 但不阻断 App 安装（该 App 的 Handler 全部不生效并记录诊断） |
| SystemConfig / RBAC | 新增 key `system/content_registry`（仅 Scheduler 可写，用户可读）、`users/{user}/content_defaults`（用户可读写）；RBAC policy 追加对应 `obj://config/...` 规则 |
| buckyos-api | 新增 `ContentRegistryClient`（§7.1），Rust 与 Web SDK 同接口；Web 端可缓存注册表并订阅变更 |
| Preview Component / Preview App | Preview Planner 改为基于 `resolve(preview)`；“使用其他应用打开”改为 `resolve(open)`；错误态“安装支持组件”指向市场中声明了对应选择器的 App |
| File Browser | 图标视图用 `resolve(icon)`；双击 / 右键“打开方式”用 `resolve(open)`；缩略图用 `preview` + `thumbnail`；Meta 面板未来用 `enrich` |
| AI Canvas | 单元格 / 自定义块渲染改为 `resolve(cell.view | cell.edit)`；`CanvasWidgetSDK` 收敛为 §9.4 协议 |
| 桌面 Shell（`src/frame/desktop`） | `buckyos://app/<app_id>?...` 深链接处理 `open` 的 launch；窗口策略仍由 Shell 决定 |

---

## 11. 安全

1. 匹配以 ContentDescriptor 为准：`ext` 只提升排序；`preview` / `cell.*` 不能绑定 `*/*` 选择器；
2. 第三方 `priority ≤ 80`，无法压过系统内建条目，除非用户显式设置默认；
3. 每次调用使用短期最小权限 Capability，Handler 不得获得超出其 AppDoc `permissions` 的内容访问能力；
4. `embed` 类 Handler 一律在沙箱 iframe 中运行，默认无网络、无同源、无顶层导航、无下载；
5. `rpc` 类 Handler 的转换任务在受控沙箱中运行，限制 FS / 网络 / GPU / 进程；
6. 缓存按 `permissionScopeKey` 隔离（Preview PRD §9.7）；
7. 修改用户默认打开方式必须来自用户交互，App 不能替用户 `setDefault`；
8. 注册表变更（新 App 接管某类型）应在通知中心提示用户，用户可一键禁用。

---

## 12. 示例

### 12.1 PSD 转换器（仅 Preview）

```json
{
  "handler_id": "psd",
  "version": 1,
  "selectors": [{ "objType": "cyfile", "mime": ["image/vnd.adobe.photoshop"], "ext": [".psd"], "sniff": { "magic": "38425053" } }],
  "intents": {
    "icon": { "icons": { "64": "icons/psd.svg" } },
    "preview": {
      "kind": "converter",
      "entry": { "type": "rpc", "endpoint": "api", "method": "preview.convert" },
      "outputs": ["image/webp", "image/png"],
      "features": ["thumbnail", "progressive"],
      "quality": { "fidelity": "medium", "latency": "normal", "cost": "medium" },
      "sandbox": { "network": false, "fs": "input-only" },
      "priority": 60
    }
  },
  "permissions": ["cyfs:///*:read"]
}
```

### 12.2 图表组件（Canvas 单元格 + 独立打开）

```json
{
  "handler_id": "chart",
  "version": 3,
  "selectors": [{ "schema": ["https://buckyos.ai/schema/canvas-chart-v*"] }],
  "intents": {
    "cell.view": { "entry": { "type": "embed", "path": "/cell/chart.html" }, "priority": 70 },
    "cell.edit": { "entry": { "type": "embed", "path": "/cell/chart.html?mode=edit" }, "sizing": { "minHeight": 240, "resizable": true }, "priority": 70 },
    "open": { "entry": { "type": "web", "path": "/chart?src={source}" }, "modes": ["view", "edit"], "fidelity": "full", "priority": 70 }
  },
  "permissions": []
}
```

### 12.3 解析调用

```ts
const plans = await registry.resolve({
  content: { kind: "cyfs-path", path: "cyfs:///home/alice/design/cover.psd" },
  intent: "preview",
  host: { runtime: { acceptOutputs: ["image/png", "image/webp", "text/html;profile=safe"] } },
  caller: { userId: "alice", appInstanceId: "files@alice" },
});
// plans[0].handlerRef === "psdtools.did.web#psd"
```

---

## 13. 分期

| 阶段 | 内容 |
|---|---|
| P0 | AppDoc `content_handlers` 字段；`system/content_registry` 投影；`icon` / `preview`(converter) / `open`；File Browser 与 Preview Component 接入；用户默认打开方式 |
| P1 | `cell.view` / `cell.edit` 与 AI Canvas 接入；`preview` renderer；管线拼接 |
| P2 | `enrich`、`create`、`enumerate`、`convert`；私有意图；市场按选择器推荐“可支持此格式的 App” |

## 14. 待定问题

1. `schema` 选择器的版本族匹配（`-v*`）是否够用，还是需要 semver 范围；
2. 跨用户可见性：owner 为 Zone Owner 的 App 安装的 Handler 是否默认对其他用户可用，是否复用 app availability policy；
3. `enumerate` 与 DFS / ZIP 容器抽象的边界，是否应由 NDN 层直接提供；
4. Web SDK 是否在客户端本地缓存整张注册表（体积可控，但需处理变更订阅），还是每次经由 kRPC 解析。
