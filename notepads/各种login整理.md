# 系统里的各种login整理

## BOOT阶段

- OOD 启动 SystemConfig， SystemConfig通过ENV拥有了第一个trust key
- OOD 启动 Scheduler进行Boot Schedule,用自己的ENV传递了用自己私钥构造的session-token
  - scheduler在完成调度后，进行首次system-config写入，此时system-config service使用上述session-token鉴权成功

## OOD 

LOGIN_TYPE : SELF

启动成功后，OOD一直使用自己自颁发的session-token与system-config通信，读取自己的node-config
根据node-config里的service list, OOD会拉起service,在拉起service时，会通过ENV传递login session-token (login jwt)

## System Config Service 

LOGIN_TYPE : None

System Config Service 是最基础的内核服务，不依赖其它服务，因此也不用login


### 鉴权
有trust key机制，认可trust key颁发的sesion-token,trust key有
- 当前设备的公钥(通过ENV启动时传递)
- kv://boot/config 中保存的两个公钥
  - zone-owner public key (root)
  - verify-hub public key

从上述鉴权模型可看到，拥有对boot/config的写权限，其实就拥有了系统的最高权限


## verify-hub 

LOGIN_TYPE : SELF

verfiy-hub拥有保存在kv://boot/config中的verify-hub public key对应的私钥
TODO:这个私钥目前也是保存在kv://中的，保存在本地是否会更安全一些？

verify-hub随时可以用自己的prviate key 自签一个有效的Session-token，来访问system-config-service

### 鉴权 login_by_xxxx

verfiy-hub的核心逻辑就是鉴权，每个login请求，都会尝试通过读取kv://里的相关配置，来对其进行验证。
- login_by_passwd : user用账号密码登录，每次登录成功都会创建session-id
- login_by_jwt : 根据jwt的iss区分
  - verify-hub: 这是一个refresh session token操作。走refresh逻辑
  - DeviceName : 从system-config中读取相关信息，并进行验证
  - 其它? 

### 授权
login登录成功后,verify-hub会创建logined-session-id,并基于该session-id颁发 session-token和refresh-token

### 安全防护与吊销授权
- verify-hub通常会因为自己的安全风险监测，触发授权吊销。吊销的实现就是讲一个session-id设置为无效
- session-id无效后，无法refresh,此时会触发重登录
- verify-hub也提供了对session-id / session-token进行验证的接口。有些服务在高风险环境下，可以放弃独立对session-token的验证，而全部走verify-hub进行验证

## Node Server 

LOGIN_TYPE : BY_JWT

Node-Server 上通常不运行SystemConfig,所以需要先登录成功，得到session-token才能连接SysstemConfig

1. 使用本地的device_private key自颁发一个session-token
2. 去verify-hub处 login_by_jwt
3. 设备有效，verify-hub颁发token-pair
4. 定期使用 verify-hub.refresh


### Gateway鉴权（OOD和Node走相同流程）
从协议的角度啊，gateway只能从两种协议流量里得到鉴权信息
- rtcp-tunnel,可以得到remote device config,并进一步得到其owner信息，构建权限信息
- http-cookie, 从http-cookie中可以读取得到user session token

鉴权发生在下面context

#### rtcp_stack:on_new_tunnel

#### node-gateway --鉴权--> service

不需要鉴权的流量

- 访问kapi : system_config , verify-hub的流量
- 安装时，声明了允许公共访问的 servcie


## Service 

LOGIN_TYPE : BY_JWT 

Service使用的login流程

1. 从ENV中得到来自Node/OOD的login jwt
2. 去verify-hub处 login_by_jwt
3. service 身份，verify-hub颁发token-pair
4. 定期使用 verify-hub.refresh

### 鉴权

使用api-runtime提供的鉴权函数，会定期读取rbac

只认来自kv://boot/config的两个public key签发的session token
  - zone-owner public key (root)
  - verify-hub public key


## User (Web/buckycli)

LOGIN_TYPE : BY_PASSWOAR 
SUDO_LOGIN_TYPE : BY_JWT

用户使用有交互的软件进行登录。有些软件支持“保存密码”，也是基于refresh-token本身的过期特性实现的(目前支持7天)


## sudo 

sudo 只对user有效
当user发起krpc携带的seession-token,是用自己的私钥签名，而不是verify-hub签名时，代表这是一个sudo session-token

期望改成如下结构（调度器构造，无法使用rbac的一些表达式特性)

```rbac
p,su_alice, kv://users/alice/key_settings ,read|write,allow

g, alice, admin
g, su_alice, sudo
```

这样对sudo类型的sesiont-token的鉴权可以被封装起来：识别到user-id是su_开头时，触发特殊的检查逻辑
- 先检查session-token
- 先基于普通用户进行enfore,如果失败再用su_xxx 进行enforce


## Client Device 

