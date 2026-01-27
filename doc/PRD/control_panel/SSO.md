# BuckyOS 中的SSO 
SSO是BuckyOS为Web类应用提供的认证安全整体解决方案

## 基本原理

SSO的核心，是Web页面在向node-gateway发起http请求时，能携带一个正确的session-token. session-token中包含了正确的appid和当前登录用户信息。

POST类请求，在构成kRPC Client的时候填入session-token（使用 buckyos-web-sdk 发起kRPC请求，会自动处理)

GET类请求，通常将session-token保存在cookie里

BuckyOS的系统服务通常只处理kRPC, 使用下面通用流程判断请求是否合法

## 对兼容应用的支持

在系统冷启动期，会有不少移植的兼容应用：开发者将已有的web app打包到docker里，然后让buckyos安装。这类没有集成buckyos-web-sdk的应用被称作兼容应用。

此时node-gateway在处理这类app请求时，根据app配置，会自动将首个http请求跳转到 login_index.html（非弹出窗口),看起来url是https://app1.$zonehost/login_index.html, 在该页面完成登录后，会写入cookie。后续重定向会原请求后，就能在http req里带上正确的cookie

随后node-gateway检查发往app的http header有了正确的授权，把流量upstream到app service

## 不要混用 app_service自己的 seession_token和来自页面的session_token

我们鼓励app在web page里直接用kRPC访问必要的BuckyOS系统服务。

但有时，需要走 app-web-page --> app-service --> kRPC 这样的流程时，千万记住要用来自app-web-page的session-token,而不是自己的。
- session token 来自web-page: 有可能是另一个用户（架构上允许用户A安装的app给zone内所有用户使用)
- session token 来自app-service:必然是app的owner user

## 更多实现细节

### session token
session-token是一个由verify-hub-private-key签名的JWT.
一次verify-hub login返回两个token。 一个是长exp的refresh token,一个是标准的的session token(时间短），每次session token快过期了，client就用refresh token去verify-hub refresh,得到新的refresh token和session token.旧的refresh token会立刻失效


### 超级用户
某些页面需要超级用户授权(sudo),本质上，是需要一个session-token,该session-token用用户的私钥签名。因为buckyos不存储任何普通用户的私钥，所以：
- 在buckyos-app中，直接拉起buckyos-app的授权签名页面
- 否则，弹出一个签名页，要求用户输入私钥后签名