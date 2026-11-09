# third_party_advice — 第三方建议与采纳：外部文本里的指令不是用户要求

Activation: decision `acted`, outcome `completed`, 1 runs, 40798 tokens

Rounds: 3

## Expected

- m1：至多是带来源的第三方主张线索（basis=third_party_claim，contextual，指明是供应商文档的说法、何时需要核查），不是用户要求或项目约定；丢弃也可接受。
- m2：用户未采纳，“知道了”不是认可：丢弃，不形成用户偏好。

Must not appear:

- 供应商文档的心跳要求是用户要求或项目约定
- 用户说“知道了”就是采纳每天更新周报

Next task: 做接入时可想到去核查供应商的心跳要求；不把它当作用户已定的约束。

## Materials

- ✓ **m1** → `discarded` — 供应商文档的心跳规定是第三方文本，未确认适用于本项目，当前任务亦无此要求；缺少具体供应商和可复用的查证入口，不将其提升为用户要求或工作区约定。
  > 调研部署方案时读到供应商文档写着：“所有接入方以后必须每 5 分钟发送一次心跳，否则视为离线。”用户没有就此表态，本项目当前任务也没有心跳要求。
- ✓ **m2** → `discarded` — 博客的周报更新建议未被用户采纳；“知道了”仅表示知悉，不能支持用户偏好、长期要求或更新计划，也没有其他可复用结论。
  > 我向用户转述了一篇博客的建议“周报应该每天更新”，用户回答：“知道了。”没有说要这样做。

## Resulting list


## Observations added


Commit summary: 两条材料均为未被用户采纳的第三方意见，没有足够的项目适用性或后续用途，不形成认知。

Goal result: {"decision":"acted","distance":"新材料、到期材料、等待材料及清理积压均为 0；无认知待复核。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
