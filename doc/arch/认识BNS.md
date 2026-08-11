# 认识 BNS

BNS 是 BuckyOS 的去中心名字系统。更准确地说，它不是一个“把名字映射到地址”的系统，而是 BuckyOS 的协议级身份与信任入口。

理解 BNS，不能从“名字怎么解析”开始，也不能从“链上存什么字段”开始。第一步应该先看今天的互联网在几个基础场景里缺了什么：

1. 普通人很难不依赖商业公司，在互联网上架设一个自己的、可信的、公网可访问的集群。
2. 普通人很难拥有一个不依赖平台账号的 global user profile，并把关键启动配置长期可靠地放在互联网上。
3. 内容创作者、消费者和索引者之间缺少一套不依赖单个平台的名字、信用、支付和分润协议。

这三个问题看起来不同，但背后其实是同一个缺口：互联网缺少一种协议级名字，让一个人、一个集群、一个内容、一个 App、一个组织，既能被人类稳定引用，又能被机器验证控制权、当前状态和经济归属。

BNS 要填的就是这个缺口。

## 1. 普通人需要自己的去中心公网集群

今天，一个普通人如果想在互联网上架设自己的家庭云、个人服务器或小团队集群，通常会被迫组合很多系统：

- 买域名。
- 配 DNS。
- 配 HTTPS 证书。
- 做端口映射、DDNS、内网穿透或云中转。
- 使用厂商账号绑定设备。
- 手工维护每台设备的访问权限、密钥和信任关系。

这些方案能工作，但它们不是一个完整的去中心协议。

DNS 和 HTTPS 主要解决“访问某个域名时，服务端是否匹配这个域名”。它们没有自然解决“客户端设备是谁”“这台设备是否属于同一个 owner”“这个服务是否由我的集群授权发布”这些问题。

厂商账号和云中转可以解决一部分易用性，但代价是控制权回到商业公司手里。用户表面上拥有设备，实际上很多关键能力依赖平台账号、平台服务器和平台策略。

BuckyOS 想让普通人拥有的是另一种东西：

```text
username
  -> global owner
  -> personal zone / cluster
  -> devices / services / apps
  -> public access with verifiable trust
```

也就是说，用户可以拥有一个自己的名字，例如：

```text
did:bns:alice
```

这个名字不是主页地址，也不是某个服务器 IP。它代表 Alice 这个 owner 在协议里的长期身份。Alice 可以在这个身份下面运行自己的 Zone，Zone 里可以有 OOD、家庭 NAS、手机、笔记本、摄像头、Agent、应用服务和公网 gateway。

当 Alice 添加一台新设备时，设备不是单纯获得一个局域网 IP，也不是只登记到某个厂商云账号。它会获得自己的设备身份，并进入 Alice 的 owner 信任域。

这带来一个关键变化：设备与设备之间可以基于共同 owner 建立基础信任。

```text
device A
  -> proves it belongs to owner Alice

device B
  -> proves it belongs to owner Alice

then:
  A and B can establish a trusted channel under Alice's zone policy
```

这件事是今天 HTTPS 没有完成的部分。HTTPS 让客户端相信自己正在访问某个服务端域名，但客户端身份通常要交给应用层登录、cookie、token、OAuth 或厂商账号来解决。BuckyOS 希望把设备身份、owner 身份和集群信任关系下沉到协议层。

因此，在 BNS 支持下，一个服务名可以是：

```text
did:bns:smb.alice
did:bns:cam01.alice
did:bns:jarvis.alice
```

调用方解析这个名字时，得到的不是裸地址，而是可验证的服务描述、设备描述、Zone 描述和 owner 关系。真正负责连接的可以是 gateway、RTCP、SN、中继或其它网络组件，但连接之前和连接之后，协议都能知道“我连到的是谁”“它属于哪个 owner”“它是否有权提供这个服务”。

BNS 在这个场景里的价值不是替代所有网络技术，而是提供一层稳定的信任入口：

- 用户名是集群的全局入口。
- Owner 是集群的共同信任根。
- Device 有可验证身份。
- Service 有可验证发布关系。
- Endpoint 可以变化，但名字不变。
- 设备加入、迁移、替换时，不需要用户重新拼接一堆中心化账号和手工配置。

这就是普通人真正需要的去中心基础设施：不是“我有一台服务器”，而是“我有一个属于自己的、可扩展的、可信的互联网集群”。

## 2. 用户需要真正属于自己的 global profile


当一个用户需要把某种启动配置、恢复配置或 profile 长期可靠地放在互联网上时，他应该依赖谁？

