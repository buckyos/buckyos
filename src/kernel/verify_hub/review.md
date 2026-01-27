
## P0 必须修的安全边界问题

### 1) `verify_token` 目前会接受“任何信任链签名的 JWT”，不只 verify-hub 签发的 session

`handle_verify_session_token()` 里只要是 JWT 就走 `verify_jwt()`，而 `verify_jwt()` 会根据 `kid` 去 `get_trust_public_key_from_kid()` 取 **root / device** 等信任公钥来验签。

结果：**root/device 签的“登录 JWT / 其它 JWT”也可能被当作 session token 接受**（只要 exp 没过）。这会把“登录凭证”和“会话凭证”的边界打穿。

✅ 建议（强烈）：

* 给 token 加一个明确用途字段，比如：`token_use: "session" | "refresh" | "login"`（或 `scope`/`typ`）
* `verify_token` 必须强制：

  * `iss == "verify-hub"`
  * `token_use == "session"`
  * （可选）`aud == appid` 或服务名
  * 算法白名单（见下一条）

这样才能保证“只有 verify-hub 签发的 session 才能用来访问 API”。

---

### 2) 你用 `kid` 来区分 refresh vs first-login，不可靠，且语义不对

`handle_login_by_jwt()` 用 `iss_kid`（其实是 header.kid）去 match `"verify-hub"`，把它当成 refresh flow 的判据。

* `kid` 是 **key id**，不是 issuer。
* 你自己生成 session/refresh 时也都用 `kid="verify-hub"`，只是靠 refresh cache + nonce 才挡住“拿 session token 冒充 refresh”。
* 将来你做 key rotation、多 key、甚至 kid 缺失/不同格式，会埋雷。

✅ 建议：

* refresh vs login **用 claim 判断**：`token_use == "refresh"` 才能 refresh。
* 对 refresh 再额外校验 `iss == "verify-hub"`（你已经在 token 里写了 `iss: Some("verify-hub")`，但当前没强制检查）。

---

### 3) JWT 算法/校验项没有收口（建议强制 EdDSA + 限制 iss/aud）

`verify_jwt()` 用 `Validation::new(header.alg)`：token 说自己是什么 alg，你就按它来校验。虽然最后 decode 需要正确 key，但安全上更推荐**白名单**：

✅ 建议：

* 明确只接受 `EdDSA`（或你实际使用的算法）
* 并在 `Validation` 里启用/配置：

  * issuer（`iss`）
  * audience（`aud`）
  * required claims（exp/iat 等）

你现在手工检查 exp 没问题，但 `iss/aud/token_use` 这些必须由校验层强制，不要靠上层“记得检查”。

---

## P1 强烈建议改（会影响可用性/安全性/运维）

### 4) Refresh token rotation 只有 “invalidate old”，缺少“复用检测（reuse detection）”

现在旧 refresh 再来一次会报 “not found or already invalidated”。这在安全上还不够：**如果旧 refresh 被再次使用，通常意味着 refresh 泄露**，应该触发更强动作：

✅ 建议：

* 如果收到一个 refresh（验证签名 OK）但 cache 里查不到 / nonce 不匹配：

  * 记录安全事件（risk log）
  * 吊销该 `sid/session_id` 的所有 token（至少清掉 session cache / refresh cache / 强制重新登录）
* 这就是标准 rotation 的“reuse detection”逻辑。

---

### 5) token 缓存只在内存里：重启全部失效 + 没有过期清理

`TOKEN_CACHE` / `REFRESH_TOKEN_CACHE` 都是进程内 `HashMap`：

* 服务重启 → 所有 refresh 失效（用户全掉线）
* 不做清理 → cache 会无限增长（尤其 first-login 用 `userid_appid_token_nonce` 标记 replay 那个 key，会持续累积）

✅ 建议：

* 至少加一个周期性 GC：清掉 `exp < now` 的 entry
* 如果你希望重启不掉登录：把 refresh/session 的“服务端状态”落盘（dfs:// 或 sqlite/rocksdb），哪怕只存 refresh 的 `sid -> current_refresh_nonce` 也行。

---

### 6) `load_service_config()` 失败后，服务仍继续跑（而且可能用硬编码私钥）

`service_main()` 里：

```rust
let _ = load_service_config().await.map_err(|error| { ... return -1; });
```

这里即使失败也不会 `return`，服务仍启动。此时：

* `VERIFY_HUB_PRIVATE_KEY` 仍然是文件里那段默认私钥（非常危险）
* trust keys 可能没加载完整

✅ 建议：

* config 加载失败就直接退出进程（panic/return）
* 同时建议把默认私钥从生产代码移除（至少用 feature gate 或仅测试编译时存在）。

---

## P2 设计细节与一致性建议（让体系更“标准”和更抗坑）

### 7) session / refresh 两个 token 的 claims 结构目前几乎一样

你现在 session 与 refresh 都是 `RPCSessionToken`，字段相同。建议至少加：

* `token_use`（必须）
* （可选）`sid`（会话 id；你用 `session` 字段承担了，OK）
* （可选）`jti`（JWT 唯一 id，便于审计/黑名单）

另外 refresh 通常不建议承载太多权限信息（scope/role），只要能换新就行；权限应放在 access/session 里。

---

### 8) `verify_token` 应该校验 appid/aud（否则 token 可能跨 app 复用）

你现在 `handle_verify_session_token()` 只检查 exp。建议：

* 明确 `appid`（或 `aud`）必须匹配调用方服务/域
* 或 verify-hub 侧提供 “verify(session_token, expected_appid)” API

---

### 9) 密码登录方案不够标准（存储与抗攻击性）

你现在的密码校验是：客户端传 `SHA256(stored_password + nonce)`（base64），服务端从配置里读 `store_password` 再做同样 hash 对比。

风险点：

* `store_password` 如果是明文或可逆，等于服务端保存“可直接登录的秘密”
* `SHA256` 不适合作为密码哈希（缺少慢哈希与盐策略）
* nonce 允许 8 小时窗口，也会扩大重放/撞库利用窗口

✅ 建议：

* 若可以：改成服务端 `argon2id/bcrypt` 存储与校验（TLS 保护传输）
* 若你坚持“客户端不直接发明文密码”：考虑 OPAQUE/SRP 之类 PAKE（你们体系偏 crypto，这条更正宗）

---

## 你这版实现里“做得好的点”

* `SESSION_TOKEN_EXPIRE_SECONDS = 15m` / `REFRESH_TOKEN_EXPIRE_SECONDS = 7d`：参数合理（后续可配置化）
* refresh flow：校验 cache 存在 + nonce 匹配 + 过期检查 + **先 invalidate 再发新**，rotation 的基本动作顺序是对的
* first login replay：用 cache 标记 nonce 用过，方向正确（只是要加 GC/落盘）

---

## 我建议你下一步最小改动的落地顺序（不大改结构，但把坑堵上）

1. **加 `token_use` claim**（session/refresh/login 三种）
2. `verify_token` 强制 `iss=="verify-hub" && token_use=="session"`
3. `login(jwt)` refresh 分支改为 `token_use=="refresh"`（别看 kid）
4. 算法白名单（只允许 EdDSA）+ issuer/aud 校验
5. refresh reuse detection：发现异常 → 吊销该 sid 会话
6. cache GC +（可选）refresh 状态落盘


