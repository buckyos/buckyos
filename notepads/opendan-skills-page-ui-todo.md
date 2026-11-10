# OpenDAN Skills 页界面形态 TODO

状态：2026-11-10 立项，未排期，未实施。来源是 [Agent UI 升级 TODO](agent-ui-upgrade-todo.md) 的裁决 D19（owner：skills 内部页面的修改做个独立 todo，那份单子不做）。下面的内容是从那份文档的 §1.4、§4.4 移过来的；任务和对照表还没有经过 owner 逐条裁决，要先过 §4 的待裁决。

范围：opendan WebUI 的 Skills 页与 [BuckyOS Agent UI 升级规划](<BuckyOS Agent UI 升级规划.md>) §3 在界面形态上的差距。Skills 的机制（分组、禁用、安装、导入、草稿、使用记录）已由 [Skills 体系改进计划](opendan-skills-system-todo.md)（下称 skills TODO）交付，不在这里。

读法：路径相对 `buckyos/`。现状按提交 `27aea9efb`（2026-11-10）核对，以符号名为准，实施前重读。

## 1. 现状

| 事实 | 位置 |
| --- | --- |
| 三类：`SkillKind::Work`、`Plan`、`Goal`。work skill 带 `scope`（`work` / `ui` / `ui,work`），规划里 UI 组与 Work 组的分界就是它 | `src/frame/lib_opendan/src/protocol/behavior.rs` |
| 首页 `SkillsCard` 显示“N 可用 / M 禁用”并链到 `#/skills` | `src/frame/opendan/web/src/pages/Skills.tsx` |
| `#/skills` 在观测页的标签布局里（和 Sessions、Agent State、Loader 并列），不是首页的手机样式；文案走 i18n | `src/frame/opendan/web/src/App.tsx` |
| 页面按 work / plan / goal 三张表列出，每张七列：名称与说明、scope、来源、大小、使用次数、所选 runtime 上的可用性、操作。work 表里 `scope` 含 `ui` 的排前。停用的行置灰，留在原表里 | `Skills.tsx` 的 `SkillRow` |
| 操作：禁用（可填原因）或启用，任何来源都行；卸载只对 `installed` / `imported` | 同上 |
| 表格下面依次是 UI 集合、从 `.pikg` 安装（浏览器内解析，先出安装计划与风险，owner 确认后才装）、草稿、文本导入（起一个 `skill_import` Work 产出草稿）、审计 | `Skills.tsx`、`src/frame/opendan/web/src/pikg.ts` |
| 可用性来自 `skills.status {runtime_id?}`，缺省私有 runtime，进入页面取一次、不轮询，可以换 runtime、可以重新检查。只覆盖 front matter 写了 `commands:` 的手册；`ui_core`、`buckyos_admin`、`share`、`xllm` 四份内置手册没写，页面上显示“手册没有声明命令” | `src/frame/opendan/src/home.rs` 的 `skills_status` |
| 读正文只有 `behaviors.cmd_skill {name}`（work 手册），页面没有用它。plan / goal skill 没有读正文的 kRPC | `src/frame/lib_opendan/src/state/krpc.rs` |
| mock e2e 六条，断言依赖现在的表格结构 | `src/frame/opendan/web/tests/e2e/skills.spec.ts` |
| 没有在真实 Zone 上验证过 | skills TODO §0 |

## 2. 规划的要求对照现状

| 要求（规划 §3，Agent UI TODO 的 D6、D7） | 现状 | 差距 |
| --- | --- | --- |
| 四组与计数：首页按 UI / Work / Plan / Goal 显示各组已启用的数量 | 类型与 `scope` 已有；页面按 work / plan / goal 三张表列；首页只有总数 | 计数不分组 |
| 停用与卸载：任何来源都能停用，只有装上去的能卸载；停用数另计；每组末尾有折叠的“已停用”小节，在原处重新启用 | 停用、启用、卸载都能做，规则一致。停用项留在原表里置灰 | 没有折叠小节 |
| 安装器：一个对话框，Package、URL、粘贴文本三个入口共用一套 LLM 扫描，owner 确认后安装 | 两条路径。`.pikg`：出确定性的安装计划与风险项，确认后安装。文本：`skill_import` Work（LLM）改写成草稿，owner 在草稿面板看过再装。URL 只有 CLI（`buckyos skill install`）支持 | 入口没有合成一个对话框；WebUI 没有 URL；包这条路径不过 LLM |
| 详情：安装与就绪分开显示，命令是否可用以 Provider Runtime 为准，逐条列出 | 列表里按所选 runtime 显示可用性、缺的命令和能补上它的扩展包 | 没有详情页，看不到正文；plan / goal 手册没有读正文的接口；四份内置手册没写 `commands:` |
| 手机优先（规划的 Agent 首页整体是手机样式） | 宽表格，在观测页布局里 | 手机宽度下要横向滚动 |

