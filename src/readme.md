# 源码的目录结构

## 开发构建中的 SDK/CLI 更新

在本目录执行 `uv run buckyos-build.py` 时，会先从同级仓库 `../../buckyos-websdk`
安装锁定的依赖、重新构建 SDK/CLI、打包并生成校验清单，然后更新
`rootfs/libexec/buckyos-tool/`。每次构建都会执行，包括 `--skip-web` 和 `-s` 指定模块的构建；
SDK/CLI 构建失败时停止，不继续使用旧分发包构建其它模块。需要本机安装 Node.js、npm、pnpm 和 Deno。

源码不在默认位置时，用 `BUCKYOS_SDK_TOOL_SOURCE` 指定本地 websdk 仓库路径；
`BUCKYOS_SDK_TOOL_DENO` 可指定 Deno 可执行文件，默认从 PATH 查找。
构建只更新 rootfs，执行 `uv run start.py` 后才会更新安装目录并重启系统。

发布构建仍可显式设置 `BUCKYOS_SDK_TOOL_ARTIFACT`、`BUCKYOS_SDK_TOOL_RELEASE_MANIFEST`、
`BUCKYOS_SDK_TOOL_DENO`、`BUCKYOS_SDK_TOOL_SBOM`，使用已生成的产物。
设置任一产物路径即进入此模式，四项必须齐全，不会重新构建本地源码。

- 依赖关系是 dapp->frame(services)->kernel services->kernel modules->components
- 模块划分的思路：保持Kernel的简洁，能在上层完成的功能就不要在底层完成。

## 模块命名

- 使用'_'分割单词的模块名，通常是一个完整的软件，而使用'-'分割单词的模块，通常是一个库。
- 如果模块以'daemon'结尾，说明该模块可以脱离buckyos zone运行，是一个本地服务.
- 以'service'结尾，说明该模块是一个标准的buckyos in-zone service.
- 以'server'结尾，说明该模块是一个传统的server,可以在buckyos zone外运行，提供广泛的服务.

## components 目录

- 以rust静态库形式存在，可以被复用的，并不是为buckyos设计的通用库。通常是一些标准协议的解析和使用的客户端库。
- 如果一个功能必须在buckyos里才能运行，那这个功能就不应该放在components里。

## kernel 目录

- 包含kernel modules 和 kernel services. 所有zone启动前可以工作的服务都在这个目录。
- 这个目录下的组件基本都是buckyos的专用组件，使用rust开发。

## frame 目录

services是以用户态提供系统的一些可扩展(可缺失)的通用功能。services一般运行在容器中，并有机会获得root权限。
services一般基于SDK开发。
我们现在最重要的一个service是K8S

## apps

apps是用户态的应用程序，apps一般运行在容器中，不会获得root权限。
app可以根据拉起方式继续细分种类。aios目前有两种
    1. app services （根据配置保持固定数量的实例）
    2. agent app (通过Agent Msg / event拉起)

app，尤其是agent app基于SDK开发。

## tools 目录

tools是一些开发辅助性的工具，通常是用来调试和管理系统的。
管理系统的工具，通常是app
而开发工具 ，通常可以不视作buckyos的一部分

## 独立产品目录

### cyfs-gateway 目录

### web3_bridge 目录
