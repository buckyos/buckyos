

# BuckyOS 设置向导增量改造 PRD

## 主题：新增 AI Provider 设置页 & Jarvis Msg Tunnel 设置页

**文档类型**：增量需求 PRD
**适用范围**：现有 BuckyOS Setup Wizard
**设计原则**：沿用现有设置向导 UI 风格与交互组件，不新增独立视觉规范

---

## 1. 背景

当前 BuckyOS 设置向导已覆盖基础系统初始化，但以下能力仍依赖用户进入系统后在控制面板中完成配置：

1. 系统 AI 相关能力所需的 AI Provider 配置
2. Jarvis 的消息通道配置（当前为 Telegram Tunnel）

这会导致用户完成安装后，仍不能立刻使用 AI / Jarvis 相关功能，首次可用路径较长。

本次需求目标是在 **不破坏向导简洁性** 的前提下，在现有向导中新增两个“**快速配置页**”，帮助用户在首次启动时完成基础接入；高级配置仍留在系统控制面板中完成。

---

## 2. 目标

### 2.1 产品目标

* 让用户在设置向导内完成主流 AI Provider 的快速接入。
* 支持通过 **SN Active Code** 自动获得 AI Provider 配置能力。
* 让用户在设置向导内完成 Jarvis Telegram Tunnel 的基础配置。
* 通过教程链接降低用户获取 Token 的门槛。
* 保持现有向导“轻量、低干扰、快速完成”的定位。

### 2.2 非目标

本次不做以下内容：

* 不做完整的 AI Provider 管理页。
* 不支持自定义 AI Provider 配置。
* 不支持本地 LLM 配置。
* 不在向导中提供模型选择、优先级排序、高级参数配置。
* 不在向导中实现 Telegram 之外的 Msg Tunnel 实装（仅预留后续扩展结构）。
* 不在向导中做复杂连通性测试或深度校验。

---

## 3. 改造范围

在现有 Setup Wizard 中新增两个步骤：

1. **AI Provider 设置页**
2. **Jarvis Msg Tunnel 设置页**

建议插入位置：

**现有基础配置步骤** → **AI Provider 设置** → **Jarvis Msg Tunnel 设置** → **完成页**

### 插入位置原因

* AI 与 Jarvis 均属于增强能力，不应阻塞系统基础初始化。
* 放在完成页之前，用户感知更自然，且不会影响前置关键配置流程。
* 两个新增步骤均可设计为“可跳过”，符合向导简洁原则。

---

## 4. 用户故事

### 4.1 AI Provider

* 作为一个已有 OpenAI / Claude / Google / OpenRouter / GLM API Token 的用户，我希望在首次设置时直接填写，以便系统完成后即可使用 AI 功能。
* 作为一个已经购买 SN 服务的用户，我希望系统识别到我的 SN Active Code，并提示我相关 AI Provider 已可自动配置。
* 作为一个暂时不打算配置 AI 的用户，我希望可以跳过，但明确知道跳过会影响什么功能。

### 4.2 Jarvis Msg Tunnel

* 作为一个希望通过 Telegram 使用 Jarvis 的用户，我希望在首次设置时直接配置 Telegram Tunnel。
* 作为一个暂时不打算配置消息通道的用户，我希望能够跳过，并在后续系统控制面板中继续配置。

---

## 5. 总体流程

### 5.1 向导主流程

```text
现有前序步骤
   ↓
AI Provider 设置页
   ↓
Jarvis Msg Tunnel 设置页
   ↓
完成页
```

### 5.2 AI Provider 页流程

```text
进入页面
   ↓
检查是否已存在 SN Active Code
   ├─ 是：展示提示“您的SN服务已经包含了自动设置的AI Provider”
   └─ 否：展示“填写 SN Active Code”入口（打开填写对话框）

用户可选择：
- 填写一个或多个 AI Provider 的 API Token
- 填写 SN Active Code
- 什么都不填

底部按钮逻辑：
- 任一 Provider 已填写，或存在 Active Code → 按钮为“下一步”
- 一个 Provider 都未填写，且没有 Active Code → 按钮为“跳过”

点击“跳过”
   ↓
弹出确认提示
   ↓
确认后进入 Jarvis Msg Tunnel 设置页
```

### 5.3 Jarvis Msg Tunnel 页流程

