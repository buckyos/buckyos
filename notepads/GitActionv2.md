# GitAction V2

目的：Git Action用来完成系统的“正确性“快速检查，尤其是方便任何开发者，在不搭建多平台环境时，对自己的修改进行验证

## 基本流程

- 我们有5个环境，选择正确的基础镜像
  - 通用:rust,python,buckyos-devtool,nodejs,docker构建工具
  - buckyos app: osx/windows 下需要，安装tauri
  - 安装包制作:windows下需要 nsis
- 先对buckyos 进行cargo test -- -thread=1, 进行快速的单元测试检查 （在手工运行时该步骤可以手工跳过）

## 构建与单点测试

- git clone cyfs-gateway
- git clone buckyosapp(windows/osx平台)
- 此时目录是  ~/buckyos, ~/cyfs-gatway, ~/buckyosapp
- 在 ~/cyfs-gatway 目录下执行 buckyos-build && buckyos-install
- 在 ~/buckyos 目录下执行 buckyos-build && buckyos-install
- 在 ~/buckyos 目录下执行 start.py --all ,启动标准单点测试环境
- 运行单点测试（一般在 ~/buckyos/src/test）目录下，目前有一个demo用的test_rbac


## 构建安装包并对安装包进行测试

- 在~/buckyos目录下执行 stop.py
- rm -rf BUCKYOS_ROOT,清理上一次单点测试的影响
- 在~/buckyos目录下执行 make_local_pkg.py build-pkg ,构造本平台安装包(这个脚本内部会判断平台走不同的实现)
- 构造完成后，调用 make_local_pkg.py verify-pkg $last_pkg 进行验证
- 立刻进行静默安装验证，用默认安装能安装成功
- 判断http://127.0.0.1:3182/index.html 可以获得成功
- GitAction结束

## 产物

如果流程出错，那么打包 $BUCKYOS_ROOT/logs ，供后续诊断
如果流程成功，那么产物就是刚刚构建的本地安装包(local_pkg)