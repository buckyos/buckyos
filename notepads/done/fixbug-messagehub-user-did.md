# MessageHub 新用户收不到消息

2026-09-07，devtest 向新建用户 lucy 发消息后，lucy 登录 MessageHub 看不到会话。

## 原因

运行库 `msg-center-main.db` 中，消息 `cymsg:6586f46b41a362a1972c18f47b06436e179e496d5d6f9adaca84f8fbe0c6d181` 已成功投递。收件记录属于 `did:web:lucy.test.buckyos.io`，位于 `REQUEST_BOX`，状态为 `UNREAD`。前端却将用户名拼成 `did:bns:lucy` 查询，后者没有消息。后端会话鉴权也使用相同的错误假设。

同一个域名推断问题还会把普通 `did:web` 用户当成 Agent，影响前端用户分类和后端观察权限。此外，联系人同步原先只覆盖系统与 tunnel owner，没有覆盖 Lucy 的私有联系人作用域，因此把 devtest 当成陌生人。

## 修复

- 前端从 `user.get` 的用户档案读取真实 DID；身份缺失时拒绝初始化，不拼接替代 DID。
- 后端验证 token 后从 `users/{user_id}/profile.did` 解析身份，用实际 DID 判断读取和写入权限。
- Agent 观察要求本地托管且存在未删除的 Agent 注册记录；普通用户和本地群组不会因域名而取得观察权限。前端同样依据 Agent 列表或明确标签分类，Zone 用户保留为普通用户。
- 启动和重载设置时，同步联系人到每个有效 Zone 用户的实际 DID 作用域，并保留已有屏蔽设置。
- 不修改数据库结构或历史消息。原有请求箱消息通过正确身份恢复可读，后续同 Zone 好友消息进入 `INBOX`。

## 验证

- `cargo test -p msg_center --locked`：79 项通过，1 项真实 Telegram 凭据测试按原配置忽略。
- MessageHub 身份、投影和会话数据模型测试：12 项通过。
- 浏览器定向回归：5 项通过，覆盖 Agent 观察隔离、附件消息和草稿重试。
- 前端 TypeScript、生产构建及变更文件 ESLint 检查通过。
- `cargo build -p msg_center --release --locked` 通过；二进制位于 `/tmp/dev-cache-root/cargo-target/release/msg_center`，前端产物位于 `src/frame/desktop/dist`。

生效时需同时更新 desktop 前端和 msg-center，重启消息中心或加载新版本后执行设置重载以同步联系人。此次未修改运行数据库，未部署或重启运行中的服务。
