# OpenDAN 对象别名与本地动作脚本 TODO

设计稿 · 2026-11-20 · BuckyOS 基线 `a72c674fc`。OpenDAN 的对象 `read` 和 `xcall` 已支持按 URL 路由到本地 HTTP adapter。本文在已有路由上增加独立的对象别名，并支持按 Profile 中的动作名直接运行已安装的脚本。个人资源是第一个用例。[既有扩展契约](../doc/opendan/agent-did-object-lib%20需求.md#L711) · [个人资源 TODO](opendan-owner-commands-todo.md)

首个完成标志是在没有个人对象 HTTP 服务的环境中，`read uri=owner` 能看到 `notify` 的用法，`xcall owner notify` 能执行用户安装的脚本。同时，`owner.cal` 可以链接到另一个已经存在的对象。两条绑定可以独立安装、替换、删除。

## 1. 作者和使用者各做什么

对象作者用 Profile 声明属性、动作、事件及其数据契约。使用者给对象起本地名字。已知动作时可以直接调用，无须让模型先 `read` 一次。执行路径取决于**完整别名**的绑定。

| 绑定 | `read uri=<别名>` | `xcall <别名> <动作>` |
| --- | --- | --- |
| `local`，指向已安装的本地对象定义 | 展示包提供的对象说明、Profile 和当前可见能力。指定 `property` 时执行对应的只读属性实现。 | 从 Profile 找到动作、校验参数，从本地清单按动作名选定脚本并执行。例如 `owner` 的 `notify`。 |
| `link`，指向明确的 Object URL | 用当前 caller 读取目标的真实 Card、Profile 和可见属性。 | 等价于以同一 caller 直接调用该 URL 上的同名动作。例如 `owner.cal` 链接已发布的日历。 |

`owner` 和 `owner.cal` 是两个键，点号是名字的一部分。可以只安装 `owner.cal`。也可以反过来，将 `owner` 链到一个 `obj://`，让 `owner.cal add` 在本地组合 AIWS API。实现无需遍历 `owner` 的属性、拼接 `/cal` 路径或推断子对象权限。一项绑定只选一条执行路径。目标未声明动作、权限拒绝或超时时，调用在原路径结束。

## 2. 现有接点

**已验证的扩展路径。** 用已安装的 `agent_tool`，通过 `AGENT_DID_OBJECT_ROUTE_CONFIG` 加载一条精确 URL 路由，让它指向临时启动的 `local_http` adapter。对同一 URL 执行 `read uri=<URL> property=title` 和 `xcall <URL> ping` 均成功。前者向 `/adapter/read` 传入 `properties=["title"]` 并返回 adapter 提供的正文，后者向 `/adapter/x-call` 传入 `ping` 并返回其结果。这里执行的是宿主和本地 adapter 的协议。采用 DID Object Provider 的 URL 则由现有 DID 适配器解析 Card/Profile、检查动作并按 form 调用。[本地 HTTP 实现与往返测试](../src/frame/agent-did-object-lib/src/adapters/local_http.rs#L504) · [DID Object Provider 测试](../src/frame/agent-did-object-lib/tests/provider_read.rs#L111)

| 层次 | 当前行为及要改的接点 |
| --- | --- |
| 输入与文件 | `ReadTool` 先分内容 ObjId、文件和 URL。CLI 的 `xcall` 会将不含 `://` 的输入展开成 `file://`，而且先按 `=` 识别参数。别名判定要在裸词转文件之前，完整位置 URL 也要在拆 `key=value` 之前识别。[read 分类](../src/frame/agent_tool/src/read_tool.rs#L556) · [xcall 参数](../src/frame/agent_tool_cli_dev/src/lib.rs#L831) · [路径展开](../src/frame/agent_tool_cli_dev/src/lib.rs#L2523) |
| 对象路由 | 既有设计把友好表达式放在 Gateway 层，`ObjectRef::parse` 接收规范 URL。URL router 按方法、URL 模式和优先级选择 adapter。`AgentObjectAdapter` 已有 read/x_call/subscribe/unsubscribe，`local_http` 已支持进程外的读和动作。新增部分是精确别名、本地 Profile 与已装脚本的绑定和执行。[Gateway 职责](../doc/opendan/agent-did-object-lib%20需求.md#L157) · [ObjectRef](../src/frame/agent-did-object-lib/src/types.rs#L13) · [router](../src/frame/agent-did-object-lib/src/router.rs#L55) · [adapter](../src/frame/agent-did-object-lib/src/adapters/mod.rs#L29) · [本地 HTTP 契约](../src/frame/agent-did-object-lib/src/adapters/local_http.rs#L53) |
| 宿主装配 | CLI 从显式路由配置、环境指定配置或 builtin 构造 runtime，CLI 的 read 也装配对象 reader。常驻 OpenDAN 会话和后台任务另行装配内置 reader，远端设备文件还有专用路径。各入口应注入同一套别名判定和同一作用域的已激活配置。[CLI 配置](../src/frame/agent_tool_cli_dev/src/lib.rs#L596) · [CLI read](../src/frame/agent_tool_cli_dev/src/lib.rs#L2389) · [会话 Loader](../src/frame/opendan/src/loader.rs#L343) · [后台任务](../src/frame/opendan/src/tasks.rs#L381) · [设备文件](../src/frame/opendan/src/urls.rs#L24) |
| 协议 | Card 描述身份、controller、服务和 Profile 引用。Profile 定义 properties/actions/events 和 schema。现有 DID 适配器会检查声明，并按 form 调用。纯本地扩展可直接提供 Profile 与来源信息，在 `read` 中渲染为对象说明。正式发布后才有可对外解析的 Card 与 endpoint。[协议](../../buckyos-base/doc/did-object-protocol.md#L137) · [Card 校验](../../buckyos-base/src/name-lib/src/did_object_card.rs#L253) · [DID 动作调用](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L201) |

## 3. 一份可照着写的扩展

以下拟议样例属于 Alice 本地启用的扩展。作者提供 `object-extensions.toml`、`profiles/person.json`、`scripts/notify.ts` 三个文件。建议在 ext-tools 的 `ext-manifest.json` 增加可选字段 `"object_extensions": "object-extensions.toml"`，让安装器能直接找到它。样例的 `fixture.test.buckyos.io` 是集成测试中提供日历 Profile 的 Agent 对象 host，可替换为实际获得的 Object URL。

```toml
version = 1
reserved_roots = ["owner"]

[[objects]]
id = "personal-tools/person"
profile = "profiles/person.json"

[objects.actions.notify]
script = "scripts/notify.ts"
host_calls = ["notify.send"]

[[aliases]]
name = "owner"
kind = "local"
object = "personal-tools/person"

[[aliases]]
name = "owner.cal"
kind = "link"
url = "obj://fixture.test.buckyos.io/calendars/alice"
```

`profiles/person.json` 是普通 ObjectProfile。样例只有一个动作，`read` 展示它的参数和副作用。本地脚本入口只在包清单中登记。以后发布为 DID Object 时，可以给同一契约配上 Provider 的 HTTP forms。

```json
{
  "@context": "https://www.w3.org/2022/wot/td/v1.1",
  "id": "urn:opendan:example:personal-notifications:v1",
  "title": "主人的通知",
  "actions": {
    "notify": {
      "input": {
        "type": "object",
        "properties": {"text": {"type": "string", "minLength": 1}},
        "required": ["text"],
        "additionalProperties": false
      },
      "output": {
        "type": "object",
        "properties": {"notify_id": {"type": "string"}},
        "required": ["notify_id"]
      },
      "x-buckyos:action": {"effect": "write", "idempotency": "required"}
    }
  }
}
```

`scripts/notify.ts` 的调用约定如下。固定的 Deno 入口导入模块并传入 `ActionInvocation` 和受限的宿主接口。示例中的 `host.call` 是本项要交付的接口。宿主将 `text` 转成 `NotifySendReq`、附上会话来源和原幂等键，从认证上下文填入本人收件人。真实投递走现有通知逻辑。[共享动作类型](../../buckyos-base/src/name-lib/src/object_profile.rs#L452) · [通知请求](../src/kernel/buckyos-api/src/msg_notify.rs#L198) · [ZoneNotify](../src/frame/lib_opendan/src/notify.rs#L158)

```ts
export default async function notify(request, host) {
  const receipt = await host.call("notify.send", { text: request.params.text })
  return { result: { notify_id: receipt.notify_id } }
}
```

```sh
agent_tool read uri=owner
agent_tool xcall owner notify --params '{"text":"评审稿已备好"}' --idempotency-key review-1120
agent_tool xcall owner.cal list --params '{"from":"2026-11-20T00:00:00Z","to":"2026-11-21T00:00:00Z"}'
```

要改用已发布的主人对象，用户可以单独覆盖名为 `owner` 的记录。绑定内容为 `{name="owner", kind="link", url="obj://fixture.test.buckyos.io/people/alice"}`，由目标 Profile 声明并实现 `notify`。`owner.cal` 继续使用原绑定。新增别的业务对象也只需提供定义或链接，通过注册即可使用，无需修改 Rust 中的名字或动作列表。

## 4. 注册、解析和发现

1. **谁可以登记。** 扩展通过已安装的扩展工具包提供。也支持有管理权限的本人显式登记自己的本地目录，注册时用同一校验流程复制为带版本的已安装快照。包默认别名随包启用。用户可在自己的作用域显式覆盖某个完整键，选已安装的对象 ID、固定 URL 或停用。现有 CLI 的 `--config` 及环境变量仍用于 URL 路由，不能注册可执行脚本或增加宿主调用权限。skill 提供发现与示例，安装 skill 不触发代码安装。[扩展发行入口](../src/rootfs/bin/ext-tools/README.md) · [skill 包约定](../src/frame/lib_opendan/src/skills.rs#L743)
2. **何时生效。** 在原有包安装/启用、升级、禁用、卸载路径上构造一份只读快照，标识来源包、版本、内容摘要和每个有效绑定的修订 `alias_revision`。同一作用域同一优先级的重名报告冲突。有权设置的显式用户绑定高于包默认。建议在用户配置中用 `users/<user_id>/object_aliases/<name>` 保存单条覆盖，内容为上述绑定或显式停用标记，修改时带 `if_revision`。删除覆盖可恢复包默认。没有安装所选扩展的 runtime 报不可用。拒绝重复对象 ID、同条记录同时指向两种目标、清单引用丢失、未知 schema 版本、脚本/模块越出包根（包括符号链接）及 Profile/处理表不一致。全部通过后原子切换，失败保留上一份。卸载使相关绑定明确失效，用户显式绑定失效不能悄悄落回默认目标。一次调用固定同一个快照。只推进有效目标、实现或启用状态发生变化的那些别名的修订，A → B → A 产生三次可区分的状态。
3. **如何定位。** 收到输入时，宿主先确定有效配置作用域（已激活包、Zone、用户以及适用的 Agent/runtime）。身份来自受验证会话。显式 URL、带分隔符的文件路径、`./owner.cal`、内容 ObjId 等沿原规则处理。其余裸词按完整名字精确匹配。建议名字使用小写字母起首、后接字母数字或下划线，可由点号分段。只有显式声明的 `reserved_roots` 对同根拼错的名字给出 `UNKNOWN_ALIAS`，例如 `owner.caal`。普通未登记的 `notes.md` 仍按文件读取。保留根只用于诊断，可以独立于该根的对象绑定存在。
4. **如何派发。** 共用 Gateway resolver 返回 `URL(规范 URL)` 或 `Local(注册对象 ID、包版本、Profile、绑定修订)`。前者沿现有 URL router，后者交给宿主显式注册的本地对象执行器。两者在同一层返回既有的读结果或动作结果，供 CLI、会话和任务消费。`ObjectRef::parse` 和 URL 路由只接收 URL。首版 `link` 接受完整的对象 URL，沿已生效的对象路由访问。只有能提供对象能力声明的目标才可通过别名执行动作。普通网页和文件按自身能力处理。`local.object` 只找已注册的对象 ID，因此没有递归展开别名的步骤。直接传 URL 也不会被同名扩展拦截。
5. **读出什么。** `read uri=owner` 展示对象名、来源、Profile 版本、可见属性/动作/事件与参数。纯本地来源标记为 `local`。元数据可附 `alias_revision`、已发布的规范 URL（如有），避免泄露安装路径和凭据。沿用现有文本窗口、`property`、去重与输出格式，同一运行内别名换绑或 caller 可见性改变后重新取值。普通发现只读取声明及已授权的只读摘要。对外写 Hint 时保存经过验证的已发布 URL 及其来源 Zone。纯本地别名只在原作用域有意义。[read 窗口与去重](../src/frame/agent_tool/src/read_tool.rs#L393) · [系统 URL 约定](../doc/key%20url.md#L17)

## 5. 脚本调用、权限和失败

- **能力与契约。** 本地对象用 `ObjectProfile` 建立声明。只有在包清单绑定实现且 caller 获准时，属性或动作才可用。属性实现以相同方式登记在 `objects.properties.<name>`，宿主只授予读取能力。动作实现独立登记在 `objects.actions.<name>`。激活时校验 Profile 与脚本的对应、schema 支持范围，执行前校验输入和协议要求的确认/幂等信息，执行后验证输出。当前 `validate_for_profile` 只查动作名称，需另做数据校验。配置快照把每个声明的动作绑定到固定入口，外部 Profile 中的 form 只由其协议适配器解释。link 在目标处检查 Profile 和实际权限，遇到未声明动作按现有对象协议失败。[Profile/动作校验](../../buckyos-base/src/name-lib/src/object_profile.rs#L477) · [Protocol 调用约定](../../buckyos-base/doc/did-object-protocol.md#L1023)
- **执行与身份。** 由固定 runner 按已安装快照选脚本，以结构化消息传递请求。Deno 使用打包的本地代码/依赖，限制执行时间和输出大小。业务参数、远端 Card/forms、Memory、cwd 和用户提交的自由文本都只能充当数据。脚本只可使用包为本动作申请、管理员或用户已授予、且真实 caller 当前有权使用的宿主调用。服务凭据留在宿主，底层操作继承原 caller、对象目标与授权。`session_id` 只用于关联会话。比如 `notify` 脚本只能请求宿主按现有规则向本人投递，日历脚本可获得只读或可写的 AIWS transport。可复用既有 Deno 子进程的结构化调用、超时和输出限制。[Deno 执行先例](../src/frame/lib_opendan/src/sync.rs#L276) · [运行时定位](../src/frame/lib_opendan/src/goal_check.rs#L196)
- **回执。** 复用 `ActionInvocation`（method、params、observed、idempotency_key、confirm_token、trace_id）与 `ActionResponse`（result 或 error，可带 meta）。纯本地实例没有已发布身份时可以省略 `obj/obj_did`。宿主校验回执、按既有 AgentTool envelope 输出 `detail.result` 或 `detail.error`，诊断保留 trace。URL 客户端目前将三项调用元数据设为 `None`，需接通传递，使 link 和直接 URL 均能把原键送达 Provider。[请求构造](../../buckyos-base/src/name-client/src/did_obj_client.rs#L177) · [回执类型](../../buckyos-base/src/name-lib/src/object_profile.rs#L517) · [AgentTool 输出](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L290)
- **重试与换绑。** 对可写且支持重试的别名动作，首次派发前原子记录作用域、实际 caller、别名、动作、请求键、输入摘要及选定的对象/实现版本。相同键不同输入报冲突。超时或重启后先复核当前授权，只向原目标以原键查证或安全重试。不能确认就返回 `WRITE_OUTCOME_UNKNOWN`，附可见的查询线索。新请求按新修订执行。别名与直接 URL 命中同一 Provider 时透传相同业务键，最终去重仍由实际写入端保证。通用层不承诺任意外部脚本恰好执行一次，扩展应声明所支持的重试能力。
- **事件。** 链接对象继续用原事件适配器及订阅回执。一次订阅固定目标、主体、事件、修订和有效期，重连/取消到原目标，撤权后停止交付。首版纯本地脚本交付发现、只读属性和动作。本地事件等有可用的订阅、取消和恢复实现后再开放，在此之前拒绝安装宣称可订阅本地事件的定义。个人用途对象要跟随日历后端变化，可由该对象发布领域事件。[订阅与桥接](../src/frame/agent-did-object-lib/src/runtime.rs#L161)
- **可诊断。** 区分未知别名、无权读取/执行、绑定目标不可用、Profile 无此动作、参数不合约、脚本启动失败或返回无效结果。脚本启动后异常退出或输出非法时，凡写入可能已发生就给出结果未知。失败不会转换成另一个后端的第二次写入。

## 6. CodeAgent 实施与验收

| 步骤 | 改动与通过条件 |
| --- | --- |
| E1 · 别名与 link | 在 CLI、会话与任务共享的 Gateway 增加已安装定义加载、精确查找及分类，沿用原 URL 路由。在真实 Provider 夹具中比较 `read` 别名与 URL，比较 `xcall` 的目标、动作、参数、授权及业务回执。覆盖带 `?v=1` 的 URL、原有 `key=value`、内容 ObjId、`notes.md`、`./owner.cal`、设备文件和拼错的保留名。裸 `xcall` 复用现有 `agent_tool xcall` 的多入口分派，补宿主与镜像的 PATH 安装，核对 Windows 的 `.exe`。 |
| E2 · 本地 Profile 与脚本 | 用上面三个文件做最小可运行夹具。无个人对象 Provider 时，`read uri=owner` 展示 `notify` 且通知计数为零。同一 caller 的 `xcall owner notify` 调用本地脚本并取得真实通知回执。再用一个只读属性夹具验证 `property` 和窗口。未声明动作、无效参数、错误收件人、只读 caller、越权宿主调用均在相应边界被拒。跑脚本异常和回包丢失用例。 |
| E3 · 独立替换与配置生命周期 | 测试「`owner` 本地、`owner.cal` 链接」及其反向组合，再单独安装 `owner.cal`。替换或删除 `owner` 后，日历的目标、修订及权限保持原值。重名、损坏的包升级保留原快照。禁用后新调用失败。测两个不同 owner、一个人的两个 Agent 和重启，确认别名、凭据与结果不会串用。在旧脚本写入后丢包，换成 link、重启、用原键重试，新 Provider 无新增。 |
| E4 · 运行环境与交接 | 从 CLI、常驻 UI/Work 和后台任务实际读取同一份已激活配置，从人类和 Agent 的正常 xcall 路径各执行一次。验证现有 `local_http`/自定义 URL adapter 路由仍命中，skill 安装只分发手册。在扩展包和宿主发布流程中覆盖安装与更新。记录真实 caller、包版本、绑定修订与动作回执。E1–E3 可在夹具上独立交付，owner 的原生日历 M1 消费此机制，用户共享由个人资源 TODO 继续验收。 |

在 `buckyos/src/` 先运行 `cargo test -p agent-did-object-lib -p agent_tool_cli_dev`，为 `agent_tool` 的 read 分类和 OpenDAN 的装配路径补回归用例。已有本地 HTTP 与 Provider 夹具可以直接复用。发布验收在安装后的宿主和 Agent 镜像运行 §3 的示例。[Provider 测试](../src/frame/agent-did-object-lib/tests/provider_read.rs) · [CLI 本地 HTTP 测试](../src/frame/agent_tool_cli_dev/src/lib.rs#L3134) · [read 测试](../src/frame/agent_tool/src/read_tool.rs#L821) · [镜像入口](../build_aios#L681)
