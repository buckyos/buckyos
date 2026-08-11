
## System 纯模板

<task_session>
  <goal>调研主流自托管 NAS 开源方案，输出选型对比报告（markdown）</goal>
  <constraints>聚焦开源/社区版；覆盖 5 个评估维度；交付结构化 markdown 报告</constraints>
  <progress>Step 15 / 预估 ~20 步</progress>
</task_session>


## Memory

能否用存模板实现？

Memory 系统函数，通过配置实现，格式相对固定？
    - Agent  Memory过滤
    - History Messages / StepLog 2选1？ 其中StepLog是History Message的超集

    
    

## Input 
纯模板 ，核心是不压缩的，放出前n个step的完整信息



<step n="14">
  <conclusion>第 4 章已成功写入。内容覆盖四方案的 Docker 支持方式、商店/模板生态、CLI 自由度三个子维度对比，含横向对比表格。报告前 4 章已完成，进入第 5 章"社区与文档质量对比"阶段。</conclusion>
  <thinking>第 5 章需要量化四个方案的社区活跃度。需要逐一搜索每个方案的论坛、GitHub、Reddit 数据。TrueNAS 作为最大的开源 NAS 项目，先搜它作为基准线，后续方案可做对比参照。</thinking>
  <next_action>web_search("TrueNAS SCALE community forum activity 2025")</next_action>
  <action_result>
    搜索返回 8 条结果：
    1. TrueNAS Community Forums - 月活帖子约 1200+，iXsystems 官方工程师
       定期回复，响应时间通常 24h 内。2025 年初因 24.10 从 K8s 迁移到
       Docker 引发大量讨论，整体反响正面。
    2. Reddit r/truenas - 订阅 85k+，日均新帖 15-20 条，NAS 相关
       subreddit 中最活跃。
    3. TrueNAS 官方文档 - 覆盖全面但部分滞后于版本更新（24.10 文档仍有
       K8s 残留描述）。有独立 contributor guide。
    4. YouTube - TechnoTim、Raid Owl 等 homelab 频道有 TrueNAS 系列，
       2024-2025 更新频繁。
    5-8. 其他论坛/博客，主要是迁移指南和硬件兼容性经验帖。
  </action_result>
</step>

<step n="15">
  <conclusion>TrueNAS 社区在四个候选方案中活跃度最高：论坛月活 1200+ 帖、Reddit 85k 订阅、官方工程师深度参与。主要短板是文档更新滞后于版本发布。YouTube 教程生态丰富，降低了上手门槛。以此作为后续方案社区数据的对比基准线。</conclusion>
  <thinking>TrueNAS 社区数据已足够，作为基准线建立完毕。下一步搜 OMV 的社区数据。OMV 基于 Debian 生态，它的社区有一部分和 Debian 社区重叠，搜索时需注意区分 OMV 专属社区和泛 Debian 讨论。优先搜 OMV 官方论坛和 GitHub 活跃度。</thinking>
  <next_action>web_search("OpenMediaVault community forum GitHub activity 2025")</next_action>
  <action_result>
    搜索返回 6 条结果：
    1. OMV GitHub (openmediavault/openmediavault) - 4.2k stars，过去 30 天
       32 commits，主要贡献者 3 人（核心维护者 votdev 占 80%+）。
       Issue 响应较快但 PR 合并周期长（平均 2-3 周）。
    2. OMV 官方论坛 (forum.openmediavault.org) - 月活帖子约 300-400，
       主要是安装配置类问题。深度技术讨论较少。
    3. Reddit r/OpenMediaVault - 订阅 15k，日均新帖 2-3 条，活跃度
       约为 r/truenas 的 1/5。
    4. OMV 插件生态 (omv-extras) - 由社区成员 omv-extras.org 维护，
       提供约 20 个额外插件，更新频率稳定。
    5-6. 博客评测文章，主要关注 OMV 7 (基于 Debian 12) 的新特性。
  </action_result>
</step>

## Output Protocol
系统决定放在System Prompt后，还是放在Input后

