# OpenDAN Agent提示词编排的三种范式总结

> 以下内容根据语音记录整理，尽量保留原意，并对口语表达做了结构化归纳。

## 一、问题背景

在 OpenDAN Agent 的设计里，提示词编排（Prompt Orchestration）讨论的核心，不只是“提示词里放什么”，还包括以下几个更关键的问题：

- 历史 step 应该如何组织。
- memory、worklog、任务目标、原始输入之间应该如何分层。
- 当上下文窗口逼近边界时，应该如何裁剪或压缩。
- 如何让大语言模型既能理解任务目标，又能保持对最近执行过程的连续性感知。

围绕这些问题，可以归纳出三种典型范式。

---

## 二、范式一：传统消息追加式

### 1. 基本结构

这是最传统、也最接近 OpenAI 早期 Chat API 使用习惯的一种结构。

典型形式是一个 message 数组，按顺序组织为：

1. **system message**：系统提示词。
2. **第一条 user message**：承载提示词编排内容，例如 memory、worklog 等。
3. **第二条 user message**：用户本次原始输入，可视为 `step0`。
4. 此后每推进一次 agent loop，就新增一条 user message，表示该 step 的结果，例如 `step1`、`step2`、`step3`……

换句话说，这种模式把整个执行过程组织成一条不断向后追加的消息链。

### 2. 窗口压缩策略

当上下文接近模型窗口上限时，通常会采用两类压缩方式：

#### 策略 A：从前往后删除旧 step，再并入 memory

- 从 `step0` 开始，删除较早的一批 step，例如每次删除 5 个 step。
- 删除后，把这几步的摘要合并到前面的 memory 区域。
- 这样可以腾出新的上下文空间。

这是一种非常朴素、也非常常见的做法。

#### 策略 B：永久保留 `step0`

- `system message`、memory 编排块，以及原始输入 `step0` 始终保留。
- 真正被压缩的是 `step1` 之后的历史执行过程。

这种做法隐含的假设是：**原始输入始终具有最高优先级，不应被裁掉。**

### 3. 优点

- 结构直观，容易实现。
- 很符合传统聊天记录式、函数调用式的线性追加逻辑。
- 与很多已有工程方案兼容，属于“默认就会做成这样”的模式。

### 4. 问题与争议

这类结构虽然自然，但也有明显讨论空间。

首先，从提示词注意力分布的角度看，如果“用户原始输入”是最重要的信息，那么它所处的位置并不一定理想。因为：

- memory 区域会持续增长；
- 前部的提示词编排块并不是静态不变的；
- 每次压缩后，前面的 user message 反而会越来越大。

结果就是：**原始输入虽然被保留了，但它在整个上下文中的相对位置会越来越尴尬。**

也就是说，这种方案在工程上很顺手，但在“重要信息应该被放在哪里”这件事上，不一定是最优解。

---

## 三、范式二：固定三段式编排

### 1. 核心思想

第二种结构可以称为“编排优先”模式。

它的基本观点是：

> `user message` 这种一条条追加的历史形式，本质上更像是配合强化训练与聊天范式形成的接口习惯；从提示词工程本身看，并没有必要机械保留每个 step 的独立消息形态。

在这种思路下，新的 API 更像是一个 **input 数组**，而不是不断增长的消息链。整个输入被固定为三块。

### 2. 固定三块结构

#### 第一块：system

- 系统提示词。
- 这一块没有太大争议。

#### 第二块：memory 大块

- 所有 step 的历史信息都被并入这一块。
- 不再强调“某个 step 是否单独作为一条 message 存在”。
- 差别只体现在压缩比例上。

例如：

- 最近 5 个 step 完整保留；
- 更早的 step 自动压缩为摘要；
- 所有这些内容都放在同一个 memory 区块里。

因此，这一块会变成一个非常大的“编排输入区”。

#### 第三块：原始输入 / 任务空间

这一块用于表达当前任务最本质的目标来源。

- 如果是聊天场景，这一块就是用户当前输入。
- 如果是任务场景，这一块更像是 `task session summary`，也就是任务创建时的目标定义与任务空间描述。

