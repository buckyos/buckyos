# hello-controls：一个控件包从写到复用的完整样子

> 这是 [扩展实现思路](<../BuckyOS AI Workspace 扩展实现思路.md>) §6.10 的配套例子，2026-10-30 建立。目的只有一个：让人在 P3 全部做完之前，就能看到**开发者最终面对的“界面”**——先干什么、再干什么，每一步在 Desktop / CLI / 文件里看到的是什么，典型源码长什么样。
>
> 它**不需要跑起来**。但分两种成分：标 **[P0 可用]** 的文件是按今天冻结的契约写的，`hello-kpi` 与 `hello-card` 的 `component.json` 已用 websdk `aiws_ext.ts` 的 `checkComponent` 预检通过，`aiws-manifest.json` 里的 `digest` 是用同一模块对这些文件算出的真值，真要用可以直接 `buckyos aiws build`；标 **[P1] [P2] [P3]** 的文件是**提案**（`hello-board`、模板），文件里带 `_comment` 说明，形态在相应阶段的任务单里冻结，可能改。权威契约始终是 aiworkspace README “Extension controls” 与 `src/frame/aiworkspace/schemas/`。

## 0. 场景

- Alice（`did:bns:alice`）有一个工作区 **Team Tasks**，里面有一张表 `tasks`（字段 `title` 文本、`status` 单选 todo / doing / done、`hours` 数字、`owner` 文本）。她想要三个东西：一张计数卡、一张任务卡、一个把任务卡摆成一列的面板。
- Bob 在另一个 Zone，想在自己的工作区用 Alice 的控件；他们还想把这套东西变成“一键建一个团队任务板”的模板。
- 两条实现路线都出现：`hello-kpi`（declarative render）、`hello-card`（HTML render）、`hello-board`（composite，Block-control）。

## 1. 开发者的时间线

每一步写三样：**做什么**、**看到什么**（Desktop / CLI / 文件）、**所处阶段**。

### 第 1 步 新建控件 [P0 可用]

Desktop：打开 Team Tasks → 主菜单 **插入 → 扩展控件 → 工作区控件…** → **新建控件…** → 填名字 `hello-kpi`、选 `declarative`、作者 DID（默认 `did:bns:alice`，可改）→ 确定。Desktop 做了三件事：在 home 建骨架、立即 build、在画布放一个测试实例。

home 里多了一个普通目录（任何编辑器、IDE、Agent 都能改）：

```
/home/alice/AI Workspace/Team Tasks/controls/hello-kpi/
├── component.json      接口与文件入口（本例见 controls/hello-kpi/component.json）
├── declarative.json    实现：几条 item
├── AGENT.md            给 Agent 看的自述
└── .aiws-build.json    客户端私有：上次 build 的 workspace_id / def_entity_id / def_rev / dir_rev
```

CLI 等价（Agent 或开发机，不经 Desktop）：

```bash
buckyos pikg init ./hello --owner did:bns:alice --kind aiws-controls --source ./aiws   # 带一个样例控件的包骨架
buckyos aiws build ./hello/aiws/controls/kpi --workspace $WS                            # 本地目录：文件内联上传
```

### 第 2 步 写源码 [P0 可用]

`component.json` 是“接口”，一眼看完（完整文件在 `controls/hello-kpi/`）：

```json
{ "manifest_version": 1,
  "ident": { "author_did": "did:bns:alice", "name": "hello-kpi" },
  "kind": "declarative", "title": "Hello KPI",
  "config_schema": { "type": "object", "properties": { "label": { "type": "string", "default": "Open tasks" } } },
  "bindings": { "tasks": { "accepts": ["buckyos.table-source"], "required": true } },
  "entry": "declarative.json", "agent": "AGENT.md", "deps": [] }
```

`declarative.json` 是“实现”，不执行任何代码：

```json
{ "layout": "stack", "items": [
  { "kind": "metric", "label": "Tasks", "bind": "tasks.count" },
  { "kind": "value",  "label": "Hours", "bind": "tasks.sum:hours" },
  { "kind": "bar",    "label": "By status", "bind": "tasks.status" } ] }
```

