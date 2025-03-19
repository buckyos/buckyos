## 身份管理
- 通过环境变量得到启动 session_token
- 验证启动身份（login）

## 正确发起zone内请求
大部分service的请求都是来自客户端的，因此请使用客户端请求的session_token来进行下一步操作
自己主动发起的请求（比如基于timer的自检操作）通常是不被鼓励的，但一定需要的时候，使用自己的启动身份 （user-id是当前的device_id)
- 使用session_token和servcie_name初始化kRPCClient
- 使用kRPCClient初始化各种 ServiceClient

## 正确验证请求
- 使用session_token库来验证请求（默认的trust key只有verify_hub)
- 使用rbac库来验证用户是否有权限（用户的权限无法超过app-id的权限）

## 访问app settings

## 访问系统的一些重要的配置
- 设备列表，node列表，ood列表
- 系统里安装的服务列表
- 用户列表，用户安装的应用列表


## 使用文件系统
- data 目录
- cache 目录
- local 目录

## 使用NDN网络
- 发布内容到语义路径
- 可信的获取别的Zone发布的NamedObject






