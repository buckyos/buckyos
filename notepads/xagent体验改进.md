# xAgent 体验改进

状态：问题收集，待 review。记录日期：2026-10-04。

这类体验问题先在本文收集，review 确认交互、范围和验收标准后再修改实现。下列 TODO 均未实施，候选方案不代表已确定设计。

## 从本地 Agent Session 目录启动

用户习惯：在本地找到 Agent Session 目录，把目录作为参数传给 CLI；或者进入该目录，直接执行 `xagent`。

这里的 Session 目录指包含 `.opendan_agent_session/` 的外层目录。

### 当前情况

| 调用方式 | 当前行为 |
|---|---|
| `xagent run /path/to/session` | 支持显式目录参数 |
| `xagent /path/to/session` | 不支持，路径被当作未知子命令 |
| 在 Session 目录执行 `xagent` | 打印帮助，退出码 2，不运行当前 Session |
| 在 Session 目录执行 `xagent run` | 报缺少目录或 SID 参数 |
| 在 Session 目录执行 `xagent run .` | 目录识别后，`SessionDir::open` 直接通过 `file_name()` 获取 SID；`.` 得到空 SID，不能正确定位 Session |

当前临时用法是进入 Session 目录后执行 `xagent run "$PWD"`。运行仍要求能定位 Agent State、Session 已登记且登记位置一致，以及调用身份符合 driver 要求。传目录会从配置读取 Agent DID，但不会据此自动找到 Agent 根目录。

### 待 review TODO

- [ ] **当前目录默认入口**：确认在有效 Session 目录中，裸 `xagent` 是否等价于运行当前 Session；`xagent run` 省略目录时是否也使用当前目录。明确当前目录不是 Session 时的提示与退出码，以及默认推进到哪个停止点。
- [ ] **显式目录快捷入口**：确认是否增加 `xagent <session_dir>`，或采用显式选项传目录；明确与已有子命令、SID 参数的解析关系，并保留清晰的帮助示例。
- [ ] **目录解析与 SID 一致性**：修复 `.` 得到空 SID 的问题。review 路径规范化与 SID 读取方案，覆盖绝对路径、相对路径、`.`、`..` 和符号链接；明确目录名称与配置内 `session_id` 的一致性校验规则。
- [ ] **从目录启动时的环境定位体验**：梳理 Agent State／Agent 根目录的定位方式及 driver 身份要求。review 是否能够自动发现所需环境，不能发现时应给出哪些配置指引；不因目录快捷入口而自动绕过登记、位置或身份校验。
- [ ] **review 后补齐验证与说明**：为确认后的交互补充 CLI 验收用例，覆盖正常目录、非 Session 目录、缺失 Agent State、登记位置不一致和 driver 不匹配；同步帮助文本与使用文档。本轮仅记录，不修改实现或新增测试。

### 代码依据

- [CLI 入口与目录定位](../src/frame/lib_opendan/src/bin/xagent.rs)：`real_main`、`cmd_run`、`locate`。
- [Session 目录读取](../src/frame/lib_opendan/src/session/mod.rs)：`SessionDir::open`。
- [Agent State 定位](../src/frame/lib_opendan/src/state/connect.rs)：`connect`。
- [运行前登记与位置检查](../src/frame/lib_opendan/src/runner/drive.rs)：`drive_locked`。
