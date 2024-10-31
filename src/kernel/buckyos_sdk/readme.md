# BuckyOS SDK (typescript Web端)

## 身份验证

然后在浏览器中允许的,属于dApp的页面，都应使用BuckyOS SDK提供的登陆功能来获得一个有效身份。
身份验证流程如下：

路径1：1.浏览器发送请求 --HTTPs--> 2.cyfs-gateway进行验证 --Local HTTP--> 3.dApp Server使用BuckyOS Rust SDK进行验证。
路径2：1. App -->http@bdt--> 2.cyfs-gateway进行验证 -- Local HTTP--> 3.dApp Server使用BuckyOS Rust SDK进行验证。
**路径2 依赖有客户端身份的App/CYFS 浏览器，暂未实现**
路径3：1.App Service --RPC@http--> 2. system_service(验证rpc.token,系统调用验证，不走cyfs-gateway)


### cyfs-gateway的验证

cyfs-gateway会根据HTTP Request的Host字段和cookie中的buckyos_token字段来进行验证。 该buckyos_token通常是有相对较长有效期的普通权限jwt.
对POST请求，验证首先要得到jwt格式的rpc.token，然后进一步验证该rpc请求是否有正确的授权。涉及到敏感操作的rpc.jwt通常是短期的，甚至是一次性的,并有sudo级别权限。

验证方根据buckyos_token / rpc_token得到appid,userid,resource,OP组成四元组，到RBAC库中查询对应权限，并进行验证。
RBCL需要的四元组:

```
appid,来自http request的host字段,web sdk里不可设置
userid:必填
resource:来自http request的path字段.完整写法是app://appid/http_path/，各种系统调用都有自己构造path的方法，与参数有关。jwt中通常不包resource信息，除非特别敏感的一次性sudo操作
OP: 来自http request的method字段 (GET/POST/PUT/DELETE/...),jwt中是可以包含的。
```

cyfs-gateway会根据这四元组，到RBAC库中查询对应权限，并进行验证。其验证方法是先判断appid是否有权限，再判断userid是否有权限。任何一个不通过，都会返回权限不足。

cyfs-gateway对于需要http验证的请求，如果没有buckyos_token字段，会重定向到一个预设的访问登陆页，引导用户登陆成功后，再重定向回原URL。该登陆页面通常是支持guest访问的。

### buckyos_token字段的生成

```javascript
// at feedlist.excample.com
let bucky_token,user_info = await buckyos.authClient.login();
document.cookie = "buckyos_token="+bucky_token+"; Domain=.; Path=/";
let user_id = user_info.userid;
let rpc_client = new buckyos.kRPCClient(feedlist_api_url,bucky_token);
let user_feeds = rpc_client.get_user_feeds(user_id);
```

执行高权限操作

```javascript
on_click_change_password(){
    let new_password = get_new_password();
    let bucky_token =await buckyos.authClient.request("change password");
    let rpc_client = new buckyos.kRPCClient(account_api_url,bucky_token);
    //高权限操作的所有参数都在bucky_token的payload中
    rpc_client.change_password(bucky_token);
}

```

下面是feedlist_api的实现

```javascript
on_request(request,response){
    let user_id = request.params.user_id
    let token = request.bucky_token;
    payload = verify_token(token,verify_hub.public_key);
    if payload.userid != user_id {
        response.send(403,"permission denied");
    }

    if payload.appid != "feedlist" {
        response.send(403,"permission denied");
    }

    
    response.send(user_feeds);
}
```

### buckyos.authClient的实现

authClient主要靠系统的内置verify_hub服务来完成功能，其基本逻辑是

1. 在弹出窗口中，加载标准的,auth.$zoneid 页面，该页面会根据login时的参数调整一些行为
2. 用户在弹窗窗口中操作,并得到token，有2种方法
    a. 使用用户名密码向verify_hub发起请求，verify_hub会根据其掌握的账号信息返回必要的jwt验证信息
    b. 要求用户输入一个加密后的私钥，当用户输入正确的解密密码后，可以用该私钥来构造jwt
       

### bucky_token jwt payload的内容
签名都是verify_hub服务完成的
```
{
    "appid": "$appid",
    "userid": "$did",
    "key":"$aes_key", 用verify_hub的公钥加密的aes key，也用来做session key
    "iss": "verify_hub", 或 "$did",使用用户的私钥来构造jwt时，会使用用户的sudo权限
    "exp": "$exp"
}
```

### 防御jwt的重放攻击

因为会工作在http环境，因此会用明文发送jwt.