自己的设备并不可靠。硬盘会坏，家庭网络会断，手机可能丢失。如果关键配置只保存在自己的某台机器上，那么“去中心”并没有真正完成。

商业平台也不应该成为最终答案。平台账号可以被封禁，平台服务可以下线，平台策略可以改变，平台数据库也不是协议的一部分。

BNS 的思路是：用户应该拥有一个基于区块链的 global user profile。

这个 profile 至少应该能回答：

1. 这个 username 当前由谁控制。
2. 当前 owner public key 是什么。
3. 当前默认 Zone、gateway、resolver 或 boot config 在哪里。
4. 如果 key 轮换、设备损坏、Zone 迁移，新的可信入口是什么。
5. 这个 profile 对应的收款地址或收益主体是什么。

这时，`did:bns:alice` 就不只是一个名字，而是一个真正意义上的账号。

它和传统账号的区别很大：

| 类型 | 控制权 | 是否可跨应用 | 是否可直接收钱 | 是否协议可验证 |
|---|---|---|---|---|
| 平台账号 | 平台 | 通常不能 | 依赖平台支付系统 | 主要由平台 API 证明 |
| 域名 | 注册商 / DNS / CA | 部分可以 | 不能天然表达 | 主要证明域名控制权 |
| 钱包地址 | 私钥持有人 | 可以 | 可以 | 可以验签，但缺少 profile 语义 |
| BNS profile | Owner / controller | 可以 | 可以 | 可以解析 DID、Owner、服务和状态 |

基于区块链，收钱是自然能力。一个 profile 可以直接成为经济主体：内容收益、App 收益、服务收益、Group 分润都可以归到这个名字背后的 owner 或合约规则上。

这就是为什么 BNS 的名字必须能表达控制权，而不能只是可读别名。一个 username 如果不能收钱、不能签发、不能恢复、不能被协议验证，它就仍然只是某个平台里的登录名。

从 BuckyOS Secure Boot 角度看，boot config 只是 global profile 的一个具体用途：

```text
new device
  -> knows or inputs did:bns:alice
  -> resolves Alice's global profile
  -> gets trusted owner key / boot config / zone entry
  -> verifies signatures
  -> joins the correct zone or loads the correct control plane
```

这个 boot config 可以用来启动设备，也可以用来启动一个 Agent、恢复一个 Zone、定位一个新的 OOD，或者让某个服务找到它应该信任的控制面。

关键点是：配置可以放在互联网上，但信任不交给托管者。任何服务都可以缓存、转发或加速这个配置，但客户端最终验证的是 BNS 返回的 owner、签名、版本和状态。

所以，BNS 在这个场景里提供的是：

- 不依赖公司账号的 global user profile。
- profile 到 owner 控制权的可验证绑定。
- profile 到当前 boot config / Zone 入口的可更新指针。
- key 轮换、吊销、恢复和迁移的协议空间。
- profile 作为收款主体和签发主体的能力。

这让 BuckyOS 里的账号不再只是“登录用的名字”，而是一个可被协议引用、可承载资产、可恢复、可迁移、可持续演化的数字主体。

## 3. 内容发布需要协调消费者、创作者和索引者

第三个场景是内容发布。

互联网内容生态里有三个基本角色：

1. 消费者：希望通过一个名字找到、购买和验证内容。
2. 创作者：希望发布内容、持续更新、建立信用并获得收入。
3. 索引者：希望发现、收录、推荐、排序、背书和加速内容，并从中获得收益。

今天这些关系大多被平台打包处理。平台提供账号、商品 ID、搜索、推荐、支付、分发、评论、信用和结算。这样做很方便，但问题是平台也自然拥有了最终控制权：

- 平台决定内容是否存在。
- 平台决定创作者身份如何展示。
- 平台决定消费者买到的到底是什么。
- 平台决定索引和推荐规则。
- 平台决定分润和结算规则。
- 平台也可能劫持、下架、替换或封禁。

去中心内容系统不能简单地说“用内容哈希就好了”。内容哈希只能回答“这份 bytes 有没有被篡改”，不能回答下面这些问题：

- 用户购买的是哪个长期内容？
- 这个内容当前最新版是什么？
- 作者是谁？
- 谁有权更新？
- 谁为这个内容背书？
- 哪个索引者帮助我发现了它？
- 支付之后，作者、协作者、索引者、推荐者如何分润？
- 后续升级是否还属于同一次购买？

BNS 的作用，是给内容一个长期语义名字，例如：

```text
did:bns:book1.alice
did:bns:jarvis.alice
did:bns:model1.team
```

