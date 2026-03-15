# 《OpenDAN Agent框架集成自动化测试》技术需求文档

* 文档版本：V1.0
* 编写日期：2026-02-28
* 适用范围：OpenDAN Agent 框架（端到端/全流程集成测试）

---

## 1. 背景与问题

随着 OpenDAN Agent 框架技术功能增多，现有测试方式面临以下痛点：

1. **真实测试成本高、速度慢**
   真实跑全流程会触发大量大语言模型（LLM）推理与真实请求：

   * 调用成本高（付费推理/请求）。
   * 等待时间长（LLM响应慢导致回归测试时间不可控）。

2. **故障定位困难**
   当测试失败时，第一反应往往会怀疑：

   * 是不是 LLM 本身输出波动/不稳定？
   * 还是框架内部逻辑/数据结构/编解码出问题？
     这会导致问题难以聚焦在“框架可控部分”。

3. **单元测试不足以覆盖系统真实运行形态**
   需求不是做纯单元测试，而是希望**在尽可能接近真实系统运行的情况下**，把整个系统启动并跑完整链路，验证 Agent loop、消息处理、worklog 等关键链路。

---

## 2. 建设目标

构建一套“**集成自动化测试框架**”，满足：

1. **系统尽可能真实地跑起来**
   除了 LLM 组件（AICC）替换为 Mock，其它组件尽量保持真实运行。

2. **端到端全功能全流程测试**
   从“向 agent 发送消息”开始，到“最终回复输出 + worklog链路验证”结束。

3. **确定性、可回放、可对比**

   * Mock AICC 按“确定性 KV 剧本”返回结果。
   * 测试可重复运行，结果稳定。
   * worklog 可作为可观测证据进行回放比对。

4. **测试执行速度快、成本低**
   回归测试阶段不发生真实 LLM 推理请求。

---

## 3. 范围与非目标

### 3.1 范围（In Scope）

* 集成测试（Integration / E2E）框架：

  * 启动 OpenDAN 系统（除 AICC 外）
  * Mock AICC 回放 LLM 输入输出
  * 通过 HTTP 向 agent 发送标准化 message（绕过 Telegram 网关）
  * 校验：最终回复 + worklog 全链路对比
* 强类型剧本生成能力（生成/记录 KV 剧本、worklog baseline）
* 类型编解码相关单元测试（覆盖关键结构，保证序列化/反序列化随版本演进）

### 3.2 非目标（Out of Scope）

* 不追求完全替代所有真实线上环境差异（例如：外部三方服务的真实可用性）。
* 不在回归阶段做真实 LLM 输出质量评估（回归关注“框架行为一致性”）。
* 不把 Telegram 网关作为必须测试对象（默认通过 agent HTTP 接口测试）。

---

## 4. 术语定义

* **AICC**：系统中的 LLM 推理/请求组件（负责发起 LLM 请求并返回结果）。
* **Mock AICC**：替换真实 AICC 的测试组件，基于“KV剧本”回放输出。
* **剧本（Script）/KV剧本**：用于 Mock AICC 的确定性映射：

  * K：LLM Request（规范化/序列化后的输入）
  * V：LLM Response（对应输出）
* **剧本生成器（强类型脚本生成器）**：基于系统强类型结构生成/编译剧本（包括 prompt 渲染、输出解码/编码）。
* **Worklog**：系统运行过程的可观测日志/事件序列（含每次 AICC 输入输出、agent loop 关键步骤）。
* **Replay（回放）**：使用 Mock AICC + 固化剧本，快速运行并验证一致性。
* **Record（录制/生成基线）**：首次跑通或更新时，生成 KV剧本与 worklog 预期基线。

---

## 5. 总体方案概述

本方案由三层构成：

1. **类型覆盖的单元测试层（Type Coverage UT）**
   目标：确保系统所有关键数据结构的解析、序列化/反序列化可随版本迭代稳定演进。
   这层能力也是“强类型剧本生成器”的基础。

2. **强类型剧本生成器（Script Builder / Compiler）**
   目标：

   * 基于强类型输入渲染 prompt/请求结构
   * 对输出进行解码（并可再编码为 Mock 回放所需格式）
   * 生成 Mock AICC 可载入的 KV 剧本
   * 同时生成/维护 worklog 预期基线

3. **集成测试执行层（E2E Runner + Mock AICC + Worklog Assert）**
   流程：

   * 启动系统（AICC 替换为 Mock）
   * Runner 通过 HTTP 给 agent 发消息（标准化 message）
   * 校验即时回复
   * 拉取/读取 worklog，与预期基线做差异比对
   * 全部一致则测试通过

---

## 6. 核心流程设计

### 6.1 流程一：剧本生成/基线生成（Record / Build）

用于首次创建或更新测试用例。