LOGIN_TYPE: BY_JWT

### Gateway鉴权

rtcp stack -鉴权-> new_tunnel -鉴权-> open_stream

- new_tunnel的鉴权，是允许授权设备接入
- open_stream的鉴权，是允许设备打开指定资源

-----------------REVIEW--------------------
**TODO**

0): RPR Session Token的设计，更加符合标准的jwt规范

这些是 JWT 里常见的标准字段/头部字段，含义如下（简明版）：
iss（Issuer）：签发者，谁发的 token
sub（Subject）：主体，token 代表谁（比如 user id / device id）
aud（Audience）：受众，token 给谁用（比如 appid / service）
exp（Expiration Time）：过期时间（Unix 时间戳）
iat（Issued At）：签发时间（Unix 时间戳）
jti（JWT ID）：token 唯一 ID，用于防重放/黑名单
kid（Key ID）：签名密钥标识（放在 JWT header 里，告诉验证方用哪把公钥）


1) **补齐 SUDO 的正确实现（端到端）**
- 明确 sudo token 的判定规则（你给的口径：user 私钥签、非 verify-hub 签；以及 `userid` 形态如 `su_*`）
- 在服务侧统一封装 sudo 校验与授权流程：
  - 先做 token 验签/过期检查
  - RBAC enforce：先用普通 `userid` 尝试，失败再用 `su_xxx`（或反之，按你文档的期望）
  - 防止 sudo token 被当作普通 session token 滥用（明确 token_use / iss / kid 规则）
- 给出最小可验证用例（buckycli 或 test crate）：普通用户访问被拒 + sudo 放行

2) **Review web-sdk 的登录/鉴权链路（与 verify-hub 协议对齐）**
- 盘点 web-sdk 实际调用的 verify-hub RPC 方法（`login_by_password` / `login_by_jwt` / `verify_token`），确认没有走废弃/不存在的 `login`
- 检查 token 存储与携带方式（cookie / localStorage / memory），以及刷新策略（refresh-token rotation / 过期重登）
- 校验 web 侧请求发往 node-gateway 时，session-token 的注入方式是否符合 SSO 约定（你文档里提到 http-cookie）
- 输出：一份“web-sdk 登录时序图 + 需要改的点 + 风险点”清单

3) **system-config：boot 阶段策略 vs 正常运行策略 分离**
- 明确 boot 阶段 system-config 接受的 trust key 范围（严格最小化）：只允许 ENV 注入的本机设备公钥 + `kv://boot/config` 中的 root/verify-hub 公钥（按你文档）
- 禁止/延后 boot 阶段的“动态扩张信任”行为（例如根据 `kid` 去 `devices/{kid}/doc` 自动加载并信任）
- 设计切换条件：进入正常运行态后，才允许加载完整设备列表/动态 trust key（或仍然保持严格策略）
- 输出：两套策略的明确差异、切换点、以及回归测试点

4) **统一口径：LOGIN_TYPE 语义修订与文档对齐**
- 把 LOGIN_TYPE 统一定义为“该角色对外提供的 login action 类型”，而不是“是否需要鉴权/是否需要 token”
- 按角色（OOD/system-config/verify-hub/node/service/user/sudo）补齐对应的 action 列表与调用方/被调用方
- 同步更新 `notepads/各种login整理.md` 里容易误解的段落（例如 system-config 的 “None”）

5) **删除默认私钥 PEM（仅测试用例保留）**
- 从生产编译路径中移除 verify-hub 的默认私钥 PEM（你已确认是给测试用例用的）
- 测试用例改为：测试模块内生成/注入 key（或 `cfg(test)` 专属的测试 key）
- 加一个防呆：生产启动若未加载到私钥直接退出，不允许 fallback

6) **node-server 兑换 token 的时序优化（与 ood/node-daemon 共享代码约束对齐）**
- 当前“5 秒后才兑换”的动机是先拉起各个 service；评估是否可以改成“并行进行：启动服务 + 立即兑换/续期 token”
- 明确哪些流程是 node 专属、哪些因为共享 node-daemon 代码导致带了 ood 逻辑；拆清边界，避免 ood/node 互相污染
- 输出：推荐时序（同步/并行）的选择 + 对启动稳定性/失败恢复的影响评估

7) **服务侧鉴权：严格限定只接受 root / verify-hub（按 runtime_type 收口）**
- 把“服务侧只认 root、verify-hub”落到代码层面：
  - runtime 的 trust key refresh 逻辑按 `runtime_type` 控制：service 类 runtime 不能引入 device key（或必须显式 allowlist）
  - 服务端 enforce/verify 层明确校验 `iss/token_use/aud`（至少对 session_token 要求 `iss=verify-hub` + `token_use=session`）
- 输出：按 runtime_type 的鉴权矩阵（允许哪些 kid/iss/token_use），以及对应实现点

