
# 配置文件生成或来源


## nodeA2
使用的是`rootfs/etc` 的配置文件

## nodeB1

(这里是描述nodeB1的配置如何生成，项目仓库里已经存在配置文件，不需要去运行)

运行
```
cargo test --package name-lib --lib -- config::tests::create_test_env_configs --exact --show-output 
```
然后将生成于 /tmp/buckyos_dev_configs的文件覆盖拷贝到 src/scripts/remote/dev_configs下

## sn



# 结构 描述

|----DEV (192.0.2.1) 
|
|----VM_SN (192.0.2.2)
     |
     |---- NAT1 ---- VM_NODE_A2 (10.0.1.2)
     |
     |---- NAT2 ---- VM_NODE_B1 (10.0.2.2)


# step
1. Check the multipass cmd
2. 生成并检查网络（br-sn）
3. 执行 create 启动vm(sn, nodeA2, nodeB1)
4. 编译buckyos，（执行 /src/scripts/build.py） vm是ubuntu的，这里的编译出来的二进制，需要对应得上。
5. 执行 install 
6. 执行active_sn， 把sn的配置和db复制到sn vm里面
7. 启动sn
8. 执行active，激活其他的node，把A和B的配置分别复制到 nodeB1 和 nodeA2 里面, 并且会修改这两个vm里面的DNS 服务，指向SN
9. 执行 `multipass exec nodeB1 -- dig bob.web3.buckyos.io` 查看改了DNS后，能否解析到。
10. 启动nodeA2 和 nodeB1

