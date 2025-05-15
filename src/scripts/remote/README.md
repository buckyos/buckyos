
# 配置文件生成
通过运行
```
cargo test --package name-lib --lib -- config::tests::create_test_env_configs --exact --show-output 
```
然后将生成于 /tmp/buckyos_dev_configs的文件覆盖拷贝到 src/scripts/remote/dev_configs下


# vm 描述

|----DEV (192.0.2.1) 
|
|----VM_SN (192.0.2.2)
     |
     |---- NAT1 ---- VM_NODE_A2 (10.0.1.2)
     |
     |---- NAT2 ---- VM_NODE_B2 (10.0.2.2)


# step
1. generate ssh key(put into ~/.buckyos_dev/id_rsa)， generate pub key into vm_init.yaml
2. Check the multipass cmd,and check the network environment (br0)
3. Check the multipass command 
4. Install the virtual machine and generate deviceinfo information (device_info.json)
5. VM install buckyos （'-i ~/.buckyos_dev/id_rsa')
6. VM start buckyos
7. Execute other commands (optional)