它类似一张挂在墙上的 to-do list，明确告诉模型：

- 主目标是什么；
- 当前进行到哪一步；
- 当前这轮推理面向的核心任务边界是什么。

### 3. 优点

- 结构稳定，不随 agent loop 次数变化而改变框架。
- 无论 loop 如何运行，本质上始终是三块内容。
- 历史信息管理更集中，压缩策略更统一。
- 对任务型 Agent 来说，任务空间与目标表达会更清晰。

### 4. 争议点

这类模式也引发了一些工程上的反对意见。

一种观点认为，这其实有一点“反模式”的味道。原因是：

- 很多大语言模型并不是围绕这种固定三段式结构训练出来的；
- 它们更熟悉一条条往下追加的聊天序列；
- 因此，在这种新结构下，模型未必能像范式一那样自然地理解“上一个 step 与当前 step 的连续性”。

换句话说，反对者担心：

> 范式二虽然更整洁，但模型对“连续执行过程”的感知，可能反而弱于传统消息追加式。

### 5. 范式二的一个常见改进

为了解决“连续性不足”的问题，有一种小幅改法：

- 在最后一个 input 中，强制把“原始输入”与“上一个 step 的结果”放在一起；
- 这样可以让上一个 step 的运行结果出现在整个提示词的最末尾。

这种改法的意图非常明确：

- 让模型在处理当前轮时，能优先看到最新一步的执行结果；
- 同时保留任务初始目标，避免偏航。

### 6. 该改进带来的新疑问

但这个补丁式优化又引出了新的担忧：

- 当 Agent 已经运行很多轮之后，原始输入与“上一个 step”之间，未必仍然具有那么强的直接相关性；
- 如果在提示词末尾把二者放得过近，是否会让模型误以为它们之间存在更强的即时关联；
- 这种“强行相邻”是否反而可能增加幻觉风险。

进一步说，模型甚至可能形成一种错觉：

> 好像整个任务只做了两步——原始输入一步、上一个结果一步。

而这会弱化中间长链路推理过程的存在感。

---

## 四、范式三：分层时间压缩式（改进方向）

### 1. 基本思路

第三种范式可以看作是在前两种模式之间寻找折中。

它的关键目标是：

- **memory 负责承载可沉淀、可覆盖、偏结论性的内容；**
- **最后一个输入块负责承载强时间顺序的信息。**

也就是说，第三种模式不否定 memory，也不否定任务空间；它试图把“长期信息”和“强时序信息”分开放置。

### 2. 结构方式

在这种方案里：

#### memory 区域

- 已经包含历史摘要；
- 与最终输入区并不冲突；
- 更适合存放“结论性信息”或“状态机式信息”。

例如：

- 已确认的事实；
- 当前环境状态；
- 可被后续覆盖的观察结果；
- 不需要严格依赖时序顺序来理解的工作记忆。

这意味着 memory 虽然也可能按时间排列，但它的**顺序性并不是最关键的属性**。

#### 第三块（最终输入区）

第三块继续保留 `step0`，也就是任务目标或原始输入，同时对历史 step 做分层展开：

1. **保留任务空间整体摘要**；
2. **保留任务目标 / 原始输入（step0）**；
3. **将较早的 step（例如 `step1 ~ stepN-3` 或 `stepN-5`）压缩到最小，只写“这一步做了什么”**；
4. **将最近几步写得更详细**；
5. **让最后一个 step（last step）尽量完整保留，甚至直接放全文。**

换句话说，模型最终看到的最后一条大输入，更像是一个按时间梯度展开的结构：

```text
任务空间摘要
→ 原始目标 / step0
→ 早期步骤的极简摘要
→ 最近几步的详细记录
→ last step 全文
```

### 3. 这种设计想解决什么问题

这种模式希望同时满足两件事：

#### 第一，保住任务目标的稳定性

- `step0` 依然很重要；
- 原始任务目标不会因为历史膨胀而被挤掉；
- 模型始终知道自己最初要完成什么。