`bind` 的 `tasks.` 前缀就是 `bindings` 里声明的名字（Q18）。

### 第 3 步 build [P0 可用]

三种触发：Desktop 控件详情里的 **构建** 按钮、**保存即构建**（watch）开关，或 CLI：

```bash
buckyos aiws build "/home/alice/AI Workspace/Team Tasks/controls/hello-kpi" --workspace $WS
# → { ok: true, status: "accepted", def_entity_id: "e_…", ident: "did:bns:alice#hello-kpi", version: 2, build: { input_digest: "sha256:…", builder_version: "aiws-ext-build/1" } }
buckyos aiws build … --dry-run   # 只要诊断
```

成功 = 一次 Commit（origin `build`，消息 `build did:bns:alice#hello-kpi`）：定义实体、源码快照 `buckyos.dir`、`derived.build` 一起落下，画布上所有引用它的实例立刻按新实现渲染。失败 = 什么都不提交，Desktop 诊断面板 / CLI 输出：

```json
{ "code": "INVALID_OPERATION", "sub_code": "BUILD_FAILED", "data": { "diagnostics": [
  { "file": "declarative.json", "line": 3, "col": 41, "severity": "error", "message": "expected `,` or `]`" } ] } }
```

改了接口且有实例在用时，诊断会列出受影响实例（`instances`），整次 build 失败——例如把 `tasks` 绑定改名或加必填属性。

### 第 4 步 插入实例 [P0 可用]

插入目录 → **扩展** 页签 → Hello KPI → 右侧出现由 `config_schema` 与 `bindings` 生成的初始化表单：`Tasks`（必填，选一张表）、`Label`、`Accent`。确定后画布多一个 Cell，数据长这样（只存覆盖，没改的属性不写）：

```json
{ "type": "buckyos.cell", "view": { "type": "declarative", "version": 1 },
  "def_ref": { "entity_id": "e_hello_kpi" },
  "bindings": { "tasks": { "entity_id": "e_tasks_table" } },
  "config": { "label": "Sprint 12" } }
```

Agent 用 `doc.commit` 写同样的 Cell；手册 `aiworkspace_template_dev.md` 的 “Controls” 一节就是这段。

### 第 5 步 改、再 build、多人 [P0 可用]

Alice 把 `metric` 的 label 改成 "Open"，保存，watch 自动 build，所有实例更新；实例的 `config` 覆盖、绑定、位置都不动。

Bob（同工作区协作者，或跨 Zone）想改这个控件：

```bash
buckyos aiws pull e_hello_kpi --workspace $WS --out ./hello-kpi      # 拿到 Workspace 里的快照 + .aiws-build.json 基线
# 改文件
buckyos aiws build ./hello-kpi --workspace $WS                        # 基线是最新的 → accepted
```

如果 Alice 在他 pull 之后又 build 过一次，Bob 得到 `REVISION_CONFLICT / BUILD_BASE_STALE`，提示先 pull 再 build（git 式，不静默覆盖）。

### 第 6 步 HTML 控件与事件 [P0 可用；事件 P2]

`hello-card` 用 HTML 画一张任务卡（`controls/hello-card/`）。`component.json` 的差别只有 `kind: "html"`、`host_api: 2`、`entry: { html, css, js }`、`assets: ["check.svg"]`。`behavior.js` 只跟宿主对象 `aiws` 说话：

```js
aiws.input('task').props().then(render)                 // 读绑定
aiws.watch('task', () => aiws.input('task').props().then(render))   // 变更跟随
done.onclick = () => aiws.record('task').set({ status: 'done' })    // 写经宿主：权限、锁、撤销都在宿主
aiws.ready()
```

**[P2]** 事件：`component.json` 加 `events: { "completed": { "title": "…", "params": { … } } }`，`behavior.js` 里 `aiws.emit('completed', {...})`；实例侧多一个 `on: { "completed": { "run": "<wish 或 command>" } }`。设计态点击选中 / 移动，运行态点击才触发；一次点击只执行一次，rebuild 不重放（§6.7）。这些键今天都不存在，形态随 P2 冻结。