```text
进入页面
   ↓
当前仅展示 Telegram Tunnel 配置区
   ↓
用户可选择：
- 填写 bot_api_token + telegram_account_id
- 不填写，直接跳过

底部按钮逻辑：
- 两个字段都为空 → 按钮为“跳过”
- 两个字段都已填写 → 按钮为“下一步”
- 仅填写其中一个 → 按钮保持“下一步”但不可提交，提示补全
```

---

## 6. 详细需求

# 6.1 AI Provider 设置页

## 6.1.1 页面目标

帮助用户在首次向导中完成主流云端 AI Provider 的快速接入，同时保留通过 SN Active Code 自动配置的路径。

## 6.1.2 页面结构

页面沿用现有向导样式，采用单页表单结构，建议包含以下区域：

### A. 页面标题

**AI Provider 设置**

### B. 页面说明文案

建议文案：

> 连接常用 AI 服务以启用系统 AI 功能。
> 为保持设置向导简洁，这里只提供快速接入；自定义 AI Provider 和本地 LLM 请在系统控制面板中配置。

### C. 教程入口

页面头部或说明文案下方提供教程链接：

**链接文案建议**：

* 如何获取 API Token

**行为要求**：

* 点击后打开外部浏览器/新窗口
* 不应丢失当前向导已填写内容
* 链接地址应可配置，不写死在前端代码中

### D. Provider 列表

展示主流 Provider 的快速输入项：

* OpenAI
* Claude
* Google
* OpenRouter
* GLM

每个 Provider 仅提供一个基础输入框：

**字段名建议**：`API Token`
**输入形式**：密码态/可切换显示
**说明**：全部为可选项，可填写一个或多个

### E. SN Active Code 区域

页面中提供 SN Active Code 相关区域，逻辑如下：

#### 情况 1：系统已存在 SN Active Code

展示一行提示文案：

> 您的 SN 服务已经包含了自动设置的 AI Provider。

可使用信息提示条/说明行样式呈现。
此时不再展示“填写 SN Active Code”入口。

#### 情况 2：系统不存在 SN Active Code

展示一个入口按钮/文字按钮：

**按钮文案建议**：

* 填写 SN Active Code

点击后打开对话框。

---

## 6.1.3 SN Active Code 对话框

### 对话框标题

**填写 SN Active Code**

### 内容

* 输入框：SN Active Code
* 按钮：取消 / 确认

### 交互逻辑

* 用户点击“确认”后，调用现有 SN Active Code 处理逻辑
* 成功后：

  * 对话框关闭
  * 页面显示成功状态
  * SN 区域切换为提示文案：

    > 您的 SN 服务已经包含了自动设置的 AI Provider。
* 失败后：

  * 对话框不关闭
  * 在输入框下方显示错误提示
  * 不阻塞用户继续手动填写 Provider，或返回后跳过本页

### 说明

本页不新增 SN 服务专用流程，只复用现有 Active Code 能力；若现有实现需要在线校验，则失败时不应卡死整个向导。

---

## 6.1.4 页面状态与按钮逻辑

| 状态                                | 页面表现                                   | 主按钮文案 | 点击结果                   |
| --------------------------------- | -------------------------------------- | ----- | ---------------------- |
| 无任何 Provider Token，且无 Active Code | Provider 输入框为空；显示“填写 SN Active Code”入口 | 跳过    | 弹出确认提示                 |
| 填写了任一 Provider Token              | 保存已填 Token                             | 下一步   | 进入 Jarvis Msg Tunnel 页 |
| 已存在 Active Code                   | 展示 Active Code 提示文案                    | 下一步   | 进入 Jarvis Msg Tunnel 页 |
| 同时存在 Active Code + 手动填写 Token     | 两者都保留                                  | 下一步   | 保存并进入下一页               |

---

## 6.1.5 “跳过”确认逻辑

当满足以下条件时：

* 一个 Provider 都未填写
* 且没有 Active Code

底部主按钮显示为 **“跳过”**。

点击后弹出确认框。

### 确认框文案建议

**标题**：跳过 AI Provider 设置？

**正文**：

> 不设置将无法启用系统 AI 相关功能。
> 你也可以稍后在系统控制面板中填写。
> 自定义 AI Provider 和本地 LLM 配置均在系统控制面板中完成。

**按钮建议**：

* 返回填写
* 仍然跳过

确认跳过后进入下一步。

---

## 6.1.6 校验规则

为保证向导简洁，本页仅做轻量校验：

* Token 输入去除首尾空格
* 非空即可保存
* 不做各 Provider 的强格式校验
* 不在向导内做阻塞式连通性验证