#### 第二，保住执行链的时间连续性

- 真正强时序的信息，被集中放在最后一个输入区；
- 模型可以从任务目标一路读到最近执行结果；
- 特别是最后一个 step，会以最强的形式暴露给模型。

这种安排隐含的判断是：

> 对大语言模型来说，真正需要按时间顺序强感知的，不是整个 memory，而是最近这条执行链，尤其是 last step。

### 4. 对 memory 的进一步理解

在这种范式下，memory 更像“可沉淀事实层”，而不是“完整时序回放层”。

一种相关观点认为：

- 历史上的某些观察结果本身未必长期有意义；
- memory 中更适合存放那些接近状态机的内容；
- 这类信息可以被新观察覆盖，而不必始终严格保留完整顺序。

因此：

- **顺序性强的内容**，放在最后输入区；
- **顺序性弱但可复用的内容**，放在 memory 区。

这是第三种范式最关键的分层原则。

### 5. 适用前提

这种模式有一个重要前提：

- 单个行为循环里，可观察的 step 数量通常不会太夸张；
- 例如双 loop 场景下，一层里通常不会超过 30 个 step；
- 如果每个 step 只用一两句话概述，那么把它们按“越近越详细”的方式放进最后输入区，是可控的。

这意味着第三种模式成立的前提之一是：

> 必须控制最后一个 input 的总体大小，避免它再次膨胀成另一个 memory 大块。

也就是说，第三种模式不是简单把所有 step 平铺，而是**有节制地平铺**：

- 早期步骤极简；
- 最近步骤展开；
- last step 完整；
- 总长度始终受控。

---

## 五、三种范式的对比

| 维度 | 范式一：传统消息追加式 | 范式二：固定三段式编排 | 范式三：分层时间压缩式 |
|---|---|---|---|
| 基本组织方式 | 多条 message 线性追加 | 固定三块 input | memory 与最终输入分层 |
| 历史 step 形态 | 每个 step 独立追加 | 全部并入 memory 大块 | 早期压缩、近期展开 |
| 原始输入位置 | 较靠前，可能被膨胀内容“挤住” | 单独保留为固定第三块 | 单独保留，并与时间链结合 |
| 连续性感知 | 强，符合传统聊天/调用序列 | 可能偏弱，存在争议 | 希望兼顾连续性与结构性 |
| memory 的角色 | 历史压缩容器 | 主要历史承载区 | 沉淀事实层 / 状态层 |
| last step 的权重 | 取决于消息序列位置 | 可通过补丁放到末尾 | 明确保持最高时序权重 |
| 主要风险 | 原始输入位置逐渐尴尬 | 模型可能不适应三段式结构 | 最终输入区若失控会重新膨胀 |

---

## 六、结论

从这三种范式来看，讨论的核心并不是“哪一种绝对正确”，而是：

1. **模型更依赖哪种上下文组织方式；**
2. **任务目标与最近执行结果，谁应该占据提示词中的关键位置；**
3. **memory 应该承担“时序回放”职责，还是承担“状态沉淀”职责。**

如果做一个归纳：

- **范式一**代表传统做法，最自然、最符合已有聊天序列，但原始输入的位置可能越来越不理想。
- **范式二**代表结构化编排思路，整体更整洁，但有人担心模型未必擅长理解这种固定三段式连续性。
- **范式三**则是一个折中改进方向：让 memory 承载可沉淀信息，让最后输入区承载强时间顺序，试图同时保住任务目标、过程连续性与上下文可控性。

因此，第三种范式的价值，不在于简单替代前两种，而在于提出一个更明确的分工：

> **把“长期可复用信息”和“短期强时序信息”拆开处理。**

这可能是 OpenDAN Agent 在提示词编排层面一个值得继续实验和验证的方向。

---

# 范式三实例（修订版）：Step 15 时刻的完整提示词与推理结果

## 核心模型修正

### Step 的真实时序

一个完整的 Step 横跨两个阶段：