1. 运行剧本生成器（Record 模式）
2. 系统以尽量真实方式跑一遍（可选择真实 AICC 或代理录制）
3. 捕获并固化：

   * 每一次 LLM 调用的 **Request/Response**（形成 KV）
   * 本次运行的 **worklog**（形成 baseline）
   * 每一步的外部可见结果（最终回复、必要中间产物）

> 备注：当初期很难判断 worklog 结构是否足够可见时，允许“先跑一遍观察”，确认可见字段后再保存 baseline，用于后续回放比对。

### 6.2 流程二：集成测试执行（Replay / Run）

用于 CI/回归的快速确定性测试。

1. Runner 加载某用例的 KV 剧本
2. 启动系统：仅替换 AICC 为 Mock，其它组件真实运行
3. Runner 通过 HTTP 向 agent 发送 message 序列
4. 校验每步 response 是否符合预期：

   * 不符合：立即 fail（快速反馈）
5. 获取本次运行产生的 worklog，执行与 baseline 的对比
6. worklog 一致：pass；否则 fail，并输出差异报告

---

## 7. 功能需求

### 7.1 类型覆盖单元测试（UT）需求

**FR-UT-01**：覆盖系统现存关键类型的序列化/反序列化测试

* 覆盖范围：与 agent loop、AICC request/response、message、worklog 相关的核心结构
* 目标：随系统版本变更及时发现类型不兼容/编解码错误

**FR-UT-02**：提供“类型快照/变更提示”能力（可选）

* 当类型字段发生变化时，能提示需要更新剧本生成器或测试 baseline

---

### 7.2 强类型剧本生成器需求

实现在test_script_builder.rs

**FR-SB-01**：支持基于强类型输入渲染 LLM 请求（prompt 渲染）

* 输入：结构化参数（强类型对象组合）
* 输出：LLM Request（与框架真实运行一致的结构）

**FR-SB-02**：支持对 LLM 输出进行解码，并可再编码为回放用输出

* 解码：将 LLM 自然语言/工具调用结果解析为系统期望结构
* 编码：将“期望结构”编码为 Mock AICC 需返回的 Response payload（确保运行时完全一致）

**FR-SB-03**：生成 KV 剧本（LLM Request → LLM Response）

* KV 必须满足确定性要求（见第8章）

**FR-SB-04**：生成/更新 worklog baseline

* 支持从一次运行中提取可比对字段并固化为 baseline

---

### 7.3 Mock AICC 需求

**FR-MA-01**：Mock AICC 只关心 KV 数据结构，不关心剧本复杂度

* 对于每个 LLM Request：

  * 计算 K（规范化/确定性序列化）
  * 命中返回 V
  * 未命中则报错（并输出诊断信息）

**FR-MA-02**：严格确定性回放

* 禁止引入随机变量/时间相关输出
* 对相同输入应始终返回相同输出

**FR-MA-03**：可加载多个用例剧本、支持命名空间隔离

* 防止不同测试用例间 KV 冲突

实现方式：当aicc启动后，如果在特定目录存在KV文件，则自动进入mock模式

---

### 7.4 Agent 消息驱动与测试 Runner 需求

**FR-RUN-01**：通过 HTTP 向 agent 发送标准化 message（脱离 Telegram）

* 每个 agent 应提供网络接口用于接收 message
* Runner 可以按步骤发送 message 序列

**FR-RUN-02**：每步都可断言“最终回复/关键输出”

* 任一步骤不符合预期，立即 fail

**FR-RUN-03**：支持提取/读取本次运行的 worklog

* 来源可以是：文件落盘、日志系统导出、或 worklog API（见第9章）

**FR-RUN-04**：worklog 对比（与 baseline）

* 允许对不稳定字段做归一化（时间戳、随机ID等）
* 输出人类可读 diff（定位到具体 step/event）

实现方式:OpenDAN本身提供 kapi/post_object HTTP POST接口 

---

## 8. 确定性要求

为确保“回放可重复、结果稳定”，系统必须满足：

1. **KV Key 生成确定性**

   * LLM Request 必须经过规范化（canonicalization）再作为 key
   * 禁止把时间戳、随机ID、动态排序字段直接纳入 key（或需归一化）

2. **测试运行过程避免随机性**

   * 随机数：固定 seed 或替换为可注入 deterministic provider
   * 时间：注入可控 clock 或在日志对比时忽略时间字段
   * 并发：对 worklog 事件顺序要可稳定比对（见 9.4）

3. **Mock AICC 不允许“模糊命中”默认通过**

   * 未命中必须失败（否则会掩盖框架问题）

---

## 9. 接口与数据结构规范（建议稿）

> 说明：以下为“需求层面的建议接口”。若现有系统已有接口，实现可映射，但必须满足等价能力。

### 9.1 Agent Message HTTP API（建议）

* `POST /kapi/post_object`
* Request（示例）：

```json
{
  "agent_id": "planner_agent",
  "session-id": "case-001",
  "objects" :[
    "msg_object": {
    
    }
  ]

}
```