### 原因

* 各家 Token 格式可能变化
* 向导不宜因外部网络/第三方服务状态而提高失败率
* 复杂校验应交由系统控制面板完成

---

## 6.1.7 存储与安全要求

* 所有 Token 均使用现有安全存储机制保存
* 输入框默认遮罩显示
* 明文 Token 不写入日志
* 页面返回/前进时，已填写内容应保留

---

# 6.2 Jarvis Msg Tunnel 设置页

## 6.2.1 页面目标

帮助用户在首次设置中完成 Jarvis 的基础消息通道配置；当前仅支持 Telegram Tunnel，同时为未来新增其他 Msg Tunnel 预留结构。

## 6.2.2 页面结构

建议页面采用“**通道卡片/模块化区域**”结构，便于后续扩展更多 Msg Tunnel。

### A. 页面标题

**Jarvis Msg Tunnel 设置**

### B. 页面说明文案

建议文案：

> 配置 Jarvis 的消息通道。当前仅支持 Telegram Tunnel，后续版本将支持更多 Msg Tunnel。

### C. 教程入口

页面头部提供教程链接。

**链接文案建议**：

* 如何获取 Telegram Bot API Token

同时建议在 `telegram_account_id` 字段附近增加一个辅助链接：

* 如何获取 Telegram Account ID

### D. 当前支持的 Tunnel

当前页面仅展示一个配置模块：

**Telegram Tunnel**

包含两个字段：

1. **Bot API Token**
2. **Telegram Account ID**

---

## 6.2.3 Telegram Tunnel 字段定义

### 字段 1：Bot API Token

* 类型：输入框
* 形式：密码态/可切换显示
* 必填条件：与 Account ID 成对出现；若配置 Telegram Tunnel，则必填

### 字段 2：Telegram Account ID

* 类型：输入框
* 形式：普通文本
* 必填条件：与 Bot API Token 成对出现；若配置 Telegram Tunnel，则必填

---

## 6.2.4 页面状态与按钮逻辑

| 状态        | 主按钮文案     | 行为                          |
| --------- | --------- | --------------------------- |
| 两个字段都为空   | 跳过        | 直接进入下一步/完成页                 |
| 两个字段都已填写  | 下一步       | 保存 Telegram Tunnel 配置并进入下一步 |
| 仅填写其中一个字段 | 下一步（不可提交） | 提示用户补全缺失字段                  |

### 缺失字段提示文案建议

> 请完整填写 Telegram Bot API Token 和 Telegram Account ID，或清空后跳过本页。

---

## 6.2.5 页面交互原则

* 当前只展示 Telegram Tunnel，但布局需支持未来继续追加：

  * Discord
  * Slack
  * Email
  * 其他自定义消息通道
* 本次不要求支持多 Tunnel 同时配置的复杂逻辑
* 本次仅保存 Telegram 的基础字段，不做深度联调

---

## 6.2.6 校验规则

* 去除首尾空格
* 两个字段需成对填写
* 不在向导中做 Telegram 在线连通性强校验
* 保存失败时给出轻提示，不阻塞用户返回修改

---

## 6.2.7 跳过逻辑

本页建议允许跳过，以保持向导轻量。

### 建议行为

* 两个字段都为空时，主按钮显示“跳过”
* 点击后直接进入下一步，不强制二次确认

### 原因

* Msg Tunnel 并非系统启动的必需项
* Telegram 只是当前支持的第一种通道
* 后续仍可在系统控制面板中补配

---

## 6.2.8 说明文案建议

页面底部可补充一行说明：

> 当前向导仅提供 Telegram Tunnel 的快速配置，更多 Msg Tunnel 将在后续版本支持；完整配置可在系统控制面板中完成。

---

# 6.3 两个新增页面的公共要求

## 6.3.1 UI/交互继承

* 继承现有设置向导的页面布局
* 继承现有按钮区样式（上一步 / 下一步 / 跳过）
* 继承现有输入框、弹窗、提示条、错误提示组件
* 不新增独立主题或视觉体系

## 6.3.2 教程链接行为

* 以外链方式打开，不中断当前向导状态
* 教程链接统一支持配置化管理
* 链接点击应做埋点统计

## 6.3.3 数据持久化

* 用户返回上一步或从教程页面返回时，不应丢失已填内容
* 页面重新进入时，应回显已保存状态

## 6.3.4 安全要求

