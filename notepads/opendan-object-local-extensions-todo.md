# OpenDAN 对象别名与本地动作脚本 TODO

设计稿 · 2026-11-20 · BuckyOS 基线 `a72c674fc`，同日按 review 与 owner 裁决修订（§7）。OpenDAN 的对象 `read` 和 `xcall` 已支持按 URL 路由到本地 HTTP adapter。本文在已有路由上增加独立的对象别名，并支持按 Profile 中的动作名直接运行已安装的脚本。个人资源是第一个用例。[既有扩展契约](../doc/opendan/agent-did-object-lib%20需求.md#L711) · [个人资源 TODO](opendan-owner-commands-todo.md)

首个完成标志是在没有个人对象 HTTP 服务的环境中，`read uri=owner` 能看到 `notify` 的用法，`xcall owner notify` 能执行管理员安装的脚本。同时，`owner.cal` 可以链接到另一个已经存在的对象。两条绑定可以独立安装、替换、删除。

## 1. 作者和使用者各做什么

别名的目的是节约 token：模型写 `xcall owner.cal add`，不必写完整的 Object URL，结果中也只回显别名。对象作者用 Profile 声明属性、动作、事件及其数据契约。使用者给对象起本地名字。已知动作时可以直接调用，无须让模型先 `read` 一次。执行路径取决于**完整别名**的绑定。

| 绑定 | `read uri=<别名>` | `xcall <别名> <动作>` |
| --- | --- | --- |
| `local`，指向已安装的本地对象定义 | 展示包提供的对象说明、Profile 和当前可见能力。首版不执行属性脚本。 | 从 Profile 找到动作、校验参数，从本地清单按动作名选定脚本并执行。例如 `owner` 的 `notify`。 |
| `link`，指向明确的 Object URL | 用当前 caller 读取目标的真实 Card、Profile 和可见属性。 | 等价于以同一 caller 直接调用该 URL 上的同名动作。例如 `owner.cal` 链接已发布的日历。 |

`owner` 和 `owner.cal` 是两个键，点号是名字的一部分。可以只安装 `owner.cal`。也可以反过来，将 `owner` 链到一个 `obj://`，让 `owner.cal add` 在本地组合 AIWS API。实现无需遍历 `owner` 的属性、拼接 `/cal` 路径或推断子对象权限。一项绑定只选一条执行路径。目标未声明动作、权限拒绝或超时时，调用在原路径结束。

## 2. 现有接点

**已验证的扩展路径。** 用已安装的 `agent_tool`，通过 `AGENT_DID_OBJECT_ROUTE_CONFIG` 加载一条精确 URL 路由，让它指向临时启动的 `local_http` adapter。对同一 URL 执行 `read uri=<URL> property=title` 和 `xcall <URL> ping` 均成功。前者向 `/adapter/read` 传入 `options.properties=["title"]` 并返回 adapter 提供的正文，后者向 `/adapter/x-call` 传入 `ping` 并返回其结果。采用 DID Object Provider 的 URL 则由现有 DID 适配器解析 Card/Profile、检查动作并按 form 调用。[本地 HTTP 实现与往返测试](../src/frame/agent-did-object-lib/src/adapters/local_http.rs#L504) · [DID Object Provider 测试](../src/frame/agent-did-object-lib/tests/provider_read.rs#L111)

| 层次 | 当前行为及要改的接点 |
| --- | --- |
| 输入与文件 | `ReadTool` 先分内容 ObjId、文件和 URL。CLI 的 `xcall` 会将不含 `://` 的输入展开成 `file://`，而且先按 `=` 识别参数，`?v=1` 的 URL 会被当成参数。别名判定要在裸词转文件之前，完整位置 URL 也要在拆 `key=value` 之前识别。[read 分类](../src/frame/agent_tool/src/read_tool.rs#L556) · [xcall 参数](../src/frame/agent_tool_cli_dev/src/lib.rs#L831) · [路径展开](../src/frame/agent_tool_cli_dev/src/lib.rs#L2523) |
| 对象路由 | 既有设计规定 Gateway 先把 alias 等友好表达式解析成规范 URL，`ObjectRef::parse` 只接收 URL。URL router 按方法、URL 模式和优先级选择 adapter。`AgentObjectAdapter` 已有 read/x_call/subscribe/unsubscribe，读结果的渲染、窗口、去重和 AgentTool 输出都在 runtime 里。`ObjectUrl` 目前只有 file/cyfs/obj/http(s)/buckyos 几种 scheme。[Gateway 职责](../doc/opendan/agent-did-object-lib%20需求.md#L157) · [ObjectRef](../src/frame/agent-did-object-lib/src/types.rs#L13) · [router](../src/frame/agent-did-object-lib/src/router.rs#L55) · [adapter](../src/frame/agent-did-object-lib/src/adapters/mod.rs#L29) · [ObjectUrl](../src/kernel/buckyos-api/src/object_url.rs#L55) |
| 宿主装配 | `read` 是 Loader 进程内的工具，会话和后台任务各自装配内置 reader，远端设备文件还有专用路径。`xcall` 只有 shell 命令 `agent_tool xcall`，在当前 runtime 里执行：可能是 Agent 容器、经 Node Executor 的宿主机，也可能是另一台设备。CLI 从显式路由配置、环境指定配置或 builtin 构造 runtime。[CLI 配置](../src/frame/agent_tool_cli_dev/src/lib.rs#L596) · [CLI read](../src/frame/agent_tool_cli_dev/src/lib.rs#L2389) · [会话 Loader](../src/frame/opendan/src/loader.rs#L343) · [后台任务](../src/frame/opendan/src/tasks.rs#L381) · [设备文件](../src/frame/opendan/src/urls.rs#L18) |
| 身份 | `agent_tool` 有 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 时用该会话。没有时，在 OOD 上以设备私钥换取内核服务会话，此时的身份不是任何人。[运行时登录](../src/frame/agent_tool/src/xllm.rs#L5239) |
| 扩展包 | ext-tools 包的根目录只允许 `ext-manifest.json`、`bin/`、`smoke/`、`store/<id>/`、`online/`。清单是 `deny_unknown_fields`，且必须有 `provides` 或 `online`，只带对象扩展的包现在会被拒。包由各节点的 reconciler 按 Zone 的期望状态安装到 `$BUCKYOS_ROOT/tools`，容器只读挂载同一目录。`EXT_TOOLS_RESERVED_COMMANDS` 与 `STANDARD_TOOLS` 由测试保持一致。[清单与布局校验](../src/kernel/buckyos-api/src/ext_tools.rs#L113) · [布局白名单](../src/kernel/buckyos-api/src/ext_tools.rs#L343) · [发行清单](../doc/app_service/paios容器需求.md#L259) · [保留命令测试](../src/frame/lib_opendan/src/baseline.rs#L798) |
| Deno 先例 | Goal check 已有 `ctx.host.call`：runner 以 MARK 行发调用、按 id 收应答，宿主经 `CheckCalls` 回答，标准 Tool 的 `deno.json` 提供 `buckyos` 导入。它以 `--allow-read --allow-net --allow-env` 启动且继承环境变量。数据源同步的 runner 先 `env_clear`，只按需开 `--allow-net`。[host.call 协议](../src/frame/lib_opendan/src/goal_check_runner.ts) · [CheckCalls](../src/frame/lib_opendan/src/goal_check.rs#L134) · [启动参数](../src/frame/lib_opendan/src/goal_check.rs#L295) · [env_clear 先例](../src/frame/lib_opendan/src/sync.rs#L276) |
| 协议 | Card 描述身份、controller、服务和 Profile 引用。Profile 定义 properties/actions/events 和 schema。没有 form 的动作可以通过 Profile 校验。现有 DID 适配器会检查声明，并按 form 调用。纯本地扩展可直接提供 Profile 与来源信息，在 `read` 中渲染为对象说明。正式发布后才有可对外解析的 Card 与 endpoint。[协议](../../buckyos-base/doc/did-object-protocol.md#L137) · [Card 校验](../../buckyos-base/src/name-lib/src/did_object_card.rs#L253) · [DID 动作调用](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L201) |

## 3. 一份可照着写的扩展

以下拟议样例属于 Alice 的 Zone 管理员安装的扩展包 `personal-tools`。作者提供 `object-extensions.toml`、`profiles/person.json`、`scripts/notify.ts` 三个文件，放在包的 `store/personal-tools/` 下。ext-tools 的 `ext-manifest.json` 增加可选字段 `"object_extensions": "store/personal-tools/object-extensions.toml"`，让安装器能直接找到它。带这个字段的包可以不提供命令。清单内的相对路径以该 toml 所在目录为基准。对象 ID 以本包 id 开头。样例的 `fixture.test.buckyos.io` 是集成测试中提供日历 Profile 的 Agent 对象 host，可替换为实际获得的 Object URL。

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
      "x-buckyos:action": {"effect": "write", "idempotency": "recommended"}
    }
  }
}
```

`scripts/notify.ts` 的调用约定如下。固定的 Deno 入口导入模块并传入 `ActionInvocation` 和受限的宿主接口。示例中的 `host.call` 是本项要交付的接口。宿主将 `text` 转成 `NotifySendReq`，附上会话来源和幂等键，从认证上下文填入本人收件人。调用带了键就用它，没带则由宿主按内容构造（§5）。真实投递走现有通知逻辑。[共享动作类型](../../buckyos-base/src/name-lib/src/object_profile.rs#L452) · [通知请求](../src/kernel/buckyos-api/src/msg_notify.rs#L198) · [ZoneNotify](../src/frame/lib_opendan/src/notify.rs#L158)

```ts
export default async function notify(request, host) {
  const receipt = await host.call("notify.send", { text: request.params.text })
  return { result: { notify_id: receipt.notify_id } }
}
```

```sh
agent_tool read uri=owner
agent_tool xcall owner notify --params '{"text":"评审稿已备好"}'
agent_tool xcall owner.cal list --params '{"from":"2026-11-20T00:00:00Z","to":"2026-11-21T00:00:00Z"}'
```

要改用已发布的主人对象，用户可以单独覆盖名为 `owner` 的记录。绑定内容为 `{name="owner", kind="link", url="obj://fixture.test.buckyos.io/people/alice"}`，由目标 Profile 声明并实现 `notify`。`owner.cal` 继续使用原绑定。新增别的业务对象也只需提供定义或链接，通过注册即可使用，无需修改 Rust 中的名字或动作列表。

## 4. 注册、解析和发现

1. **谁可以登记。** 首版只从已安装的扩展工具包登记，包由管理员安装。包默认别名随包启用。用户可在自己的作用域显式覆盖某个完整键，选已安装的对象 ID、固定 URL 或停用。现有 CLI 的 `--config` 及环境变量仍用于 URL 路由，不能注册可执行脚本或增加宿主调用权限。skill 提供发现与示例，安装 skill 不触发代码安装。[skill 包约定](../src/frame/lib_opendan/src/skills.rs#L743)
2. **何时生效。** 两种来源分开存放和计算修订。一次调用固定同一份快照与覆盖。

   | 来源 | 存放 | 修订 `alias_revision` | 校验与失败 |
   | --- | --- | --- | --- |
   | 包默认 | 节点 reconciler 在包安装/启用、升级、禁用、卸载时，在 `$BUCKYOS_ROOT/tools/` 下生成只读快照，记来源包、版本、内容摘要和每条有效绑定。容器经只读挂载看到同一份。 | 由包 id、版本和该绑定的内容摘要计算，同版本的包在各节点上相同。 | 按包校验，拒绝重复对象 ID、同条记录同时指向两种目标、清单引用丢失、未知 schema 版本、脚本/模块越出 `store/<id>/`（包括符号链接）、Profile 动作与脚本不一一对应、宣称可订阅本地事件。失败或与别的包重名的包，其别名不生效，在该节点的 ext-tools 状态里报告，其余包照常切换。卸载使该包的别名失效。 |
   | 用户覆盖 | system-config `users/<user_id>/object_aliases/<name>`，内容为上述绑定或显式停用标记。RBAC 照 `content_defaults`：用户读写自己的，app/agent 只读、禁止写，control-panel 读写。人类入口建议为 `buckyos obj alias list/set/rm`。[RBAC 先例](../src/kernel/buckyos-api/src/rbac_config.rs#L99) | 值内的修订号，写入带 `if_revision`，每次设置、停用、删除都推进，A → B → A 产生三次可区分的状态。 | 高于包默认。删除覆盖恢复包默认。目标对象 ID 被卸载时报告失效，不悄悄落回包默认。没有安装所选扩展的 runtime 报不可用。 |
3. **如何定位。** 宿主先取本 runtime 的快照和当前用户的覆盖。身份来自受验证会话。显式 URL、带分隔符的文件路径、`./owner.cal`、内容 ObjId 等沿原规则处理。其余裸词按完整名字精确匹配。名字使用小写字母起首、后接字母数字或下划线，可由点号分段。裸词同时命中别名和当前目录下已有的同名路径时报歧义，提示文件写 `./<name>`，避免 `package.json` 一类文件被别名悄悄接管。只有显式声明的 `reserved_roots` 对同根拼错的名字给出 `UNKNOWN_ALIAS`，例如 `owner.caal`。包只能把自己绑定的根声明为保留根。普通未登记的 `notes.md` 仍按文件读取。
4. **如何派发。** 共用 resolver 把别名改写成规范 URL：`link` 得到目标 URL，`local` 得到内部 URL `ext-object://<对象 ID>`，例如 `ext-object://personal-tools/person`。之后统一走 `ObjectRef::parse`、URL router 和 adapter。本地对象由新的 `local_script` adapter 处理，经内置 scheme 路由命中，读结果的渲染、窗口、去重、AgentTool 输出和错误映射全部沿用。`ObjectUrl` 增加该 scheme，并登记到 [key url.md](../doc/key%20url.md#L29)。直接输入 `ext-object://` 也只能命中当前快照中的对象。内部 URL 是宿主的实现细节：`cmd_args`、标题、摘要和 `detail.object` 等模型可见的回显保留别名。首版 `link` 接受完整的对象 URL，沿已生效的对象路由访问。只有能提供对象能力声明的目标才可通过别名执行动作，普通网页和文件按自身能力处理。`local.object` 只找已注册的对象 ID，因此没有递归展开别名的步骤。直接传 URL 也不会被同名扩展拦截。
5. **读出什么。** `read uri=owner` 展示对象名、来源、Profile 版本、可见属性/动作/事件与参数。纯本地来源标记为 `local`。`link` 别名给出一次目标的规范 URL，供保存 Hint；`xcall` 结果不重复它。元数据可附 `alias_revision`，不泄露安装路径和凭据。沿用现有文本窗口、`property`、去重与输出格式。`alias_revision` 计入读结果的 `version`，换绑后不会被判成未变化。普通发现只读取声明及已授权的只读摘要。对外写 Hint 时保存经过验证的已发布 URL 及其来源 Zone。纯本地别名只在原作用域有意义。[read 窗口与去重](../src/frame/agent_tool/src/read_tool.rs#L393) · [系统 URL 约定](../doc/key%20url.md#L17)

## 5. 脚本调用、权限和失败

- **能力与契约。** 本地对象用 `ObjectProfile` 建立声明。激活时 Profile 的每个动作都必须在包清单 `objects.actions.<name>` 中有脚本，脚本也只能对应已声明的动作。首版不执行属性脚本，以后开放时登记在 `objects.properties.<name>`，宿主只授予读取能力。激活时校验 schema 支持范围，执行前校验输入和协议要求的确认信息，执行后验证输出。当前 `validate_for_profile` 只查动作名称，需另做数据校验。副作用按 `x-buckyos:action.effect` 判断：`effect` 为 `write`、`destructive`、`external` 的动作，执行器对只读权限模式的 Agent 直接拒绝。现有 DID 适配器读的是协议未定义的 `x-buckyos:risk`，可一并改成读 `effect`。link 在目标处检查 Profile 和实际权限，遇到未声明动作按现有对象协议失败。[Profile/动作校验](../../buckyos-base/src/name-lib/src/object_profile.rs#L477) · [Protocol 调用约定](../../buckyos-base/doc/did-object-protocol.md#L1023) · [effect 字段](../../buckyos-base/doc/did-object-protocol.md#L593)
- **执行位置与身份。** `read` 在 Loader 进程内解析别名，本地对象只渲染 Profile，不跑脚本。`xcall` 由当前 runtime 中的 `agent_tool` 执行脚本，快照、Deno 和身份都取自该 runtime。执行器只接受用户会话或 Agent App（`app:<agent>@<user>`）会话。无会话 token 时 `agent_tool` 在 OOD 上会以设备身份登录，这类服务或设备身份一律报身份不可用。`session_id` 只用于关联会话。
- **runner。** 沿用 Goal check 的 runner 协议与 `CheckCalls` 形态，以及标准 Tool 的 `deno.json`。启动参数按数据源同步的做法收紧：`env_clear`，不给 `--allow-env`、`--allow-net`、`--allow-run`、`--allow-write`，`--allow-read` 只到本包的 `store/<id>/`。限制执行时间和输出大小。业务参数、远端 Card/forms、Memory、cwd 和用户提交的自由文本都只能充当数据。脚本只能经 `host.call` 使用清单为本动作声明的宿主调用。管理员安装包即授予这些调用，调用者能否执行由下游服务按真实 caller 判定。服务凭据留在宿主，底层操作继承原 caller、对象目标与授权。首批宿主调用是 `notify.send`，只能向本人投递。日历脚本的 AIWS 访问也经宿主调用，由宿主以原 caller 会话转发白名单内的 AIWS 方法。
- **回执。** 复用 `ActionInvocation`（method、params、observed、idempotency_key、confirm_token、trace_id）与 `ActionResponse`（result 或 error，可带 meta）。纯本地实例没有已发布身份时可以省略 `obj/obj_did`。宿主校验回执、按既有 AgentTool envelope 输出 `detail.result` 或 `detail.error`，诊断保留 trace。buckyos-base 的 URL 客户端目前在 `invoke_action_from_resolved` 里把三项调用元数据写死为 `None`，要改接口接收它们，使 link 和直接 URL 均能把原键送达 Provider。buckyos-base 先推 main，buckyos 再 `cargo update`。[请求构造](../../buckyos-base/src/name-client/src/did_obj_client.rs#L177) · [回执类型](../../buckyos-base/src/name-lib/src/object_profile.rs#L517) · [AgentTool 输出](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L290)
- **幂等与重试。** 调用带 `--idempotency-key` 时，键原样送达写入端：本地脚本经宿主调用，link 经 Provider。宿主转给系统服务时加 `xcall:<别名>:<动作>:` 前缀，避免和 Agent 自己的通知键相撞。调用没带键时，由宿主调用自己构造：`notify.send` 用通知类型与正文的 hash，加上两小时的时间片（§7 Q1）。msg_center 按 `(requester, key)` 永久去重，不加时间片的话，同一句话以后再也发不出新卡。`notify` 与 `ask` 都经 `notify.send`，规则相同。相同键不同输入报冲突。最终去重由写入端保证，通用层不记录派发。别名在首次调用和重试之间改绑时，重试按新绑定派发，这是已知风险。不能确认结果时返回 `WRITE_OUTCOME_UNKNOWN`，附可见的查询线索。通用层不承诺任意外部脚本恰好执行一次，扩展应声明所支持的重试能力。[通知去重](../src/frame/msg_center/src/notify.rs#L316)
- **事件。** 链接对象继续用原事件适配器及订阅回执。一次订阅固定目标、主体、事件、修订和有效期，重连/取消到原目标，撤权后停止交付。首版纯本地脚本只交付发现和动作。本地事件等有可用的订阅、取消和恢复实现后再开放，在此之前拒绝安装宣称可订阅本地事件的定义。个人用途对象要跟随日历后端变化，可由该对象发布领域事件。[订阅与桥接](../src/frame/agent-did-object-lib/src/runtime.rs#L168)
- **可诊断。** 区分未知别名、别名与文件歧义、身份不可用、无权读取/执行、绑定目标不可用、Profile 无此动作、参数不合约、脚本启动失败或返回无效结果。脚本启动后异常退出或输出非法时，凡写入可能已发生就给出结果未知。失败不会转换成另一个后端的第二次写入。

## 6. CodeAgent 实施与验收

| 步骤 | 改动与通过条件 |
| --- | --- |
| E1 · 别名与 link | 在 `ReadTool` 的分类和 CLI `xcall` 的参数解析之前插入同一个 resolver，读本 runtime 的快照与用户覆盖，沿用原 URL 路由。实现歧义判定与 `reserved_roots`。模型可见的回显保留别名，`alias_revision` 计入读结果 `version`。新增用户覆盖键、RBAC 规则与 `buckyos obj alias`。在真实 Provider 夹具中比较 `read` 别名与 URL，比较 `xcall` 的目标、动作、参数、授权及业务回执。覆盖带 `?v=1` 的 URL、原有 `key=value`、内容 ObjId、`notes.md`、`./owner.cal`、与别名同名的已有文件、设备文件和拼错的保留名。裸 `xcall` 复用现有多入口分派：加入 `STANDARD_TOOLS` 与 `EXT_TOOLS_RESERVED_COMMANDS`，在 `build_aios` 和宿主安装中加入口，核对 Windows 的 `.exe`。 |
| E2 · 本地 Profile 与脚本 | ext-tools 清单增加 `object_extensions`，有它时放宽 `provides` 必填。`ObjectUrl` 增加 `ext-object` 并写入 key url.md。实现 `local_script` adapter、runner、身份检查、`notify.send` 及其键规则，以及 buckyos-base 的调用元数据。用上面三个文件做最小可运行夹具。无个人对象 Provider 时，`read uri=owner` 展示 `notify` 且通知计数为零。同一 caller 的 `xcall owner notify` 调用本地脚本并取得真实通知回执。不带键在同一个两小时时间片内调两次只有一张卡，带两个不同的键得到两张卡。未声明动作、无效参数、多传收件人、只读 Agent 调写动作、设备身份、越权宿主调用均在相应边界被拒。跑脚本异常、读环境变量和回包丢失用例。 |
| E3 · 独立替换与配置生命周期 | 测试「`owner` 本地、`owner.cal` 链接」及其反向组合，再单独安装 `owner.cal`。替换或删除 `owner` 后，日历的目标、修订及权限保持原值。重名或损坏的包不生效且不阻塞其他包切换，损坏的包升级保留原快照。禁用后新调用失败。测两个不同 owner、一个人的两个 Agent、两个节点上的同版本包和重启，确认别名、修订、凭据与结果不会串用。绑定不变时，回包丢失后以原键重试，写入端只有一次写入。 |
| E4 · 运行环境与交接 | 从 CLI、Loader 内的 `read`、经 Node Executor 的宿主 runtime 和后台任务实际读取同一份已激活配置，从人类和 Agent 的正常 xcall 路径各执行一次。未安装扩展的 runtime 报不可用。验证现有 `local_http`/自定义 URL adapter 路由仍命中，skill 安装只分发手册。在扩展包和宿主发布流程中覆盖安装与更新。记录真实 caller、包版本、绑定修订与动作回执。E1–E3 可在夹具上独立交付，owner 的原生日历 M1 消费此机制，用户共享由个人资源 TODO 继续验收。 |

在 `buckyos/src/` 运行 `cargo test -p agent-did-object-lib -p agent_tool_cli_dev -p agent_tool -p libopendan -p buckyos-api -p opendan -- --test-threads=1`，在 buckyos-base 运行 `cargo test -p name-client -p name-lib`。为 `agent_tool` 的 read 分类和 OpenDAN 的装配路径补回归用例。已有本地 HTTP 与 Provider 夹具可以直接复用。发布验收在安装后的宿主和 Agent 镜像运行 §3 的示例。[Provider 测试](../src/frame/agent-did-object-lib/tests/provider_read.rs) · [CLI 本地 HTTP 测试](../src/frame/agent_tool_cli_dev/src/lib.rs#L3134) · [read 测试](../src/frame/agent_tool/src/read_tool.rs#L821) · [镜像入口](../build_aios#L681)

## 7. 裁决与待确认

owner 2026-11-20 已定：

1. 别名的目的是节约调用和结果中的 token。内部 URL 只在宿主内部使用，模型写的和看到的都是别名（§4.4）。
2. 首版不建通用派发账本，改绑后的重试按新绑定派发，记为已知风险（§5）。
3. `notify` 没带幂等键时，按通知内容的 hash 自动构造（§5）。

待 owner 确认：

- **Q1 时间片。** 第 3 条另加了按小时的时间片，因为 msg_center 的去重是永久的，单用内容 hash 时同一句话只能发一次。代价是同一小时内重复发同一句话会被合并，跨整点的重试会多出一张卡。
> 按你的建议，2小时足够了
- **Q2 review 中替 owner 选的取舍。** 首版不执行属性脚本，`read` 本地对象只展示 Profile。去掉“本人登记本地目录”和 Agent/runtime 作用域。宿主调用随管理员安装授予。只读 Agent 在执行器按 `effect` 拒绝。按包校验，坏包不阻塞其他包。别名与已有文件同名报歧义。用户覆盖的人类入口放在 `buckyos obj alias`。
> 按你的建议