消费者购买的是这个 `content_name`，不是某个一次性的 `content_id`。

```text
content_name
  -> current content meta / AppDoc / package meta
  -> concrete content_id / ObjId
  -> downloadable bytes
  -> hash verification
```

这样，名字和内容哈希各自负责清楚：

- BNS 名字负责长期语义、控制权、当前版本、支付关系和更新权。
- ObjId / Hash 负责具体内容不可篡改。

消费者从任何地方发现内容都可以：

- 搜索引擎。
- 应用商店。
- 社交推荐。
- 好友分享。
- 某个 Source。
- 某个社区 curator。

索引者可以提供非常重要的价值：

- 收录内容。
- 做搜索和排序。
- 做人工或自动审核。
- 给出信用评分。
- 推荐高质量内容。
- 提供下载加速。
- 帮助创作者触达用户。

但索引者不应该拥有内容名字的最终控制权。它可以推荐 `did:bns:jarvis.alice`，可以声明“我验证过这个 App”，可以参与分润，但它不能替 Alice 改掉 `jarvis.alice` 的 owner，不能伪造 AppDoc 签名，也不能把用户购买的长期内容名悄悄替换成另一个对象。

站在创作者角度，发布内容赚钱的基本流程应该是：

```text
1. 拥有一个 BNS profile，例如 did:bns:alice。
2. 为内容创建长期名字，例如 did:bns:jarvis.alice。
3. 生成内容 meta / AppDoc，写入版本、权限、价格、收益规则和具体 ObjId。
4. 用 owner key 或授权 signing key 签名。
5. 把内容本体放到 Repo、P2P 网络、镜像或任意下载源。
6. 把当前 meta 指针发布到 BNS / Source / indexer。
7. 用户通过标准支付合约购买 content_name。
8. 标准支付合约按规则给作者、团队、索引者、推荐者自动分润。
```

这里的支付不是外部附加功能，而是名字系统天然要协调的经济关系。

因为如果用户购买的是长期名字，那么 receipt、订阅、授权、升级资格和分润都应该绑定到这个名字，而不是绑定到某个单次下载链接。内容升级后，用户仍然持有对同一个 `content_name` 的权益；具体下载时，再用新的 `ObjId` 校验内容本体。

一个具体的购买流程可以是：

```text
1. Alice 发布一本电子书，内容名是 did:bns:book1.alice。
2. book1 的 content meta 写入：
   - content_name = did:bns:book1.alice
   - current_content_id = obj:...
   - price = 10 USDB
   - beneficiary = did:bns:alice
   - split_policy = 作者 80%，索引者 15%，推荐者 5%
3. Alice 用 owner key 或授权 content signing key 签名 content meta。
4. 某个索引者 indexer1 收录并推荐这本书，生成 index proof：
   - content_name = did:bns:book1.alice
   - indexer_did = did:bns:indexer1
   - recommendation_id = ...
5. Bob 从 indexer1 看到这本书，钱包先解析 did:bns:book1.alice：
   - 确认名字仍由 Alice 控制。
   - 验证 content meta 签名。
   - 确认价格、权益、分润规则和 current_content_id。
6. Bob 调用标准支付合约购买：
   - buyer = did:bns:bob
   - content_name = did:bns:book1.alice
   - indexer = did:bns:indexer1
   - recommendation_id = ...
   - amount = 10 USDB
7. 标准支付合约完成扣款和自动分润：
   - 8 token -> did:bns:alice
   - 1.5 token -> did:bns:indexer1
   - 0.5 token -> 推荐者
8. 合约生成 receipt：
   - receipt_id = ...
   - buyer = did:bns:bob
   - content_name = did:bns:book1.alice
   - rights = read / download / future_updates
   - paid_amount = 10 token
   - content_version_policy = current_and_future
9. Bob 从任意 Repo、镜像、P2P 节点下载 obj:...。
10. 客户端用 ObjId / Hash 校验下载内容，并用 receipt 证明 Bob 拥有 did:bns:book1.alice 的购买权益。
```

这个例子里，BNS 不负责扣款，也不负责托管电子书本体。BNS 负责把 `did:bns:book1.alice` 解析到可信的 owner、content meta、当前内容指针和收益主体；支付合约负责扣款、分润和 receipt；内容网络负责下载；客户端负责验签和校验 Hash。

支付合约的语义需要更严格：