### 9.3 KV 剧本文件格式（建议）

建议使用 JSON Lines 或 JSON，便于 diff 与合并。

**llm_kv.json（示例结构）**：

```json
{
  "case_id": "opendan_it_case_001",
  "version": 1,
  "entries": [
    {
      "request": { ... },
      "response": { ... }
    }
  ]
}
```

**Key 生成规则建议**：

* 对 request 做 canonical JSON（排序key、统一浮点/空值、去除无关字段）
* 再做 hash（sha256）作为稳定 key
* 同时保留 request_canonical 便于排查“为什么 miss”

### 9.4 Worklog 格式与对比规则（建议）


**对比规则**：

* 默认按 `seq` 对比（若并发导致 seq 不稳定，则引入 correlation_id 并按稳定规则排序）
* 忽略字段：timestamp、运行环境路径、随机ID（可配置 ignore list）
* 对 payload 做结构化 diff（而非纯文本 diff）

---

## 10. 用例结构与执行约定

### 10.1 用例目录结构（建议）

```
tests/
  integration/
    cases/
      case_001_xxx/
        case.yaml                  # 用例定义：消息步骤 + 预期回复
        llm_kv.json                # Mock AICC 回放剧本
        worklog_expected.json      # baseline
        README.md                  # 用例说明、更新方式
```

### 10.2 case.yaml（建议）

```yaml
case_id: opendan_it_case_001
description: "验证 planner_agent 在 mock LLM 下的完整 loop"
steps:
  - id: step-01
    send:
      agent_id: planner_agent
      message: "你好"
    expect:
      reply_contains: "你好"
  - id: step-02
    send:
      agent_id: planner_agent
      message: "帮我做一个TODO列表"
    expect:
      reply_contains: "TODO"
worklog:
  expected: worklog_expected.json
llm_script:
  kv: llm_kv.json
```

---

## 11. 错误处理与诊断输出要求

当测试失败时必须提供可定位信息：

1. **LLM KV 未命中（Mock AICC miss）**

   * 输出：case_id、step_id、request_key、request_canonical
   * 输出：最相近的若干 key（可选，用于快速判断是哪里不一致）

2. **回复断言失败**

   * 输出：期望、实际、上下文 step 信息

3. **worklog diff**

   * 输出：第一个不一致 event 的位置
   * 输出：event type、payload diff
   * 支持导出完整 diff 报告文件

---

## 12. CI/CD 集成要求

* **FR-CI-01**：回归测试默认走 Replay 模式

  * 不依赖外部真实 LLM
  * 可在离线/内网环境稳定运行

* **FR-CI-02**：提供 Record 模式但不在每次 CI 默认执行

  * Record 用于更新 baseline（人工触发或特定 pipeline）

* **FR-CI-03**：对执行时间设定目标

  * 单用例执行应为“秒级到几十秒级”（取决于 agent loop 复杂度）
  * 全量回归可控在可接受时长（具体阈值由项目设定）

---

## 13. 风险与对策

1. **worklog 含大量不稳定字段导致频繁更新 baseline**

   * 对策：定义 ignore/normalize 规则；只对“关键语义事件”做断言

2. **LLM Request 规范化不一致导致 KV 频繁 miss**

   * 对策：key 基于强类型结构生成；统一 canonicalization；保留 request_canonical 辅助排查

3. **并发/异步导致事件顺序不稳定**

   * 对策：引入 correlation_id；对比时按稳定排序策略；或在测试模式下限制并发

4. **系统版本升级导致类型变化**

   * 对策：类型覆盖 UT 作为“前置闸门”；一旦类型变更，提示更新脚本生成器与 baseline

---

## 14. 验收标准（Definition of Done）

满足以下条件视为交付达标：

1. 能在本地与 CI 中以 Replay 模式稳定运行至少 N 个典型用例（N 由项目设定）
2. 用例执行过程中不触发真实 LLM 请求（成本为 0 或可控）
3. 每个用例同时具备：

   * 消息步骤输入
   * 预期回复断言
   * worklog baseline 对比
   * Mock AICC KV 剧本
4. 当失败发生时，能输出可定位的差异信息（KV miss / reply mismatch / worklog diff）
5. 类型覆盖 UT 能覆盖关键结构的序列化/反序列化，并能在类型变更时有效报警

---

## 15. 最小可行版本（MVP）建议拆解

为快速落地，建议按以下顺序实现：

1. **Mock AICC + KV 加载/命中/失败诊断**
2. **Runner：HTTP 发消息 + 断言回复**
3. **worklog 采集与对比（先忽略时间戳等字段）**
4. **Record 模式：跑一遍并保存 worklog baseline + KV**
5. **构造强类型剧本生成器：从强类型输入渲染请求/解码输出，减少手写复杂脚本** （这是未来测试开发的主战场)

