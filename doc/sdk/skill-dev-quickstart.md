# skill 开发快速上手

给 skill 作者的最短路径：**写手册 → 从目录装到自己的 Agent 上试用 → 打包 PIKG → 发布 → 别人安装。** 机制（冻结、选择、scope、草稿）见 [OpenDAN Agent 开发指南](OpenDAN_Agent_Dev_Guide.md) §2.4；PIKG 命令规范见 [PIKG 文档总览](../pikg/README.md)。可照抄的最小工程在 [test/skills_pikg_sample/](../../test/skills_pikg_sample/README.md)。

`buckyos skill` 的子命令以 `buckyos command list` / `buckyos command describe skill <verb>` 的输出为准。

## 1. 四类 skill 与目录

一个 skill 包的源目录（下文叫 `skills/`）：

```text
skills/
  cmd_skills/<name>/SKILL.md          work skill（scope 含 ui 时也是 ui skill）
  cmd_skills/<name>/references/*.md   可选：按需读的细节
  plan_skills/<name>/SKILL.md         plan skill
  goal_skills/<name>/SKILL.md         goal skill
```

- work skill：一类事的命令手册。Work Session 创建时由创建者点名，正文整篇进入它的系统提示。
- ui skill：`scope: ui` 或 `scope: ui, work` 的 work skill，给对话用。每个对话都带着它的全文，要短。
- plan skill：拆解、排序、验收一类目标的方法。`plan_simple` 在建 TodoList 前按需读。不写命令。
- goal skill：Goal 自己记下的方法。

`<name>` 只用小写字母、数字、`_`、`-`，与目录名一致。直接放在 `cmd_skills/` 下的单个 `.md` 文件不是 skill。包里出现 `tools/` 目录或不是 UTF-8 文本的文件会被拒绝（§3）。

## 2. 写 SKILL.md

```markdown
---
name: hello
description: Print a greeting with the `hello` command. Use it to check that an installed skill's commands reach a work session.
requires: nothing beyond this skill's own command (tools/hello/bin/hello)
scope: work
baseline: standard
---
# hello

Run `hello [name]`. Success: it prints `hello, <name>` and exits 0.
```

| 字段 | 取值 | 说明 |
|---|---|---|
| `name` | 目录名 | 可省略；写了必须与目录一致 |
| `description` | 一两句话 | 选择方只看这一行：同时写“做什么”和“何时用”，用请求里会出现的词 |
| `requires` | 自由文本 | 给人读：账号、权限、服务、工具 |
| `commands` | 逗号分隔的命令名，如 `commands: ffmpeg, ffprobe` | 正文运行的外部命令。shell 内建与常见 Unix 命令、标准层命令（`agent-session`、`buckyos`、`gen_image` 等，清单见 `xagent runtime baseline`）不写。只用来告诉 owner 某台 runtime 缺什么，不拦安装和使用 |
| `scope` | `work`（缺省）/ `ui` / `ui, work` | 只有 work skill 能写 |
| `baseline` | `standard`（缺省）/ `extended` | 用到标准层之外的命令（`ffmpeg`、`pandoc`、`python3`、`node` 等）写 `extended` |
| `platforms` | `linux`、`macos`、`windows` | 逗号分隔；到处能用就不写 |

plan / goal skill 只写 `name`、`description`、`requires`；出现 `scope` 或 `commands` 是错误。

正文的写法：

- 写成捷径而不是规定：这样做更快更稳，哪里容易错，怎么确认做成了。
- 命令放在 shell 代码块里；不是 shell 的代码块标上语言（`json`、`toml`、`text`），否则会被当成命令扫描。
- 不复制会过期的接口说明，指向 `--help`、`buckyos command describe` 或源码。
- 长表格、逐条命令细节、少用的流程放 `references/<topic>.md`，正文写明何时读：`read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/<topic>.md`。
- 大小：ui skill 的 SKILL.md 不超过 16 KB（每个对话都带着它，越短越好），plan skill 不超过 3 KB；work skill 只留每次都用得上的内容。
- plan skill 不出现具体命令、创建 Session 的命令或具体的 work skill 名，写“这一步交给带 X 类手册的 Todo”。

## 3. 手册依赖的命令

skill 是纯文本：包里只有 `SKILL.md` 和 `references/`，不带可执行文件，也不带安装脚本，安装就是校验加复制。这是分发的安全边界：装一个 skill 不会在任何 runtime 上运行或放置任何东西。

手册依赖的命令只有三个来源：

- 标准层命令（`agent-session`、`buckyos`、`gen_image` 等，清单见 `xagent runtime baseline`），每台 runtime 都有，不用写。
- 扩展包提供的命令（`ffmpeg`、`pandoc`、`python3` 等）。扩展包由 Zone 管理员在 Desktop “设置 > 扩展工具”或 `buckyos ext-tools` 里启用，装到节点的 `$BUCKYOS_ROOT/tools`，全平台同一组命令。手册在 `commands` 里写命令名，用到标准层之外的命令时声明 `baseline: extended`。
- 机器上已有的命令（owner 自己装的）。同样只在 `commands` 里写名字。