- 购买操作必须是一次标准支付合约调用。钱包可以把 `approve / permit`、购买确认和下载引导包装成一个 UI 流程，但协议层真正产生购买权益的是 `purchase(content_name, amount, indexer, recommendation_id, ...)` 这类合约调用。单纯从钱包直接转账给作者，不能生成 BNS 可验证的 receipt，也不能触发索引者、推荐者和后续授权规则。
- `beneficiary` 是收益主体，不等于 `content_name` 的控制权 owner。合约结算时最终只能把 token 转到链上账号地址；`did:bns:*` 需要按 BNS / 支付合约规则解析成实际 `payment_target`。
- `content_name` 的当前权益拥有方可以把自己的收款目标设置成合约地址。比如 `book1.alice` 的当前 owner 可以把 `beneficiary` 或 `payment_target` 改成一个分账合约，由这个分账合约再按内部股东、团队成员、投资人或 DAO 规则分配收益。BNS 和标准支付合约只需要验证这个收款目标确实由当前 owner / controller 签发，不需要理解内部股东表。
- 收款目标改为分账合约只影响修改生效后的新购买。已经完成的购买 receipt、历史支付和当时的分润结果不应被后来修改重写。
- 手续费不能按固定金额写死。以 ERC20 / USDB 为例，标准 `transfer` 到合约地址通常不会执行收款合约代码，gas 消耗相对可预测；但实际 gas 仍取决于 token 合约实现、收款地址余额状态、是否有 hook、分润人数和当时链上 gas price。支付合约直接给多个地址分润时，gas 大致随收款方数量增长；如果内部股东很多，更适合让标准支付合约只转给一个分账合约，再由股东按需 claim。

这样三方权利边界是清楚的：

- Bob 买到的是长期内容名 `did:bns:book1.alice` 的权益，不是一次性下载 URL。
- Alice 保留内容名字的控制权和后续更新权。
- indexer1 因为帮助 Bob 发现内容而获得协议内可验证分润。
- 任何下载源都可以提供内容 bytes，但不能伪造 Bob 的购买权益，也不能把内容替换成另一个 Hash。

信用体系也不应该由 BNS 独占。BNS 不需要成为唯一的评分系统或内容商店。它只需要提供可验证事实：

- 这个名字当前由谁控制。
- 这份 meta 是谁签的。
- 当前版本指向哪个 ObjId。
- 哪个索引者收录或背书过。
- 支付 receipt 绑定哪个 content_name。
- 分润规则是什么。
- 历史版本和当前版本如何区分。

基于这些事实，不同社区、索引者、搜索引擎、curator、AI Agent 都可以建立自己的信用体系。信用可以多元，但底层事实必须可验证。

BNS 在这个场景里的价值，是把消费者、创作者和索引者的权利边界分开：

- 消费者拥有可验证购买对象和下载验证能力。
- 创作者拥有名字、更新权和收益权。
- 索引者拥有发现、推荐、背书、加速和参与分润的权利。

三者都重要，但任何一方都不应该通过中心化数据库垄断其它两方。

## 4. BNS 与 DID Object Protocol

Agent 需要一个稳定的世界抽象，但 BNS 不需要定义整个对象协议。

对 BNS 来说，名字很多时候只是一个可信起点。它要回答的是：

- 这个对象是谁。
- 当前由谁控制。
- 当前可信文档在哪里。
- 后续应该沿哪条验证链继续走。

DID Object Protocol 则是下一层：当 Agent 已经通过 BNS 找到一个可信对象后，如何读取对象属性、调用对象动作、订阅对象事件。

二者关系可以简化成：

```text
BNS name
  -> verified DID Document / current document
  -> DID Object Card (DID Document)
  -> DID Object Profile
  -> declared property / action / event
```

例如，Alice 家里的摄像头可以被表示为：

```text
did:bns:cam01.alice
```

BNS 负责让 Agent 确认 `cam01.alice` 当前确实属于 Alice 的 Zone，并得到可信的 DeviceDocument / ServiceInfo / DID Object Card。随后 DID Object Profile 再声明这个对象能做什么，例如：

- `battery` property。
- `query_clip` action。
- `low_battery` event。

于是 Agent 执行的是声明过的能力：

```text
read_property(object = did:bns:cam01.alice, property = "battery")
x_call(object = did:bns:cam01.alice, action = "query_clip", params = { ... })
subscribe_event(object = did:bns:cam01.alice, event = "low_battery")
```

真正重要的是，Agent 不是在操作一个任意 URL，而是在操作一个经过 BNS / DID 验证的对象。BNS 保证可信起点，DID Object Protocol 保证对象能力边界。

所以 DID Object Protocol 对 BNS 的要求并不复杂：

