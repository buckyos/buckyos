
# 配置文件生成或来源


## nodeA2
使用的是`rootfs/etc` 的配置文件

## nodeB1

(这里是描述nodeB1的配置如何生成，项目仓库里已经存在配置文件，不需要去运行)

运行
```
cargo test --package name-lib --lib -- config::tests::create_test_env_configs --exact --show-output 
```
然后将生成于 `/tmp/buckyos_dev_configs`的文件覆盖拷贝到 `src/scripts/remote/dev_configs`下

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
1.  安装multipass
2.  网络：执行`main.py network`, 生成并检查网络（br-sn）
3.  创建VM： 执行 `main.py create` 启动vm(sn, nodeA2, nodeB1)
4.  编译buckyos，（执行 `/src/scripts/build.py`） vm是ubuntu的，这里的编译出来的二进制，需要对应得上。
5.  执行 `main.py install --all`
6.  执行 `main.py active_sn`， 把sn的配置和db复制到sn vm里面
7.  关闭sn vm上面的`systemd-resolved`程序，否则会因为占用53端口问题，导致sn的gateway进程中的dns_server启动失败 
     `sudo systemctl stop systemd-resolved`
     `sudo systemctl disable systemd-resolved`
8.  执行 `main.py start_sn`, 单独启动sn
9.  执行active，激活其他的node，把A和B的配置分别复制到 nodeB1 和 nodeA2 里面, 并且会修改这两个vm里面的DNS 服务，指向SN
10.  测试sn配置和dns配置是否成功： 执行 `multipass exec nodeB1 -- dig bob.web3.buckyos.io` 查看改了DNS后，能否解析到。
11. 启动nodeA2 和 nodeB1
12. (只支持Linux) 模拟网络隔离：执行bash network.sh 


# 注意事项

使用`dev_configs/ssh/id_rsa` 这个私钥的时候，可能会出现提示私钥权限过大，需要手动修改。
`chmod 600 dev_configs/ssh/id_rsa`

如果是在wsl环境，并且项目在/mnt/xxx目录下，wsl无权修改windows目录下的文件，会无法修改私钥文件

这个私钥文件跟ood无关，不是ood的身份文件。
是给vm用的（可以分离multipass，单独使用ssh）


