# What a conversation perceives

## city_query — S4：查询参数

Said: 上海明天会下雨吗？不用真的去查，告诉我你打算怎么查就行。

Expected: none — 城市只是这次问题的参数，不是用户属性。

Perceived (0):


✓
## city_stay — S4：临时所在地

Said: 顺便说一下，我现在在上海，周五离开。

Expected: some — 有截止时间的停留：保留“到周五”，不写成常住。

Perceived (1):

- 用户于2026年11月9日说现在在上海，周五（2026年11月13日）离开；这是临时停留，不代表长期居住地。
  tags=["上海"] objects=[] suggested=Some("fact") intent=None cites=[] occurred_at=None

✓
## city_home — S4：常住地／C7

Said: 对了，我长期住在上海。

Expected: some — 用户自述常住地：保留“长期居住”的原意。

Perceived (1):

- 用户说自己长期住在上海；这是长期居住地，不一定代表实时所在地。
  tags=["上海"] objects=[] suggested=Some("fact") intent=None cites=[] occurred_at=None

✓
## city_default — S4：默认天气城市／P1

Said: 以后我问天气，默认查上海。

Expected: some — 限天气查询的持续偏好，不是所在地。

Perceived (1):

- 用户要求：以后询问天气时，默认查询上海。这是天气查询的默认地点偏好，不代表用户居住地或实时位置。
  tags=["weather", "shanghai"] objects=[] suggested=Some("preference") intent=None cites=[] occurred_at=None

✓
## exact_url — C14／§4.1.1 精确原文

Said: 项目 A 的测试入口是 https://staging.proj-a.example.com:8443/api/v2?tenant=qa ，别和生产搞混；以后我说“测试入口”就是它。

Expected: some — 完整 URL（端口、路径、参数）与用途都要能恢复。

Perceived (1):

- 用户明确指定：项目 A 的测试入口是 https://staging.proj-a.example.com:8443/api/v2?tenant=qa ，不要与生产环境混淆；以后用户说“测试入口”即指该完整 URL。此别名适用于项目 A。
  tags=["项目 a", "测试入口"] objects=[] suggested=Some("fact") intent=Some("explicit") cites=[] occurred_at=None

✓
## owner_evaluation — C13／P2

Said: 供应商 X 上次在项目 A 交付拖了两周，我对他们的时效不太放心。

Expected: some — 保留评价者（用户）、对象、理由和经历，不写成“X 不可靠”。

Perceived (1):

- 用户表示：供应商 X 上次在项目 A 的交付拖了两周，因此用户对其交付时效不太放心。这是用户基于该次项目经历的判断，不代表供应商在所有项目中都不可靠。
  tags=["供应商 x", "项目 a"] objects=[] suggested=Some("fact") intent=None cites=[] occurred_at=None

✓
## anomaly_unknown_cause — P4：原因未明的异常

Said: 记一下刚才的情况：你用 tool-x v1 在 Linux 上生成报告失败了两次，加上 --out build/ 之后才成功。到底是不是只和输出目录有关，我们还没查清。

Expected: some — 条件、行动、结果都在；原因未查清要写成未知，不能漏记也不能编原因。

Perceived (1):

- 用户要求记住此前的工具经历：在 Linux 上用 tool-x v1 生成报告失败了两次，加上 --out build/ 后才成功。这是用户对先前情况的陈述；是否仅与输出目录有关尚未查清，不能据此认定输出目录是唯一原因。
  tags=["tool-x"] objects=[] suggested=Some("experience") intent=Some("explicit") cites=[] occurred_at=None

✓
## lasting_requirement — S5／C5／P1

Said: 以后项目 A 的周报控制在一页以内，明细放附件。

Expected: some — 明确的持续要求：范围是项目 A 的周报。

Perceived (1):

- 用户要求：今后项目 A 的周报控制在一页以内，明细放附件。范围为项目 A 的后续周报，持续适用，直至用户更改。
  tags=["项目 a", "周报"] objects=[] suggested=Some("preference") intent=None cites=[] occurred_at=None

✓
## task_parameter — S5：本次例外

Said: 这次的回复写详细一点：用三四句话介绍一下你自己。

Expected: none — 只改变本次交付。

Perceived (0):


✓
## greeting — §4.1 不捕获

Said: 你好，你都能做些什么？

Expected: none — 问候与能力自述不是材料。

Perceived (0):


✓