- 对象可以从 `did:bns:$name` 或 `did:dev:xxxx` 开始解析。
- 解析结果能给出可信 controller、service endpoint 和当前文档。
- endpoint 可以变化，但必须由当前文档授权。
- Agent 后续使用的 Profile / Trait / action / event 都建立在这个可信起点之上。

换句话说，BNS 不负责定义所有对象能力。它只负责让 Agent 从一个稳定名字出发，进入一条可验证的对象访问链。

## 5. 这些场景共同要求什么

上面三个场景，以及 Agent 对稳定世界抽象的要求，最终会收束到同一组能力。

### 5.1 稳定名字

用户、Zone、Device、Service、App、Content、Group 都需要长期名字。这个名字不能因为 IP 变化、设备替换、Zone 迁移、内容升级、key 轮换就失效。

这是所有上层关系能够持续存在的前提。

### 5.2 可验证控制权

名字必须能解析到 owner、controller 和 public key。否则它只能做 UI 展示，不能作为协议主体。

控制权是 BNS 和普通昵称、平台账号、搜索索引之间的根本区别。

### 5.3 当前状态指针

名字必须能指向当前 profile、Zone config、boot config、ServiceInfo、AppDoc 或 content meta。

去中心系统里的实体不是静态文件。它们会升级、迁移、恢复、吊销和转移。名字必须允许合法更新。

### 5.4 下级签发能力

一个 owner 需要能签发下级对象：

- Zone。
- Device。
- Service。
- App。
- Agent。
- User。
- Group。
- Content meta。

这使得一个 global profile 可以扩展成一个个人集群、一个内容体系、一个组织或一个应用生态。

从 X.509 熟练用户的视角看，这里最容易误解的一点是：BNS 不是在复刻 WebPKI 里少数公共 CA 给所有人签证书的模型。

在 X.509 里，`CA:TRUE`、Key Usage、Extended Key Usage、Name Constraints 等扩展通常用来回答“这张证书是不是 CA”“它能签什么”“它能被用于什么目的”。普通互联网用户大多数时候只是 end-entity certificate 的持有者：他能证明自己控制某个域名或某个服务，但通常不能自然地成为一个可被协议承认的签发者。

BNS 的判断相反：每个人都应该能成为自己名字空间里的 CA。

这不是说每个人都能替任意别人的名字签发身份，而是说只要 Alice 拥有 `did:bns:alice`，她就应该能在这个 owner 根下面签发有效的 Zone、Device、Service、Agent、Content、子账号和子名字。有效性的来源不是某个商业 CA 的背书，而是解析链能回到 Alice 当前可验证的 owner / controller 状态。

这样一来，过去在证书里表达“这个身份可以做什么”的一部分语义，就不必都塞进同一个身份本身。BNS 更倾向于为不同权限组合构造不同的子账号或子身份：

```text
did:bns:alice
  -> did:bns:zone.alice
  -> did:bns:cam01.alice
  -> did:bns:content-signer.alice
  -> did:bns:jarvis.alice
```

每个子身份只绑定它需要的权限、命名空间、文档类型、有效期和吊销规则。例如 `content-signer.alice` 可以只被允许签 content meta，`cam01.alice` 只代表一台设备，`jarvis.alice` 只代表一个 Agent。限制身份能做什么，不是把 `did:bns:alice` 这个最高身份变成一个越来越复杂的证书，而是把“权限组合”变成一个可独立验证、可独立吊销、可独立迁移的身份。

这和传统 X.509 的直觉不同：X.509 更常见的是围绕一张证书附加用途约束；BNS 更强调由 owner 持续签发一棵身份树。身份本身保持简单，复杂性被分散到 owner / controller 策略、子账号和解析验证链里。

### 5.5 内容哈希配合

BNS 不应该保存所有内容本体，也不应该替代内容寻址。

BNS 负责长期名字和当前指针，ObjId / Hash 负责具体内容校验。二者结合，才能同时支持“可升级”和“不可篡改”。

> git的分支名 是名字，分支指向的commit hash是内容Hash(ObjId)

### 5.6 支付和收益主体

如果名字代表真实的数字主体，它就必须能承载经济关系。

这包括：

- profile 收款。
- 内容购买。
- App 付费。
- 服务订阅。
- Group 收益。
- 自动分润。
- 索引者和推荐者收益。

基于区块链，支付不是额外插件，而是 BNS 名字成为真正账号和资产入口的自然结果。

### 5.7 历史状态与恢复

去中心系统不能只关心“当前是什么”。它还需要处理：