* Token 类字段默认遮罩
* 不在日志、埋点、错误信息中输出 Token 明文
* 敏感配置沿用现有安全存储方案

---

## 7. 推荐页面文案汇总

### 7.1 AI Provider 页

**标题**
AI Provider 设置

**说明文案**
连接常用 AI 服务以启用系统 AI 功能。为保持设置向导简洁，这里只提供快速接入；自定义 AI Provider 和本地 LLM 请在系统控制面板中配置。

**教程链接**
如何获取 API Token

**SN 提示文案**
您的 SN 服务已经包含了自动设置的 AI Provider。

**SN 按钮文案**
填写 SN Active Code

**跳过确认文案**
不设置将无法启用系统 AI 相关功能。你也可以稍后在系统控制面板中填写。自定义 AI Provider 和本地 LLM 配置均在系统控制面板中完成。

---

### 7.2 Jarvis Msg Tunnel 页

**标题**
Jarvis Msg Tunnel 设置

**说明文案**
配置 Jarvis 的消息通道。当前仅支持 Telegram Tunnel，后续版本将支持更多 Msg Tunnel。

**教程链接**
如何获取 Telegram Bot API Token
如何获取 Telegram Account ID

**字段提示**

* Bot API Token
* Telegram Account ID

**缺失项提示**
请完整填写 Telegram Bot API Token 和 Telegram Account ID，或清空后跳过本页。

**底部说明**
当前向导仅提供 Telegram Tunnel 的快速配置，完整配置可在系统控制面板中完成。

---

## 8. 边界场景

### 8.1 AI Provider 相关

1. **用户同时填写多个 Provider Token**

   * 全部保存
   * 不在本页新增“默认 Provider”选择逻辑

2. **用户已有 Active Code，又手动填写 Provider Token**

   * 两类配置均保留
   * Provider 的实际启用优先级沿用现有系统逻辑

3. **用户点开教程后返回**

   * 已输入内容保持不丢失

4. **SN Active Code 校验失败**

   * 显示错误提示
   * 不阻塞用户手动填写 Provider 或跳过本页

### 8.2 Jarvis 相关

1. **只填了 Bot API Token，未填 Account ID**

   * 不允许提交
   * 提示补全

2. **只填了 Account ID，未填 Bot API Token**

   * 不允许提交
   * 提示补全

3. **两个字段都为空**

   * 可直接跳过

4. **教程打开后返回**

   * 已输入内容保持不丢失

---

## 9. 埋点建议

建议新增以下埋点，便于后续评估新增页面的实际使用率：

### AI Provider 页

* 页面曝光
* 教程链接点击
* 任一 Provider Token 填写
* SN Active Code 对话框打开
* SN Active Code 提交成功/失败
* 点击跳过
* 跳过确认成功

### Jarvis Msg Tunnel 页

* 页面曝光
* Telegram 教程链接点击
* Bot API Token 填写
* Telegram Account ID 填写
* 提交成功
* 点击跳过

---

## 10. 验收标准

### AI Provider 页验收

1. 向导中新增 AI Provider 设置页。
2. 页面展示 OpenAI、Claude、Google、OpenRouter、GLM 五个 Provider 的 Token 输入项。
3. 页面提供“如何获取 API Token”教程入口。
4. 当系统已存在 SN Active Code 时，页面展示提示文案“您的 SN 服务已经包含了自动设置的 AI Provider”。
5. 当系统不存在 SN Active Code 时，页面展示“填写 SN Active Code”入口，并可打开填写对话框。
6. 当未填写任何 Provider，且无 Active Code 时，主按钮显示为“跳过”。
7. 点击“跳过”后弹出确认提示，文案符合需求。
8. 当填写任一 Provider 或存在 Active Code 时，主按钮显示为“下一步”。
9. Token 内容可保存且不明文暴露。
10. 页面遵循现有设置向导 UI 风格。

### Jarvis Msg Tunnel 页验收

1. 向导中新增 Jarvis Msg Tunnel 设置页。
2. 当前页仅展示 Telegram Tunnel 配置项。
3. 页面至少包含 Bot API Token 与 Telegram Account ID 两个字段。
4. 页面提供获取教程链接。
5. 当两个字段都为空时，主按钮显示“跳过”。
6. 当两个字段都填写时，主按钮显示“下一步”，并可正常进入下一步。
7. 当仅填写其中一个字段时，不允许提交，并给出补全提示。
8. 页面结构可扩展到未来更多 Msg Tunnel。