```
action_result(n-1)
    ↓
[LLM 推理]
    ↓
LLM Result(n): conclusion(n) + thinking(n) + next_action(n)
    ↓
[系统执行 next_action]
    ↓
action_result(n)
```

- **conclusion**：承上——消化上一个 action_result，提炼结论
- **thinking**：桥梁——基于 conclusion 推导下一步该做什么
- **next_action**：启下——发起下一个系统调用

### 三级压缩规则

| 压缩级别 | LLM Result 保留 | action_result |
|---------|----------------|---------------|
| 不压缩 | conclusion + thinking + next_action | 全文保留 |
| 半压缩 | conclusion + next_action | 丢弃 |
| 全压缩 | conclusion 仅保留 | 丢弃 |

**关键原则**：conclusion 是 LLM 对 action_result 的消化，只要 conclusion 在，action_result 就可丢弃。无需 `set_memory`，无需系统侧智能摘要，纯机械裁剪。

---

## 场景设定

- **用户任务**：调研自托管 NAS 方案，输出一份选型报告
- **Agent 类型**：Research Agent（OpenDAN Behavior Loop）
- **当前状态**：Step 15 的 action_result 刚返回，系统正在组装 input 发起第 16 轮 LLM 推理

---

## Input 结构总览

```
input[0]  →  System Prompt
input[1]  →  Memory 块（跨步骤聚合的高阶状态）
input[2]  →  展开区（任务空间 + 三级压缩的 Step 序列）
```

---

## input[0] — System Prompt

```xml
<system>
你是 OpenDAN Research Agent。

## 输出格式
每次推理必须输出以下三个字段：
- <conclusion>: 基于上一步 action_result 的结论提炼（必填，即使结论是"无新发现"）
- <thinking>: 基于 conclusion 和当前任务状态的推理过程
- <next_action>: 下一步要执行的操作（或 finish）

## 可用 action
- web_search(query)
- read_url(url)
- note(content)
- write_section(title, content)
- ask_user(question)
- finish(summary)

## 行为约束
- 每步只执行一个 action
- conclusion 应当是自包含的——后续步骤可能看不到原始 action_result，
  所以关键数据和判断必须在 conclusion 中完整表达
</system>
```

> 注意 system prompt 里对 conclusion 的约束："自包含"。
> 这是让三级压缩能 work 的前提——LLM 知道 conclusion 会被独立保留。

---

## input[1] — Memory 块

> **定位**：跨步骤聚合的高阶状态。不记录单步结论（那些在展开区），
> 只存经过多步交叉验证后的稳定结论和用户偏好。

```xml
<memory>
## 用户偏好
- 偏好开源，闭源方案可纳入对比但需注明 license
- 技术背景：熟悉 Linux/Docker，有 homelab 经验
- 预算：硬件 ¥3000-5000，软件免费优先

## 候选方案（已稳定）
- TrueNAS SCALE | OpenMediaVault | Unraid | CasaOS

## 已排除
- Synology DSM / QNAP QTS（闭源，用户明确排除）

## 评估维度（已确认）
- 文件系统与数据安全 / 容器生态 / 社区与文档 / 硬件要求 / 易用性

## 报告进度
- [完成] 第 1-4 章
- [进行中] 第 5 章：社区与文档质量对比
- [待写] 第 6-7 章
</memory>
```

---

## input[2] — 展开区

