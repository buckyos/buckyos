# s4_city_four_meanings — S4 同样是“上海”：查询参数、临时所在地、常住地、默认天气城市

Activation: decision `acted`, outcome `completed`, 1 runs, 42423 tokens

Rounds: 3

## Expected

- m1 只是查询参数：丢弃，不形成用户属性。
- m2 至多是带截止时间的当前位置线索：volatile，valid_until 约为 2026-11-06（周五），不是常住地。也可丢弃（没有跨 Session 用途时）。
- m3 形成常住地属性：basis=user_statement，说明不代表当前所在地；不强制 stable。
- m4 形成限天气查询的持续偏好：explicit=true，范围是天气查询，不是所在地或时区。
- m3 与 m4 是两条不同用途的认知，不合并成“用户在上海”。

Must not appear:

- 查询参数说明用户在上海
- 常住地等于当前所在地
- 由任一句推导日程时区
- 所有地点统一给半年有效期

Next task: 问天气时默认查上海；安排日程时不据此推导时区；周五之后不再把“在上海”当作当前位置。

## Materials

- ✓ **m1** → `discarded` — 上海是这一次天气查询的参数，不能单凭此推断常住地或长期默认偏好。
  > 用户问：“查一下上海天气。”已回答。
- ✓ **m2** → `discarded` — 2026-11-03 所说的周五为 2026-11-06；临时在上海的时间范围已过去，不能用于判断 2026-11-09 当前所在地，也没有跨任务保留的必要。
  > 用户说：“我现在在上海，周五离开。”（说这句话时是 2026-11-03 周二）
- ✓ **m3** → `absorbed` ["item_alice_residence_shanghai"]
  > 用户说：“我长期住在上海。”
- ✓ **m4** → `absorbed` ["item_alice_weather_default_shanghai"]
  > 用户说：“以后问天气默认查上海。”

## Resulting list

- `item_alice_residence_shanghai@1` [active, new] basis=user_statement explicit=false stability=stable weight=0.6 confidence=1
  scope={"subjects":["did:bns:alice"]} tags=["常住地", "上海"] last_evidence=2026-11-03T03:00:00Z
  > 用户长期住在上海；需要常住地背景时可据此理解，但不能据此判断用户此刻的位置。
- `item_alice_weather_default_shanghai@1` [active, new] basis=user_statement explicit=true stability=stable weight=0.7 confidence=1
  scope={"subjects":["did:bns:alice"]} tags=["天气", "默认城市", "上海"] last_evidence=2026-11-03T04:00:00Z
  > 用户问天气而未指定地点时，默认查询上海；这项默认仅适用于天气查询，不代表用户当前所在地。

## Observations added

- `obs_alice_residence_shanghai`: 用户说：“我长期住在上海。”原始事件发生于 2026-11-03T03:00:00Z。
- `obs_alice_weather_default_shanghai`: 用户说：“以后问天气默认查上海。”原始事件发生于 2026-11-03T04:00:00Z。

Commit summary: 分别保留用户的上海常住地与天气默认城市偏好；丢弃单次天气查询及已过期的临时所在地信息。

Goal result: {"decision":"acted","distance":"全部材料已处理；无待处理材料或到期复核项。","pending":[]}

## Mechanical checks

all passed (the list above still needs reading)