规划里“Skill 与命令可独立安装”“扫描可拆解的 CMD Skills”要按后来的裁决读：skill 是纯文本，不带命令也不带安装脚本，命令来自扩展包和 runtime 的基线（runtime TODO 裁决 28）。页面只显示手册依赖的命令是否可用、哪个扩展包能补上。

## 3. 任务

从 Agent UI TODO 原来的 SK1 拆出。都还没有裁决，先看 §4。

- [ ] **SP1 首页卡片分组计数。** 按四组显示已启用的数量：UI（`scope` 含 `ui` 的 work skill）、Work（其余 work skill）、Plan、Goal；停用数另写“· N 已停用”。数据来自现有的 `skills.list`，不加后端。
- [ ] **SP2 列表页改成手机优先。** `#/skills` 搬进首页样式的外壳（Agent UI TODO 的 HM1 提供），七列宽表改成按组的列表行：名称、说明、来源、是否就绪。每组末尾是折叠的“已停用”小节，在原处重新启用。UI 集合、从 `.pikg` 安装、草稿、文本导入、审计保留，收到页面下部或二级入口。观测页的标签栏去掉 Skills。就绪状态的取法不变：进入页面取一次，不轮询，保留“重新检查”和 runtime 选择。
- [ ] **SP3 详情页与读正文接口。** `#/skill/<kind>/<name>`：手册正文和 `references/` 列表；依赖命令逐条显示可用、缺失、由谁提供，缺失时显示能补上它的扩展包；适用基线与平台；来源与安装记录；使用次数；禁用、启用、卸载的按钮。没写 `commands:` 的手册显示“手册没有声明命令”，不要显示成“就绪”。后端加一个读 plan / goal 手册正文的方法（名称待定，例如 `skills.read {kind, name}`，返回 `{text, references}`，归入只读方法）；work 手册沿用 `behaviors.cmd_skill`。新增 kRPC 按 `AGENTS.md` 表里 skill kRPC 那一行核对手册。
- [ ] **SP4 测试。** 重写 `tests/e2e/skills.spec.ts` 里依赖表格结构的断言；mock 与类型同步。

## 4. 待裁决

1. **补哪几条。** 现有页面功能齐全，差的都是形态。SP1–SP3 可以各自取舍。
2. **`scope` 是 `ui, work` 的手册怎么计数。** 算进 UI 组、Work 组，还是两组都算（今天只有 `share` 一份是这样）。
3. **安装器要不要合并。** 是否合成一个对话框、给 WebUI 加 URL 入口、让包这条路径也过 LLM 扫描。这三件动的是安装流程，不只是界面，没有排任务。
4. **四份内置手册补 `commands:`。** 属于内置手册的维护，补上后详情才能显示它们的命令可用性。没有排任务。
5. **与 HM1 的先后。** SP2、SP3 要用 Agent UI TODO 的 HM1（首页样式的详情外壳）。HM1 没做之前，是等它，还是在这里先做一个外壳。

## 5. 验证

- 在 `src/frame/opendan/web/`：`pnpm check`、`pnpm lint`、`pnpm test:e2e`。动了后端再跑 `cargo test -p opendan -- --test-threads=1`（在 `src/` 下）。
- 规划的验收场景“安装了 Skill，Provider Runtime 缺少依赖命令”：安装 `test/skills_pikg_sample/` 改出的、`commands` 里写了一条不存在命令的包，列表里显示已安装，详情指出缺的是哪条命令。现有页面的列表已经能标出缺的命令（mock e2e 有覆盖），真实 Zone 上没验过。
- 容器里的 OpenDAN 要 `./build_aios --local-test` 后重建容器才有改动。