- 旧签名如何验证。
- key 泄露后如何吊销。
- 设备丢失后如何恢复。
- 内容转移后旧版本如何归属。
- Group 成员变化后历史收益如何解释。

因此，BNS 必须给版本、吊销、恢复和历史 proof 留出协议空间。

### 5.8 中心化服务可以存在，但不能拥有最终控制权

BNS 不是要消灭所有中心化服务。

DNS、HTTPS、Source、搜索引擎、应用商店、网关、缓存、镜像、SN、中继服务都可以存在，而且很多时候非常有用。

但它们的角色应该是发现、加速、缓存、推荐和连接，不应该成为最终信任根。最终控制权应该回到名字、owner、签名、合约状态和内容哈希。

## 6. BNS 的核心功能

基于这些场景，BNS 的核心功能可以归纳为七个。

### 6.1 注册和拥有名字

BNS 首先要允许用户注册、拥有和更新名字 owner。

名字可以代表：

- 用户。
- Zone。
- Device。
- Service。
- App。
- Content。
- Group。
- Agent。

高价值名字可以直接进入全局 BNS。大量 Zone 内名字可以由顶层 Zone 派生解析，以降低成本和提高扩展性。

### 6.2 解析到 DID Document / profile

BNS 必须能把名字解析到可验证文档，而不是只解析到地址。

文档里至少应该能表达：

- owner。
- controller。
- public keys。
- service endpoints。
- version。
- expiration。
- revocation state。
- payment address 或 beneficiary。

这让名字从“字符串”变成协议主体。

### 6.3 表达 owner 和 controller

BNS 必须表达谁当前控制这个名字，以及谁可以代表它签发哪些声明。

owner 是最终控制者，controller 可以是被授权的操作主体。很多实际系统都需要这种区分：用户不应该每天用最高权限 key 签所有 AppDoc、设备文档和内容 meta。

### 6.4 更新当前指针

BNS 必须支持合法更新：

- Zone 迁移。
- gateway 变化。
- OOD 替换。
- key 轮换。
- App 升级。
- content meta 更新。
- Group controller 变化。
- profile 恢复。

这就是 BNS 区别于内容哈希的地方。哈希表达不可变对象，BNS 表达可演化实体。

### 6.5 支持签名验证链

BNS 解析结果必须能进入完整验证链：

```text
BNS name
  -> owner / controller
  -> signed document
  -> service / app / content / device proof
  -> concrete connection or downloaded object
  -> hash / signature verification
```

如果一个名字不能进入验证链，它就不能作为协议级基础设施。

### 6.6 支持支付和分润

BNS 名字应该能作为支付主体、收益主体和合约规则入口。

这不是为了把 BNS 变成支付系统，而是因为真正的账号必须能承载经济关系。内容、App、服务、索引、推荐、Group 协作都会自然需要支付和分润。

### 6.7 支持多种 provider 和缓存，但输出信任上下文

BNS 的实现可以有合约 provider、SN 可信查询、DNS TXT 兼容路径、HTTPS DID provider、Zone resolver、缓存和本地索引。

但 resolver 输出不能只是“查到了什么”。它还应该告诉上层：

- 结果来自哪里。
- 由谁签名。
- 由谁验证。
- 是否过期。
- 是否降级。
- 是否命中缓存。
- 是否与链上状态冲突。

这让应用可以在普通模式、强验证模式和开发模式之间做清楚选择。

## 7. 为什么这些功能就足够了

BNS 不需要一开始就包办所有事情。

它不需要成为唯一内容数据库，不需要成为唯一搜索引擎，不需要成为唯一支付应用，不需要成为唯一社交平台，也不需要替代所有网络连接协议。

BNS 只需要守住协议级基础设施该守住的边界：

1. 名字长期稳定。
2. 控制权可验证。
3. 当前状态可更新。
4. 下级声明可签发。
5. 历史状态可追溯。
6. 经济主体可表达。
7. 中心化服务不能夺走最终控制权。

有了这些能力，上层生态就可以自然生长：

- 个人集群可以基于同一个 owner 自动建立基础信任。
- 设备和服务可以拥有协议级身份。
- global profile 可以成为真正账号。
- boot config 可以可靠地发布和恢复。
- 内容可以按长期名字购买和升级。
- 创作者可以脱离单个平台赚钱。
- 索引者可以竞争推荐和信用体系。
- App、Agent、Group 可以成为可验证、可支付、可迁移的主体。

这也是为什么 BNS 必须是协议级基础设施。