```xml
<task_execution>

<!-- ===================== 任务空间（锚点） ===================== -->
<task_session>
  <goal>调研主流自托管 NAS 开源方案，输出选型对比报告（markdown）</goal>
  <constraints>聚焦开源/社区版；覆盖 5 个评估维度；交付结构化 markdown 报告</constraints>
  <progress>Step 15 / 预估 ~20 步</progress>
</task_session>


<!-- ===================== 全压缩区：Step 1-10 ===================== -->
<!-- 仅保留 conclusion -->

<steps_compressed>
  <step n="1">
    <conclusion>初始搜索确定 6 个候选方案：TrueNAS SCALE、OMV、Unraid、CasaOS、Rockstor、XigmaNAS。后两者社区较小，暂列备选。</conclusion>
  </step>
  <step n="2">
    <conclusion>TrueNAS vs OMV 的主流对比观点：TrueNAS 更适合数据安全优先场景（ZFS），OMV 更适合轻量/低功耗部署（Debian 基础）。</conclusion>
  </step>
  <step n="3">
    <conclusion>TrueNAS SCALE 硬件要求：最低 8GB 内存（强烈推荐 ECC），需 x86_64，不支持 ARM。启动盘需独立 16GB+ SSD。</conclusion>
  </step>
  <step n="4">
    <conclusion>OMV 硬件门槛最低，可运行在树莓派 4/5 上。基于 Debian，插件体系通过 omv-extras 扩展，核心插件约 30 个。</conclusion>
  </step>
  <step n="5">
    <conclusion>Homelab 社区中 Unraid 用户满意度最高（灵活磁盘管理是核心卖点），但 license $59-$129 且非开源是主要争议点。</conclusion>
  </step>
  <step n="6">
    <conclusion>Unraid 定价：Basic $59（6盘位）/ Plus $89（12盘位）/ Pro $129（无限）。一次付费终身有效。源码不公开，社区插件开源。</conclusion>
  </step>
  <step n="7">
    <conclusion>CasaOS 定位为入门级，UI 最简洁但功能偏弱：无原生 RAID 支持，无 Btrfs/ZFS 集成，不适合作为主力存储方案。</conclusion>
  </step>
  <step n="8">
    <conclusion>用户确认：Unraid 可纳入对比但需在报告中明确标注非开源 license。Rockstor 和 XigmaNAS 因社区规模过小正式排除。</conclusion>
  </step>
  <step n="9">
    <conclusion>完成四方案五维度评估矩阵草稿。初步判断：数据安全维度 TrueNAS 领先，易用性维度 Unraid/CasaOS 领先，综合需要进一步量化。</conclusion>
  </step>
  <step n="10">
    <conclusion>报告第 1 章（引言与需求定义）和第 2 章（候选方案概览）已写入。第 2 章包含四方案的基本介绍和定位对比表格。</conclusion>
  </step>
</steps_compressed>


<!-- ===================== 半压缩区：Step 11-13 ===================== -->
<!-- 保留 conclusion + next_action -->

<steps_summary>
  <step n="11">
    <conclusion>ZFS 在数据完整性方面仍是 2025 年社区共识最强选择。Btrfs RAID5/6 仍标记为不稳定，生产环境不推荐。Unraid 自有 parity 机制可靠性介于两者之间，优势是允许混合容量磁盘。据此判定数据安全维度排序：TrueNAS (ZFS) > Unraid (parity) > OMV (Btrfs/mdadm) > CasaOS (无)。</conclusion>
    <next_action>write_section("3. 文件系统与数据安全对比", ...)</next_action>
  </step>
  <step n="12">
    <conclusion>第 3 章已写入，覆盖 ZFS / Btrfs / Unraid parity 三条技术路线的对比，含数据完整性、自愈能力、快照支持三个子维度的对比表格。</conclusion>
    <next_action>web_search("TrueNAS SCALE docker compose support 2025")</next_action>
  </step>
  <step n="13">
    <conclusion>TrueNAS SCALE 在 24.10 (Electric Eel) 版本正式从 K8s 切回 Docker Compose 原生支持，社区反响正面。OMV 通过 Portainer 插件管理容器，体验流畅但自定义性弱于直接 CLI。Unraid Community Apps 商店模板 2000+，一键体验最好但自定义 compose 需额外插件。CasaOS Docker 原生但缺 compose 和高级网络配置。容器生态排序：Unraid (生态) ≈ TrueNAS (灵活性) > OMV (中规中矩) > CasaOS (基础)。</conclusion>
    <next_action>write_section("4. 容器生态对比", ...)</next_action>
  </step>
</steps_summary>


<!-- ===================== 不压缩区：Step 14-15 ===================== -->
<!-- 完整保留 conclusion + thinking + next_action + action_result -->

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

</task_execution>
```

