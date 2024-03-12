# Project Templates

基本思路：保持Kernel的简洁
能用app实现的功能就用app
能用service实现的功能就用service
最后才考虑放到kernel里

在上述思想下，大量AI相关的功能都被放到了service层和app framework(sdk)层


## components
以rust静态库形式存在，可以被复用的通用库。通常是一些协议的解析，和对service的调用客户端
如果不知道自己做的功能未来会是kernel还是service

## kernel 
buckyos是一个NOS,因此kernel的组件都是以kernel service的形式存在的。通过kRPC调用其接口。
从实现的角度来看，每一个完整的kernel组件都是一个独立的service,大部分时候不会运行在容器内，根据配置，在Zone可以有多个实例。但大部分情况下，在一台机器上只会运行一个实例。

kernel service一旦出现故障，整个Zone就视作出现故障，可能无法正常工作。

## services 
services是以用户态提供系统的一些可扩展(可缺失)的通用功能。services一般运行在容器中，并有机会获得root权限。
services一般基于SDK开发。
我们现在最重要的一个service是K8S

## apps
apps是用户态的应用程序，apps一般运行在容器中，不会获得root权限。
app可以根据拉起方式继续细分种类。aios目前有两种
    1. app services （根据配置保持固定数量的实例）
    2. agent app (通过Agent Msg / event拉起)

app，尤其是agent app基于SDK开发。

## tools
tools是一些开发辅助性的工具，通常是用来调试和管理系统的。
管理系统的工具，通常是app
而开发工具 ，通常可以不视作buckyos的一部分