### 第 7 步 组合控件 [P1]

Alice 想把若干 `hello-card` 摆成一列并带个标题框。两种起点：画布上选中几张卡 → **定义为控件…**，Desktop 在 home 生成 `controls/hello-board/`；或者直接手写。`component.json` 多三样：`kind: "composite"`、`entry: "composition.json"`、`deps: ["did:bns:alice#hello-card"]`。`composition.json`（提案形态，见 `controls/hello-board/composition.json`）只说三件事：

```json
{ "layout": { "algorithm": "stack", "direction": "column", "spacing": "{{config.gap}}" },
  "slots":  { "header": { "max": 1 }, "cards": { "accepts": ["cell:did:bns:alice#hello-card", "card"] } },
  "chrome": { "background": "{{config.background}}", "title": { "text": "{{config.title}}" } } }
```

卡片仍是 BlockTree 里的真实 Block（可选中、批注、连线、Agent 寻址），面板只负责排布与外框（R23）。build 同一条链路；改 `composition.json` 后既有面板实例重排，用户内容不重建。内核为此要先拆校验层（Q15）、放开 `kind`、加定义依赖边（Q16）。

### 第 8 步 导出 [P0 可用；Desktop 生成 .pikg 待 Q20]

工作区控件列表里多选 `hello-kpi`、`hello-card` → **导出控件…**：

- **设为对我所有工作区可用**：服务直接把 bundle 记进 Alice 的本机库（`ext.install { from }`），其他工作区的“从本机库添加…”立刻能看到。
- **把 bundle 写到我的 home**：得到 `/home/alice/AI Workspace/exports/hello-controls/aiws/`，里面是 `aiws-manifest.json` + `controls/<name>/<文件>`（本目录就是这个形态），随后显示一条命令。

CLI 一步到位：

```bash
buckyos aiws ext-export e_hello_kpi,e_hello_card --workspace $WS --out ./pack \
  --name hello-controls --version 0.1.0 --owner did:bns:alice --pikg --install
# ./pack/hello-controls-0.1.0.pikg（AppDoc app_type: aiws，唯一子包 pkg_list.aiws = 这个 bundle）；--install 同时进自己的库
```

导出的是 Workspace 快照，不是 home 里的文件；home 有没 build 的改动时应提示（P0 遗留 L3）。签名与发布在拿到 `.pikg` 之后另做（PIKG 信任体系）。

### 第 9 步 别人复用 [P0 可用]

Bob 在自己的 Zone：

```bash
buckyos app install ./hello-controls-0.1.0.pikg     # 走 Installer → aiworkspace ext.install；不建 AppSpec、不起服务，app list 里看不到
buckyos aiws library list                           # pkg hello-controls 0.1.0：did:bns:alice#hello-kpi, did:bns:alice#hello-card
```

然后在他的工作区 Desktop：**从本机库添加…** → 勾两个控件 → 一次 Commit 进 `data/extensions`（`provenance` 记包名 / 版本 / 摘要）→ 插入目录出现 Hello KPI / Hello Card，照第 4 步插入。他没有 Alice 的源码目录；想改就 `pull` 到自己 home，改完 build 进自己的工作区（身份仍是 `did:bns:alice#hello-kpi`，`provenance` 保留）。

跨 Zone 把库里的控件引进别人的工作区：客户端从自己 Zone `ext.library_bundle` 取字节，向目标工作区 `ext.build`（内联 + `provenance`）。

### 第 10 步 升级 [P2]

Alice 发 0.2.0。Bob `app install` 新包后库里是新版本；他的工作区还是旧定义（D4）。工作区控件详情出现 **有新版本 0.2.0 → 升级…**：列出本地改动（如果他 build 过）、受影响实例、接口变化（收窄会要求迁移），确认后一次 Commit 替换定义与快照，实例覆盖与内容保留。今天同身份异来源只会报 `EXT_IDENT_EXISTS`。

### 第 11 步 模板 [P3]

