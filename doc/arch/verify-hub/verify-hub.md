# 理解buckyos的身份体系

## 自验证流程

objid->namedobject , hash验证,objid里是内容相关的，和授权部分（签名段）无关
did->did_document ，授权验证（提供了可验证的身份体系）


## 常见的业务验证需要

一个业务在处理请求的时候，需要得到请求的下面关键要素
- action
- target resource path list
- operation userid
- operation appid

OOD/设备启动的时候，首先就是要知道自己所在的zone情况
    通过阅读设备配置文件，可以知晓：自己所在的zoneid,设备的owner,设备本身的did/device_info,设备自己的私钥

启动的时候，首先需要得到一个可信的ZoneConfig,然后根据这个ZoneConfig决定自己的下一步的动作
    得到可信zoneconfig的方法
        1. 验证zoneid->did_document(zoneconfig)，这个过程是did有限的
        2. 验证zoneconfig里的owner和设备的owner是相同的 （这意味着当zone的owner改变后，zone内的所有设备必须重新激活，防止zoneid配置被攻击后的潜在的隐私问题）。反过来，如果私钥丢失，只需要修改了zoneid的配置，那么所有的设备都会处于启动失败的状态。
        理解这个双向验证机制可以进一步理解系统对风险的管理。
        
        黑客攻击得到了owner密钥：用户用zone管理密钥（这甚至可以是传统的中心化身份）可以让zone不可用
        黑客攻击得到了zone管理密钥：不会修改

        


## sudo机制

SUDO 机制是由 Verify-Hub 提供的,通过一个特殊的提权对话框，要求管理员用户输入密码。在输入密码之后Verify-Hub  会签发一个 SUDO Session Token.

后续在发起请求时，就可以在请求中带上这个 Session Token。这就是 SUDO 的基本机制。Verify-Hub 的 Sudo 授权 token 通常都是时间比较短的（3分钟）,而且在 Sudo 的时候，可能会有明确的操作边界（填写aud)

sudo 的执行权限一样会受到 AppID 的限制。也就是说，其实对于非系统类的应用来讲，申请调用这个权限的意义不太大，因为它还是会被 AppID 限制住。

所以说，一般都是在类似于 Control Panel 这种系统 UI 中，即它的 AppID 本来就具有大权限的情况下，给 sudo 才有意义。



## 会话持久化与混合 SSO 方法

- refresh family 存在本地 SQLite `data/var/verify-hub/sessions.db`（不存 token 明文，宽限答复按记录重签）；单实例，迁移或恢复后全部重新登录。
- 每对用户 token 带 `auth_method`（password / exclusive_invite / did_federation / assertion）与会话上限（`restricted` + `scopes`）；refresh、delegate 只收窄不放宽。
- 新方法：`login_by_exclusive_invite`、`revoke_families`、`issue_federation_code`、`login_by_remote_verify_hub`（这四个只接受 Control Panel 服务会话）与公开的 `exchange_federation_code`。wire 与流程见 [混合 SSO 技术契约](混合SSO技术契约.md) §5、§7、§9。
- Limited 账号不签发 sudo；`password_state=unset` 的账号在比较摘要前即拒绝密码登录。