---

## LLM 推理输出（第 16 轮）

基于上述 input，LLM 生成的 Step 16：

```xml
<conclusion>
OMV 社区规模约为 TrueNAS 的 1/4 到 1/3：GitHub 4.2k stars（TrueNAS 未统计
但生态更大）、论坛月活 300-400（TrueNAS 1200+）、Reddit 15k（TrueNAS 85k）。
核心风险是项目高度依赖单一维护者 votdev，公交因子（bus factor）极低。
但插件生态 omv-extras 由独立社区维护，增加了一定的抗风险性。
文档质量中等，基础安装文档完善但高级配置靠社区帖子。
</conclusion>

<thinking>
TrueNAS 和 OMV 的社区数据已拿到。还剩 Unraid 和 CasaOS。
Unraid 虽然非开源但社区非常活跃（从 Step 5 的调研已知用户满意度最高），
它的论坛是 NAS 领域最大的独立社区之一，值得单独搜。
CasaOS 是较新的项目，社区规模预期较小，可以和 Unraid 一起搜或单独一步。
考虑到 Unraid 社区数据量可能较大，先单独搜 Unraid。
</thinking>

<next_action>web_search("Unraid community forum Reddit activity 2025")</next_action>
```

---

## 压缩效果分析

### conclusion 的信息密度对比

以 Step 3 为例，对比全压缩区的 conclusion 和原始 action_result：

| | 内容 | 预估 token |
|---|---|---|
| action_result 原文 | TrueNAS 官方文档全页内容，含安装步骤、兼容硬件列表、BIOS 设置建议... | ~800 |
| conclusion | "TrueNAS SCALE 硬件要求：最低 8GB 内存（强烈推荐 ECC），需 x86_64，不支持 ARM。启动盘需独立 16GB+ SSD。" | ~40 |

压缩比约 **20:1**，且保留了对后续决策真正有用的信息。

### 展开区 token 预估

| 区域 | 步数 | 单步 token | 合计 |
|------|------|-----------|------|
| 任务空间 | — | — | ~100 |
| 全压缩 (conclusion only) | 10 步 | ~40 | ~400 |
| 半压缩 (conclusion + next_action) | 3 步 | ~80 | ~240 |
| 不压缩 (完整四段) | 2 步 | ~350 | ~700 |
| **展开区合计** | | | **~1440** |

加上 System Prompt (~200) 和 Memory 块 (~300)，**总 input 约 2000 token**。

### Step 25 时的预估

| 区域 | 步数 | 合计 |
|------|------|------|
| 全压缩 | 20 步 | ~800 |
| 半压缩 | 3 步 | ~240 |
| 不压缩 | 2 步 | ~700 |
| **展开区合计** | | **~1840** |

增长 10 步，展开区仅增加 ~400 token（全压缩区的 10 个 conclusion）。

### 与旧版对比

| 改进点 | 旧版 | 修订版 |
|-------|------|--------|
| 全压缩保留内容 | action final（干了什么） | conclusion（得出了什么） |
| 信息密度 | 低——"搜索了 TrueNAS 文档" | 高——"TrueNAS 最低 8GB 内存，推荐 ECC" |
| Memory 写入依赖 | 需要 set_memory 显式操作 | 不需要——conclusion 即持久化单元 |
| 压缩机制 | 需要 LLM 判断"是否值得存" | 纯机械裁剪，无智能决策 |
| LLM 额外负担 | 每步需做 set_memory 元决策 | 仅需保证 conclusion 自包含（system prompt 约束） |


## 七、可进一步验证的问题

结合以上三种范式，后续可以重点验证以下问题：

1. 模型是否真的更擅长理解“消息追加式”而不是“固定三段式”。
2. 原始输入是否必须长期固定在高优先级位置。
3. last step 放在提示词最末尾，是否会显著提升动作连续性。
4. memory 中到底应该存“历史观察”，还是只存“状态化结论”。
5. 在 step 数量约 20～30 的情况下，分层压缩是否能比简单摘要带来更好的稳定性与更低幻觉率。