去中心的东西最终都是协议级的。只要某个能力的控制权留在单个公司、单个平台或单个数据库里，它就还不是去中心基础设施。BNS 要做的是把名字、身份、控制权、当前状态和经济关系抽象成协议，让任何实现者、索引者、钱包、设备、App、Agent、网关都可以围绕它协作。

因此，BNS 的完成不是某一个应用功能完成，而是一个基础协议闭环完成：

```text
human-readable name
  -> verifiable owner
  -> updatable profile / document
  -> signed sub-objects
  -> trusted connection / content / payment
  -> open ecosystem
```

当这个闭环成立时，BNS 就足够了。

它不需要控制所有上层业务，因为上层业务正应该由开放生态完成。它只需要保证所有上层业务都能回到同一个可验证的名字和控制权根上。

这就是 BNS 的核心价值：让一个名字同时成为账号、集群入口、启动锚点、内容资产和支付主体的可信根。

也正因为它处在这个位置，BNS 一定会走向完成。

## 8. BNS 的核心 API

这里说的 API 不是某个 SDK 的函数形状，而是 BNS 作为协议级基础设施必须提供的最小能力闭环。

站在最终协议确定性的角度，BNS 的唯一真相源一定是智能合约。SN、Global Provider、Zone resolver、HTTPS、DNS TXT 和本地缓存都可以存在，但它们只能作为查询、缓存、加速、兼容或 Zone 内授权解析路径，不能成为最终权威状态。

BNS 协议层接受的合法 DID 形态应该收敛为两类：

```text
did:bns:$name
did:dev:xxxx
```

其中 `did:bns:$name` 是进入 BNS 合约状态和 DID Document 解析链的语义名字；`did:dev:xxxx` 是设备自认证 DID，适合表达设备公钥身份，可以出现在 ZoneConfig、DeviceDocument、ServiceInfo 和连接协议里。

BNS 的 API 可以分成三层：

1. 名字资产层：解决名字如何注册、拥有、续期和释放。
2. DID 文档层：解决名字当前指向什么、由谁签发、是否有效、如何更新。
3. Resolver 层：解决客户端如何得到可验证结果，以及如何判断 provider、缓存和降级状态。

这三层合起来，才能支撑前面说的 personal cluster、global profile、boot config、内容发布和支付分润。

### query_name_state(name)

查询一个名字的链上状态。

返回信息至少包括：

- owner：当前名字资产 owner。
- status：`available | active | expired | released | tombstoned`。
- expire_at：过期时间。
- owner_config_version：当前 OwnerConfig 或 profile 版本。
- latest_document_versions：各 `doc_type` 的最新版本。
- payment_policy：续期、owner 更新、收益相关策略。

这个 API 用来回答“这个名字现在是否存在、由谁拥有、是否还能被注册或更新”。

### register_name(name, owner, options)

注册一个全局名字。

`owner` 是名字资产的拥有者，可以是公钥、钱包地址、合约账户或未来兼容的 NFT owner。注册时可以同时设置初始 OwnerConfig / profile。

`options` 至少应该允许表达：

- expire policy。
- 初始 resolver hint。
- 初始 payment address / beneficiary。
- 是否允许 owner 更新或迁移。
- 是否允许子名字由 Zone resolver 扩展。

注册成功后，这个名字进入 BNS 全局名字空间，成为后续 DID 文档和 profile 的根。

### renew_name(name, duration)

续期名字。

名字过期是 BNS 经济模型的一部分，但不应该所有名字都必须过期。一般来说，越短、越稀缺、越高价值的链上名字，越适合引入自动过期和续费机制；Zone 内派生名字通常不需要走全局续期。

续期只改变名字资产状态，不应该改变 DID Document 内容。

任何人都可以为名字续期付费。

### resolve_did(did, doc_type) -> ResolveResult

解析 DID，返回可验证文档和信任上下文。

`resolve_did` 只接受合法 DID：

- `did:bns:$name`：通过 BNS 合约状态和 DID Document 解析。
- `did:dev:xxxx`：通过设备自认证 DID 或已验证 DeviceDocument 解析。

`doc_type` 用来复用宝贵的一级名字。例如同一个 `did:bns:alice` 可以解析：

- `owner`：OwnerConfig / global profile。
- `boot`：ZoneBootConfig。
- `zone`：ZoneConfig。
- `service`：ServiceInfo。
- `app`：AppDoc。
- `content`：content meta。

`ResolveResult` 不能只包含 document，至少应包含：

```text
document
verified_owner
controller
doc_type
version / seq
status: active | revoked | expired | migrated | tombstoned
provider
trust_root
proof
cache_state
warnings
```

