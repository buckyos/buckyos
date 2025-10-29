# BuckyOS Test Plan 

## 测试分类
- 基础组件，cargo test
所有的相关开发工作，都应该可以通过cargo test进行验证

- 基础协议（协议实现等），完善cargo test
主要在cyfs-gateway repo里完成测试
需要规划一些基础的框架(在cargo test中运行特定server),并在此基础上编写cargo test

- buckyos的基础功能
依赖一个运行中的单节点buckyos
在单节点测试(devtest)中完成，代码在test目录下,使用pytest驱动

- buckyos的复杂功能

在标准分布式开发环境中完成，代码在cluster-test目录下，使用pytest驱动

## 基础组件
### buckyos-kit

### name-lib

### rbac
- 完整的系统权限设计文档
- 基于上述文档的每条规则，构造 3条允许的测试和3条不允许的测试
- 基于bug,构造被遗漏的错误配置的测试

## 基础协议
### name-client

### kRPC

### ndn-lib
- 完善完整的协议文档
- 完善named_mgr的设计文档，这关系到数据落地的格式和向下兼容能力

### package-lib
- 完成pkg-env的设计文档，这关系到数据落地的格式和向下兼容的能力
- 所有测试有2份，1份是在独立的pkg_env中完成，另一份是在有parent的pkg env中完成

### cyfs-gateway
cyfs-gateway是一个极其复杂的组件，有专门的文档描述其测试

## buckyos的基础功能

### 调度器

### system-config

### SN

### repo

### smb
- 通过命令行激活后，验证smb可以访问


## buckyos的复杂功能

### 激活逻辑

### SN

### repo
- 从remote source上更新新版本的pkg