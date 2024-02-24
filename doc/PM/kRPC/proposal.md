# kRPC
1. 身份认证（该模块可以独立？）
2. kRPC的协议设计，范式定义，如何在保持高可扩展性的同时，尽力提高性能
3. kRPC的3种通信场景本地kRPC,Zone内kRPC（同内网/非同内网）
4. 


## 身份认证
kRPC的核心是让运行在User状态的App与运行在Kernel状态的service通信,Kernel Service可以使用kRPC的身份认证接口准确的判断其System Call的来源。
如何在APP的客户端里（web or ios APP），实现统一的身份管理?

思路一、先获得源端口号，然后根据端口号找到pid，然后根据pid找到进程的身份(appid)。
优点：
对APP开发者友好（基本是透明的）

缺点：
只能实现本地通信的身份认证，无法实现Zone内（跨设备）的身份认证。
有一定的一致性问题，缓存`源端口<->pid<->appid`的关系可能有安全风险。
通过pid找到进程的身份appid的实现需要监控app的所有子进程创建，这个实现可能对app本身的灵活性有一定的影响。

安全风险：
源端口仿冒
pid仿冒，因为kernel service不会每次都去检查pid是否合法，所以如果app的pid被仿冒，那么app的身份也会被仿冒。

思路二、建立握手协议，app首先通过握手协议申请一个token,然后在协议中使用这个token. Kernel Service可以使用kRPC的身份认证接口+Token来准确的判断其System Call的来源
优点：
可缓存性好，一个Token本身带有有效期信息，可以很方便的进行缓存。
可以很好的兼容传统的name/password的认证方式,并得到一个可以通用的Token

缺点：
握手时的鉴权方式
1. Kernel在启动AppRuntime时，会给定一个有效的Token。
2. 使用登陆模型或秘钥对模型，通过Kernal的AuthService来得到一个Token。该Token会说明其有效期和绑定的appid

安全风险：
APP本身没有泄露自己system token的主动性。
但如果APP的实现有安全漏洞，恶意APP可以窃取有漏洞APP的Token，并以该APP的名义（用户可能很信任该APP）

### 强Root身份（无Token验证）
如果一个kRPC调用，有针对本次调用的`管理员签名`，那么可以不经过上述机制直接得到高权限。这类似与sudo命令，是和单次kRPC绑定的。
系统真正需要root权限的地方很少，大部分情况下，使用有管理员权限的普通账号就能解决所有问题。


### 实现
基于casbin-rs实现，casbin-rs主要是统一了相关的配置文件的格式。
在使用casbin-rs前，要验证一些典型场景如何实现？

## 协议设计