provider 的使用顺序需要按上下文区分，但最终权威状态必须能回到智能合约：

1. 全局名字的权威状态来自 BNS 合约。
2. Zone 内子名字可以先查本 Zone provider，但必须能回溯到父 Zone / Owner 授权。
3. Global Provider，例如 SN，可以作为合约状态的加速、缓存或证明转发入口。
4. HTTPS / `.well-known` / DNS TXT 可以作为兼容发现路径，但返回内容必须继续验签和校验状态。

因此，Zone provider 不能任意覆盖全局 BNS 名字；它只应该解析自己被授权的命名空间。

### update_did_document(did, doc_type, document)

更新某个 DID 的当前文档。

授权判断必须基于“更新前”的当前状态，而不是由新提交的 `document` 自己声明。

也就是说，系统需要先确认：

```text
current name state
  -> current owner / controller policy
  -> proof signer has update permission
  -> expected_seq matches latest seq
  -> new document is accepted
```

`document` 可以声明下一轮 owner、controller、service endpoint、payment address、resolver hint、version policy 等信息，但这些信息只能影响后续状态，不能反过来授权本次更新。


### revoke_did_document(did, doc_type, version_or_range)

吊销某个文档或某个版本范围。

吊销不等于删除。它表达的是“这个版本从某个时间点开始不再应被视为当前有效状态”，但历史上基于该版本产生的签名、receipt、内容版本和审计记录仍然可以按当时状态验证。

典型用途：

- key 泄露。
- AppDoc 废弃。
- boot config 失效。
- Zone gateway 被替换。
- content meta 被发现有问题。

### set_did_alias(did, target_did)

设置别名或迁移关系。

这个 API 不应该叫 `rename_did`，因为 rename 容易让人误解为旧 DID 被直接替换。更准确的语义是：

- alias：`did` 可以作为 `target_did` 的别名使用。
- migrated：`did` 已迁移到 `target_did`，新请求应优先使用目标 DID。
- canonical：客户端展示时可以把目标 DID 视为规范名字。

客户端可以提示用户迁移保存的 DID，但不能无条件改写所有历史记录。历史签名、购买 receipt、收益归属、旧版本内容仍可能必须保留原 DID。

### change_owner_key(name, new_owner_key, proof)

轮换 owner key。

这个 API 表达的是“同一个名字 owner 更换控制密钥”，不是名字资产转让。

owner key 轮换通常用于：

- 日常安全轮换。
- 旧 key 泄露后的恢复。
- 从临时 key 迁移到硬件钱包或多签。
- 从单 key 迁移到 controller policy。

如果名字资产 owner 是 NFT / 合约账户，那么 `change_owner_key` 可能体现为更新 OwnerConfig 中的 auth key，而不是改变 NFT owner。


### release_name(name)

释放名字。

释放后，名字可以进入过期、冻结或重新注册流程，具体取决于经济模型。

但 release 不应被解释为“这个名字从来没存在过”。如果名字曾经发布过文档、收过款、签过内容、产生过 receipt，那么历史状态仍需要可追溯。

对于高风险名字，可以使用 `tombstone` 状态：名字不再可用，也不允许被别人重新注册，但历史记录仍可查询。

### BNS 名字的自动过期

自动过期属于 BNS 经济模型，而不是普通 DID 文档能力。

建议原则是：

- 短名字、一级名字、高价值名字可以自动过期，需要续费。（目前定位是7位以下，或有特殊意义的名字会自动过期，比如8个相同字母）
- 长名字、Zone 内子名字、临时服务名可以由 Zone resolver 自行管理。
- 过期影响后续更新权和重新注册资格，不应抹除历史 proof。
- 已经绑定支付、receipt、内容归属的名字，过期后的重新注册需要有明确保护期或 tombstone 规则，避免钓鱼和历史权益混淆。

### 最小闭环

如果只保留最小核心，BNS 至少需要这些协议能力：

```text
query_name_state
register_name
renew_name

resolve_did
update_did_document
revoke_did_document

set_did_alias
change_owner_key
release_name
```

这组 API 之所以足够，是因为它覆盖了 BNS 的全部基础职责：

- 名字可以被注册和拥有。
- 名字资产可以续期、更新 owner 和释放。
- 名字可以解析到可验证文档。
- 文档可以合法更新和吊销。
- owner key 可以轮换。
- owner / controller 可以通过 DID Document 被授权和限制。
- 迁移和别名不会破坏历史可信。
- provider 和缓存可以参与解析，但不能取代最终控制权。
