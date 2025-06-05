

# 测试环境描述
标准分布式开发测试环境含有3个Zone
A: test.buckyos.io
B: bob.web3.buckyos.io
SN: sn.buckyos.io / web3.buckyos.io

## 配置文件生成或来源
### devtest.ood1(A1)
使用的是 rootfs/etc下的配置

### devtest.node1(A2)

非OOD节点,该节点暂时未启用

### bob.ood1 B1
身份和设备配置文件不是通过正常流程激活，由test脚本生成

### sn
测试环境sn的db由test脚本生成


# 结构 描述

|----DEV (192.0.2.1) 
|
|----VM_SN (192.0.2.2)
     |
     |---- NAT1 ---- VM_NODE_A2 (10.0.1.2)
     |
     |---- NAT2 ---- VM_NODE_B1 (10.0.2.2)


## step
1.  安装multipass

### 自动创建虚拟机
因为涉及到一些网络调用，所以目前是Linux Only的，其它平台要手工创建合适的虚拟机


2.  检查网络环境, 根据情况，修改 dev_vm_config.json里的bridge字段。
可以设置为 multipass 默认创建的`mpqemubr0
3.  创建VM： 执行 `main.py create` 启动vm(sn, nodeA2, nodeB1)

### 构造将要部署到虚拟机中的rootfs
建议在本步执行前手工创建VM checkpoint,方便在测试结束后快速回滚到起点

4.  编译buckyos，（执行 `/src/scripts/build.py`） vm是ubuntu的，这里的编译出来的二进制，需要对应得上。
5.  执行 `main.py install --all`
6.  执行 `main.py active_sn`， 把sn的配置和db复制到sn vm里面


### 按顺序启动集群中的服务 
7.  执行 `main.py start_sn`, 单独启动sn
8.  执行 `main.py active --all`，这里会将其他node激活，把A和B的配置分别复制到 nodeB1 和 nodeA2 里面, 并且会修改这两个vm里面的DNS 服务，指向SN
9.  测试sn配置和dns配置是否成功： 
     执行 `multipass exec nodeB1 -- dig bob.web3.buckyos.io` 
     查看是否成功获得解析结果
10. 启动nodeA2 和 nodeB1
     执行 `multipass shell nodeB1`
     ` /opt/buckyos/bin/buckycli/buckycli sys_config --get  boot/config`
     查看是否执行成功，并获得config结果
     如果成功，则nodeB1 启动成功
11. (只支持Linux) 模拟网络隔离：执行bash network.sh 


## 注意事项

### 执行创建失败
出现错误提示，`Error: launch failed: Remote "" is unknown or unreachable.`
可能是vm的镜像服务器链接不上，可等待后重试，或者手动切换镜像服务。

### vm的ssh 密钥
使用`dev_configs/ssh/id_rsa` 这个私钥的时候，可能会出现提示私钥权限过大，需要手动修改。
`chmod 600 dev_configs/ssh/id_rsa`

如果是在wsl环境，并且项目在/mnt/xxx目录下，wsl无权修改windows目录下的文件，会无法修改私钥文件

这个私钥文件跟ood无关，不是ood的身份文件。
是给vm用的（可以分离multipass，单独使用ssh）




### bob.ood1 B1 配置文件的生成
```
cargo test --package name-lib --lib -- config::tests::create_test_env_configs --exact --show-output 
```

把 `/tmp/buckyos_dev_configs`的文件覆盖拷贝到 `src/scripts/remote/dev_configs`下
