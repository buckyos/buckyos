# 用户类型的分类

可登陆用户，是指可以在desktop中登陆的用户
非登陆用户，是指可以在某些dapp网页中，可以使用一个长期身份进行交互的用户

## root

root用户拥有系统内的全部权限
root用户的认证用zone外私钥，因此不可登陆（单操作授权）

## admin / su_admin

可登陆用户中权限最大者（root用户日常也用该账号登陆)

拥有admin用户该有的读权限，一些敏感操作，需要使用sudo权限（是verify-hub能签发的最高权限），需要再次输入用户名密码

- 能读系统配置和自己的home目录 （不能读其它用户的数据）
- 

相比root用户,admin少了哪些权限？（即使用了sudo)

- 无法调整zone-config (最重要的就是配置ood + gateway)
- 无法操作用户自己的on-chain资产
- 无法解密用root密钥参与加密的数据（通常是一些敏感备份）

> 因为root用户的私钥不在系统里，所以系统不能真的用root用户的权限
  
## users (普通用户) / su_user

最常见的可登陆用户，主要使用系统，对使用二级did的用户来说，这是主要存在的情况
使用app,单不能对系统进行管理

> 二级did也可以是管理员，权限组是权限组，鉴权是鉴权

## limit user （规划中）

可登陆用户，相比users,有一些特殊的限制（比如不允许修改密码）
未成年用户也在这个范围内，因此根据特殊的限制

## friends

非系统登陆用户（在某些app里有登陆状态）
通常拥有SNS的读权限
通常拥有评论类的写权限

## guest

非登陆用户（匿名用户）
系统会对这类用户做一个统一的权限设置（默认是不开启，阻止任何匿名访问）
系统只有一些明确的zone级别的public resource允许匿名访问（这需要一个列表）


## 让某个(传统)app提供公共服务的方法：

- app设置为公共访问(gateway）不会拦截 ：这在buckyos看来是让app拥有了特殊权限
- app可拥有自己的用户管理组件
- app基于自己的权限管理系统管理信息的读写，单能管理的信息局限在app data范围内

## 用户是否有顶级bns name?

不管有没有顶级的bns name,用户账号都可以用did表达
doc_type = user 用户的公共profile
doc_type = owner 用户作为zone-owner的profile, 不一定是公开的（可以加密了）

系统不在意用户是否有bns name，因为这取决于用户向外分析自己的方法
如果用户用的是  did:bns:$username, 分享自己的个人主页，那么于用户进行交互的时候，就要经历一个resolve_default_zone的过程
如果用户用的是 did:bns:$username.$zonename的方法，那么就明确的对外说在这个zone上可以拿到自己的个人资料，并且说明自己是一个子账号

zone内是否保留用户的私钥，也是产品选择

## 用户如何“声明一个内容”是我创建的（用户的私钥如何使用）

- 用root密钥签名 （100%手工，效力最强）
- 用controller密码签名（这个密钥是否放到zone上外部不太介意，半手工）
- 用bind-zone的controller密码签名 （日常普通信息发布）

推论：除非用户是二级did,否则都需要通过更新自己的ownerconfig来让zone拥有可用的私钥

## zone外登陆(SSO)

用户的binded zone为current-zone
要验证用户登陆的zone为target-zone

1）使用钱包登陆
2）使用用户的current-zone登陆
3）在target-zone上开临时账号（这就不算登陆了）,开通临时账号的第一次，也是需要一次联合登陆的

## 通过did或的用户的profile

profile是一个json

1）先从current-zone上获得profile
2) 获得用户保存在bns上的profile(不依赖任何zone)

合并json，如果都有的字段以BNS上的为准。

> 普通用户总是拥有100%的权利，基于root密钥来修改自己的profile

## 用户管理相关的核心流程

### 新建

- 在system_config中添加必要的配置
  - user settings: 比如登陆密码），用户所在的权限组
  - user profile: 社交网络类的信息
- 普通用户:更新自己在BNS上的zone-binded
- 二级用户:自动创建全套的OwnerConfig资料 
- 在DFS上创建用户数据目录（一般是首次使用时按需创建的）


### 开放注册