例子

```yaml
system: |
  <<process_rule>>
  Core mission (DO):
  - Execute the CURRENT TODO toward its acceptance criteria as fast as possible.
  - Work in an **Observe → Execute** loop.
  - Stay focused on the current todo. Do NOT create new todos (`todo add` is forbidden).
    Extra work goes to `todo note <id> "..." --kind=note`.

  --------------------------------------------------------------------
  OBSERVE → EXECUTE LOOP

  Phase = step_index parity:
  - **OBSERVE** (even: 0, 2, 4, …) — read, analyze, plan, decide transitions.
  - **EXECUTE** (odd:  1, 3, 5, …) — write, build, test; motivation in `reply`.

  ### OBSERVE (even step_index)

  Goal: Gather the information the next EXECUTE needs. Be efficient — batch reads.

  `thinking`: analyze observations, plan write actions for EXECUTE.
  `reply`: brief status ("what I found, what I'll do next").
  `actions`/`shell_commands`: **READ-ONLY**.
  - `read_file`, `cat`, `ls`, `grep`, `git diff`, `cargo check`, `cargo test`, …
  - `todo show/ls/current/next/pending`
  - Exception: `todo start <id>` allowed at step 0.

  `next_behavior` — **only OBSERVE decides transitions**:
  - Self-check PASSED → `CHECK:todo=<id>`
  - Persistent failure → `ADJUST:todo=<id>`
  - Need user input → `WAIT_FOR_MSG`
  - Otherwise: do NOT set (continue loop).

  ### EXECUTE (odd step_index)

  Goal: Apply changes based on the preceding OBSERVE. Move fast.

  `thinking`: derive execution plan from last_step_summary.
  `reply`: **motivation** — WHY you make these changes + expected outcome.
  `actions`/`shell_commands`: **WRITE allowed**.
  - `write_file`, `edit_file`, shell build/test, `todo start/done/fail/note`.

  `next_behavior`: **MUST NOT be set**. Next OBSERVE decides.

  Self-check: when you believe the todo is done, run validation commands
  in this EXECUTE step and record via `todo note <id> "..." --kind=result`.
  The following OBSERVE will read the results and transition to CHECK.

  Phase discipline (MUST):
  - OBSERVE (even): read-only. No write_file/edit_file.
  - EXECUTE (odd): writes allowed. next_behavior MUST NOT be set.
  - next_behavior only in OBSERVE.

  Todo commands (MUST):
  - Allowed: `todo start`, `todo done`, `todo fail`, `todo note`
  - Forbidden: `todo pass`, `todo reject`, `todo add`
  - Notes: --kind=note (progress), --kind=result (self-check), --kind=error (errors)

  Failure: retry once in next EXECUTE; if still failing, ADJUST in next OBSERVE.

  Reply:
  - OBSERVE: brief status.
  - EXECUTE: motivation (why + expected outcome).
  <</process_rule>>

  <<task_session>>
  # ======================
  # Current TODO (My target)
  # ======================
  __OPENDAN_VAR(workspace_todolist,$workspace_todolist)
  {{workspace_todolist.__OPENDAN_ENV(params.todo)__}}
  <</task_session>>

memory:
  agent_memory: { limit: 3500, max_percent: 0.25 }
  step_logs: { limit: 10000, max_percent: 0.5 }

input: |
  __OPENDAN_VAR(new_msg,$new_msg)
  __OPENDAN_VAR(last_step,$session_last_step)
  {% if step_index > 0 %}
  {% for i}
  {% endif %}

  # Current Step Index: {{step_index}} 

  {% if new_msg %}
  # New message:
  {{new_msg}}
  {% endif %}



output_protocol:
  mode: behavior_llm_result

faild_back: adjust
step_limit: 64

limits:
  max_prompt_tokens: 64000
  max_completion_tokens: 200000
  max_tool_rounds: 2
  max_tool_calls_per_round: 8
  deadline_ms: 120000

llm:
  model_policy:
    preferred: llm.code.default

```