Alice 把“一张 KPI + 一个面板 + 一张空 tasks 表”保存为模板：`aiws-manifest.json` 的 `workspace_templates` 多一项（提案见 `templates/team-board.template.json`），和控件一起进 `.pikg`。Bob 装包后，新建工作区对话框多出 **从模板创建 → Team task board**，填 `team_name`，得到一个独立工作区：定义先按闭包引入，再由一次内核级子图导入写结构、实例与初始数据（Q17 重映射）。安装模板包本身不建工作区；再建一次得到另一个独立工作区。

### 第 12 步 Agent 看到的 [P0 可用；组合结果 P3]

Agent 没有 Desktop，只有手册和 CLI：`aiws library list` 找到控件 → `describe` 读 `AGENT.md` → `ext-import` 进工作区 → `doc.commit` 写 Cell（第 4 步的 JSON）。要改控件：`pull` 到本地目录 → 改 → `build <本地目录>`（文件内联，不需要能访问 nfs-server）。许愿格的 HTML 结果今天就是一个 `html` 定义（作者 `did:aiws:<workspace_id>`），`pull` 会合成出 `component.json` 让它走同一条路。

## 2. P3 做完后的“界面”总览

| 入口 | Desktop | CLI（`buckyos aiws` / `pikg` / `app`） | Agent |
| --- | --- | --- | --- |
| 发现 | 插入目录 **扩展** 页签（工作区已有）；**从本机库添加…**（本机库） | `library list` | 同 CLI + `describe` 读自述 |
| 新建 / 编辑 | **新建控件…** 建骨架；**编辑控件源码…** 打开 home 目录（Text Editor / File Browser） | `pikg init --kind aiws-controls`；任意编辑器 | 直接写本地目录 |
| 生效 | **构建** / **保存即构建**；诊断面板 | `build [--watch \| --dry-run \| --force]` | `build <本地目录>` |
| 协作 | **拉取**；基线冲突提示 | `pull` → 改 → `build` | 同 CLI |
| 实例 | 初始化表单（属性 + 命名绑定 + 事件绑定 [P2]）；Inspector 改覆盖 | `doc.commit` 写 Cell | 同 CLI |
| 组合 [P1] | **定义为控件…**；composite 宿主、slots、Chrome | 手写 `composition.json` + `build` | 同 CLI |
| 导出 | **导出控件…** → 全局可用 / bundle / `.pikg`（Q20） | `ext-export [--pikg] [--install]` | 同 CLI |
| 分发 | — | `pikg sign` / 发布（PIKG 体系） | — |
| 安装 | 拖 `.pikg` 到桌面（Q21） | `app install x.pikg` / `library install` | 同 CLI |
| 升级 [P2] | 详情 **升级…** | `ext-upgrade`（动词待定） | 同 CLI |
| 模板 [P3] | **从模板创建工作区** | `aiws create --template` （动词待定） | 同 CLI |

## 3. 文件清单

| 文件 | 阶段 | 说明 |
| --- | --- | --- |
| `controls/hello-kpi/{component.json, declarative.json, AGENT.md}` | P0 可用 | declarative render 控件；一个必填命名绑定、两个属性 |
| `controls/hello-card/{component.json, view.html, style.css, behavior.js, check.svg, AGENT.md}` | P0 可用（`behavior.js` 末尾注释里的 `aiws.emit` 是 P2） | HTML render 控件；读记录、写经宿主、`assets` 一个 |
| `aiws-manifest.json` | P0 可用 | 导出 bundle / `.pikg` 子包的清单；`digest` 为真值，`version` 是假定的 build 修订 |
| `controls/hello-board/{component.json, composition.json, AGENT.md}` | P1 提案（事件键 P2） | composite 控件：`manifest_version: 2`、`kind: composite`、`deps`、`events` 今天都会被 build 拒绝 |
| `templates/team-board.template.json` | P3 提案 | `workspace_templates` 的一项；今天清单里该数组非空即拒绝 |

与 P0 冻结契约的对应关系、以及 P1–P3 各提案要在哪个任务单冻结，见设计文档 §6.10 与 §8。