要随 skill 一起交付新命令时，把命令做成扩展包（`app_type = ext_tools` 的 pikg，见 [Session Runtime](<../opendan/OpenDAN Agent-Session-Runtime.md>) §3），再发一个依赖它的 skill pikg：skill 的 `commands` 写这些命令名，系统按扩展包目录的 `provides` 告诉 owner 启用哪个包能补上（`buckyos skill status`）。两个 pikg 各发各的。

## 4. 从目录试用

安装只有 owner 能做，Agent 自己没有安装命令。用 owner 身份把源目录直接装到一个 Agent 上：

```bash
buckyos skill install ./skills --agent <agent_id>
buckyos skill list --agent <agent_id> --kind work
buckyos skill status --agent <agent_id>
```

- `install` 先打印安装计划与风险清单（会装哪些 skill、覆盖谁、手册依赖哪些命令、lint 结果），确认后才写入；非交互环境加 `--yes`。
- `<agent_id>` 是完整的 AgentId（如 `jarvis.test.buckyos.io`），不是短名。
- 安装只影响之后创建的 Session。试用时开一个新的 Work，让创建者点名这份手册；ui skill 还要 `buckyos skill ui-add <name> --agent <agent_id>`，然后等对话的 Session 重建。
- 改了再装一遍即可；不用了 `buckyos skill remove <name> --agent <agent_id>`，临时关掉用 `buckyos skill disable <name> --agent <agent_id>`。
- 也可以在 Agent 的 Skills 页（OpenDAN WebUI）做同样的事，并按 runtime 看每份手册的命令是否齐全。

## 5. lint 规则

`buckyos pikg doctor` 与安装计划用同一套检查。拒绝安装的：

- 文件不在上面的目录布局里，或 skill 没有 `SKILL.md`；
- front matter 的 `scope` / `baseline` / `platforms` 不是固定词，`name` 与目录不一致；
- plan / goal skill 写了 `scope` 或 `commands`；
- 包里有 `tools/` 目录（命令、`install.toml`、脚本），或有不是 UTF-8 文本的文件：skill 是纯文本，命令做成扩展包。

只警告的：

| 代码 | 含义 |
|---|---|
| `no_description` | 没有 `description`，选择方只能看到名字 |
| `undeclared_commands` | shell 代码块里运行了没写进 `commands` 的命令 |
| `unused_commands` | `commands` 里的名字正文从未出现 |
| `ui_manual_large` / `plan_manual_large` | ui skill 超过 16 KB / plan skill 超过 3 KB |
| `plan_depends_on_commands` | plan skill 正文有命令或点名了具体手册 |
| `ui_scope` | 这份手册会进入对话的系统提示 |
| `replaces_skill` | 与已装的 skill 同名，会覆盖 |
| `network_script`、`package_manager`、`privilege`、`secrets` | 手册让 Agent 下载脚本并执行（`curl … \| sh`）、用包管理器、提权（`sudo`）、读取密钥类环境变量 |

## 6. 打包

在 `skills/` 的上一级目录：

```bash
buckyos --non-interactive pikg init . --name <pkg> --owner did:bns:<owner> --kind agent-skills --source ./skills
buckyos pikg doctor ./dapp_meta
buckyos pikg build ./dapp_meta
buckyos pikg pack ./dapp_dist
buckyos pikg info ./dapp_dist/<pkg>-<version>.pikg
```

得到 `app_type = agent_skills` 的 PIKG，唯一子包 `agent_skills` 就是 `skills/` 的内容。一个包可以带多份 skill（例如一份 plan skill 配几份 work skill）。版本在 `dapp_meta/app.json`，升版后重新 build / pack。用 `.pikg` 再装一次，确认与目录安装的结果一致：

```bash
buckyos skill install ./dapp_dist/<pkg>-<version>.pikg --agent <agent_id>
```

## 7. 发布与安装

发布走普通 App 的流程，步骤与当前限制（Owner 签名、BNS 发布）见手册 `buckyos_dev_release`：

```bash
buckyos app publish <control-panel 主机上的绝对目录> --app-doc <AppDoc 模板>
```

使用者（owner）安装：

```bash
buckyos skill install <x.pikg | https://… | did:bns:…> --agent <agent_id>
```

来源解析与 `buckyos app install` 相同，非本地来源由 CLI 下载后做离线校验，再走同一份安装计划与确认。skill 包不进普通 App 安装器，不会出现在 App 列表里。

## 8. Agent 产出的草稿

Agent 不能安装，但能把一段文本或反复做的一类工作写成草稿，放在 `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/`（布局同 skill 包，外加 `draft.json`：来源、拆分说明、命令核验结果、风险说明）：

```bash
buckyos skill import <文件 | URL | -> --agent <agent_id> [--name <slug>] [--scope work|ui|plan]
buckyos skill drafts --agent <agent_id>
buckyos skill install --draft <slug> --agent <agent_id>
```

`import` 启动一个 `skill_import` Work，结束后草稿出现在 Skills 页的草稿区；在对话里对 Agent 说“把这段做成 skill”效果相同。装上的草稿来源记为 `imported`，只对核验过的那台 runtime 的平台有效；要发行给别人，先整理成 `skills/` 目录再按 §6 打包。