允许did符合条件的用户自助完成账户开通的流程，开通后可以正常在系统中登陆
(目前暂不支持）

### 登陆

登录的目标，是让 Client 得到一个可以保存并用于后续请求的 Session Token。

Session Token 表示一个已经完成认证的用户会话，通常包含以下信息：

1. 当前用户身份，例如 User ID / DID。
2. 当前 App ID。
3. 过期时间、nonce 等会话属性。
4. 签发方信息，通常是 Verify Hub。

业务服务拿到 Session Token 后，只需要验证 token 并结合 RBAC 做权限判断，不需要理解用户最初是通过密码、钱包、Passkey 还是其它凭证完成登录的。因此，对鉴权方来说，登录方式的扩展应当是透明的。

从结果上看，系统需要支持两类登录方式。

第一类是直接使用钱包或私钥签发凭证。用户可以基于自己的 Root Key / Control Key，通过钱包私钥签发可被系统接受的 Session Token 或登录凭证。这种方式安全敏感，尤其不适合随意签发长效 token；但如果钱包自己管理了相应的安全策略，系统层面可以接受这种由高权限私钥证明的身份。

第二类是通过 Verify Hub 登录。这里的登录流程是日常主路径：用户向 Verify Hub 提交某种登录凭证，Verify Hub 验证通过后，签发标准 Session Token。后续系统服务只关心这个 token 是否可信，以及 token 中声明的用户、App 和权限上下文。

Verify Hub 登录页需要支持用户输入完整 DID，也可以支持输入系统内的 username。为了避免冒名或伪造身份，登录输入最终必须能映射到一个明确的 DID。Verify Hub 会读取 User Settings 和用户状态，判断该用户是否存在、是否允许登录，以及是否具备当前登录方式所需的凭证。

当前最基础的方式是用户名 / DID + 密码登录。只要 User Settings 中存在对应的密码凭证，并且用户状态允许登录，Verify Hub 就可以验证凭证并签发 Session Token。

Verify Hub 登录页也可以支持钱包登录，但这里的钱包登录和“用户直接用钱包签发最终 Session Token”不是一回事。更合理的流程是：

1. 钱包对一个短期登录 challenge 或短期登录 token 签名。
2. Verify Hub 验证钱包签名。
3. Verify Hub 把该证明换成自己签发的标准 Session Token。

从产品设计上看，这种钱包登录可以优先用于“忘记密码”或账号恢复流程。日常登录不应鼓励用户频繁暴露 Root Key / Control Key 的签名能力。未来如果钱包体验足够好，或者与系统本机 KeyPass、Passkey 等机制集成，也可以把它作为 Verify Hub 支持的一种普通凭证类型。

因此，登录能力的扩展点应集中在 Verify Hub 的统一登录页上。只要 User Settings 中能描述某种凭证与用户 DID 的绑定关系，Verify Hub 就可以增加新的验证方式；对后续业务服务和 RBAC 鉴权逻辑来说，仍然只看到标准 Session Token。


### SSO

SSO 可以看成是 Verify Hub 登录能力在 Zone 之间的扩展。

这里有两个角色：

- Current Zone：用户当前绑定或拥有的 Zone。
- Target Zone：用户希望登录并执行操作的目标 Zone。

典型流程如下：

1. 用户在 Target Zone 输入或选择一个 DID。
2. Target Zone 通过 DID 解析得到用户的 Current Zone，以及该 Zone 的 Verify Hub 信息。
3. Target Zone 拉起 Current Zone 的 Verify Hub 登录页。
4. 用户在自己的 Current Zone 完成认证，例如输入用户名 / 密码，或使用钱包、Passkey 等方式。
5. Current Zone 的 Verify Hub 签发一个联合登录凭证。
6. Target Zone 验证该凭证，并把它映射到一个明确的 DID 或本地用户账号。
7. Target Zone 的 Verify Hub 再签发本 Zone 内可用的 Session Token。

用户最终要在 Target Zone 内访问资源，因此最终使用的仍然应是 Target Zone 签发的 Session Token，并继续接受 Target Zone 的 RBAC 和 scope 约束。Current Zone 的联合登录凭证只负责证明“这个用户是谁”，不自动代表他在 Target Zone 里拥有什么权限。

这与传统 Google SSO 的本质类似：Target Zone 信任一个外部身份提供方完成身份认证。区别在于，BuckyOS 体系内的身份提供方可以是用户自己的 Zone，而不是固定的中心化平台；身份提供方的发现和信任根来自 DID、ZoneConfig 和 Verify Hub 公钥。

对 BuckyOS 内部跨 Zone 登录来说，DID 通常是明确的。因为系统凭证的核心是 username / DID，每个可登录用户都应该能映射到一个确定的 DID。

如果未来支持 Gmail 等传统外部账号作为联合登录方式，也必须在登录完成后绑定到一个 DID。绑定方式有两种：

1. 绑定到一个已经存在的 DID，相当于为该 DID 增加一种新的登录方式。
2. 作为 Target Zone 内的本地账号创建一个二级 DID。



### 改变用户状态

- 改变用户组
- 改变profile

### 用户数据管理

对app service来说，不假设自己能看到一个用户还是多用户的数据，这是系统管理员在安装时的配置，下面是几种典型的模型

单服务(容器)多用户:最常见，但是管理员可以随时调整用户是否能使用该app
单服务(容器)单用户：是上面情况的特例，app service还可以通过在app-id中包含owner-user-id实现隔离（一份app代码，但是用多个不同的容器启动，每个容器只为一个用户服务，此时app.owner恒为userid）

对于app开发者，应该声明自己是那种模式,来方便管理员在安装的时候选择正确的模型.默认是多用户模式
注意app的owner和app的当前用户是两个概念。owner是app的安装者

默认情况下，用户之间的数据都是完全隔离的（包括管理员），只有root用户可以看到所有的数据。
使用root用户，可以导出任意用户的用户数据，这是一个典型的运维需求，而不是日常使用需求。
常规的用户数据导出（管理员可以辅助），应该用用户的登录密码加密。比如用户被禁用后，普通管理员可以协助导出的用户数据。


### sudo机制

SUDO 机制是由 Verify-Hub 提供的,通过一个特殊的提权对话框，要求管理员用户输入密码。在输入密码之后Verify-Hub  会签发一个 SUDO Session Token.

后续在发起请求时，就可以在请求中带上这个 Session Token。这就是 SUDO 的基本机制。Verify-Hub 的 Sudo 授权 token 通常都是时间比较短的（3分钟）,而且有可能在 Sudo 的时候，会有明确的操作边界

sudo 的执行权限一样会受到 AppID 的限制。也就是说，其实对于非系统类的应用来讲，申请调用这个权限的意义不太大，因为它还是会被 AppID 限制住。

所以说，一般都是在类似于 Control Panel 这种系统 UI 中，即它的 AppID 本来就具有大权限的情况下，给 sudo 才有意义。
