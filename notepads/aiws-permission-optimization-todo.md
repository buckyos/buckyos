# AIWS 权限优化 TODO

状态：待立项。2026-11-12 从 [核心 API 调整 TODO](aiws-core-api-unification-todo.md) 的 review 裁决 R4 拆出；条目是汇总，尚未经 owner 逐项过目，未实施。

日期：2026-11-12

## 0. 范围

把 AIWS 里与权限有关的待办集中到一处：文档里已经列出的缺口、核心 API 调整带出的梳理项，以及 AI Native PRD 提出的权限问题。核心 API 调整单沿用现有 ACL 实现，不等本单；本单负责系统梳理和补洞。

权限模型本身以 [权限、资源与保留](../doc/workspace/kernel/11-权限资源与保留.md) 为准：一套 ACL，按实体与子树授权，不提供行列级 ACL，不引入第二个权限引擎。本单不改这些原则。

## 1. 现状

依据源码静态核对，没有运行测试。

| 范围 | 当前实现 |
| --- | --- |
| 调用者种类 | 账号（受限 / 不受限）、Agent、访客 token、邀请链接、run token、演示相关身份；见 [workspace.rs](../src/frame/aiworkspace/store/src/workspace.rs) 的 `Caller` |
| 实体级可见范围 | owner 私有、presenter 范围、participant 及其 `readers`，加上从祖先继承的子树授权；见 [access.rs](../src/frame/aiworkspace/core/src/access.rs) 的 `sees_scope` 与 `caps` |
| 子树授权 | 后端支持按任意实体子树授权（`ws.grant` 的 `scope_entity_id`）；权限面板只能选整个工作区或某张画布 |
| 用户组 | BuckyOS 用户组只能以 `group:<id>` 直接写进授权条目；没有带稳定组 ID 的 Workspace 权限组 |
| run token | 方法白名单在 [server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) 的 `RUN_TOKEN_METHODS`：`proc.*` 的一部分，以及 `ws.get_info`、`doc.read`、`doc.query`、`doc.resolve`、`doc.outline`、`doc.list_children` |
| 访客 | 只能到达 `public` 授权和访客入口；`ws.publish` 让 `public` 主体可读某个实体 |
| 对象入口动作 | 没有任何 Provider 声明写动作；审批门在 2026-10-29 定为“等第一个写动作出现时再定”（[对象 URL TODO](done/agent-object-url-todo.md)） |
| 阶段工具的路径 | `path_of` 拼路径时不检查祖先是否可读，会打印出祖先的标题 |

## 2. 条目

### A. 文档里已经列出的缺口

来源：[内核文档 README](../doc/workspace/kernel/README.md) 的“权限”一节，原文写明“需要专门做 TODO，同时改底层和 UI”。

- [ ] **A-1** 针对单个 Block / 数据源配置权限：后端已支持，权限面板补上任意实体子树的选择。
- [ ] **A-2** Workspace 权限组：稳定组 ID，BuckyOS 用户组作为成员来源而不是静态名单，授权条目可以指向组。
- [ ] **A-3** 组映射、成员和授权变更的审计记录；成员缓存的失效机制。

### B. 核心 API 调整带出的梳理

来源：核心 API TODO 的 review（R4）。核心 API 单按“每个结果在查询时过 `can_read`”实现，下面是它不负责的部分。

- [ ] **B-1** 调用者矩阵：§1 的每种调用者对发现（`ls` / `find`）、文本查询、SQL 查询、路径解析和 `body` 读取分别能看到什么，写成表并形成测试矩阵。重点是访客只能覆盖已发布的实体。
- [ ] **B-2** run token 白名单：已有的直读是否按 PRD §6 收紧。核心 API 设计草案保留了 `doc.query`（换成文本和 SQL 之后，运行中的程序可以对所有可读表做 JOIN，比原来的单表读取宽），`doc.find` 不加入；这里要判断是否收回。AI Native PRD §6.5 的建议（2026-11-12）：run token 只留 `proc.*` 和 `ws.get_info`，运行中的读取一律走快照上的阶段工具、限于声明的输入；托管手册在运行内只用 `run-*` 命令，收掉后流程不变。**owner 2026-11-12 裁决：先能力再安全，暂不收紧，白名单保持现状。** 风险（读取超出声明的输入、不进读集、与快照版本不一致、托管位置的程序可能拿到 token）列在 PRD §6.5；本条留作以后是否收紧的入口。
- [ ] **B-3** 泄露检查：数量在过滤后计；`find` 的候选只来自可见范围；错误信息不暴露不可读实体是否存在；路径里不可读的祖先用 ID 段表示。修阶段工具 `path_of` 不检查祖先的问题。同级不允许重名（核心 API TODO R14）之后，`NAME_CONFLICT` 会让调用者得知一个自己看不见的同级实体叫什么，要定这种情况怎么报。
- [ ] **B-4** 搜索索引引入后的可见性维护：规则是索引不保存可见性，查询时过滤；移动、`readers` 变化和授权变化因此不需要改索引。验证撤权后旧游标和缓存不泄露。
- [ ] **B-5** SQL 查询的授权边界：逻辑表范围等于可读表，可读范围内的 JOIN，authorizer 白名单的评审；确认不因 SQL 绕过受控提交表单的读取限制。
- [ ] **B-6** 对象入口写动作的审批门：在 Session 内审批、Provider 侧确认协议和维持现状三个选项里定一个，之后才声明启动许愿格运行、触发同步等动作。owner 2026-11-12 确认晚定，核心 API 首批不声明写动作。

### C. AI Native PRD 提出的问题

- [ ] **C-1** PRD §7.4：使用 owner 凭据获取的数据不自动向全部成员开放。PRD §12 把“是否有实体级可见范围”列为未核对；现有机制见 §1，需要判断它够不够用、默认值是什么。
- [ ] **C-2** PRD §10 的同步程序授权主体（owner 安装授权、可委托触发）与 `run`、`manage` 能力的对应。

## 3. 待 owner 确定

1. A 组要同时改底层和 UI，是否与 B、C 放在同一单里做，还是本单只做梳理和补洞，A 另开。
2. B-6 审批门选哪个选项（已定晚定，声明第一个写动作之前给出）。
3. 各组的先后；B-1、B-3 建议跟着核心 API 首批一起验收。

## 4. 实施回写

- 待实施。
