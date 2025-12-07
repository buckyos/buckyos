# dev_config目录介绍

每个典型环境是一个目录，该目录下保存了一个典型的“分布式测试环境“所需要的全部信息和配置。
- nodes.json 定义了该环境下有多少个vm,格式是vm_name->vm_config,vm_config中可以引用vm模版(mutilpass格式)
- apps目录 重要，里面有$appname.json ,定义了一组app的基本行为
- $vm_template.yaml 用于初始化的模版
注意当前开发机是ood1@test.buckyos.io, 总是以无SN WLAN节点的角色存在的

- 2zone_sn : 最常用的环境，包含3个虚拟机节点 SN + Alice.ood1(端口映射) + bob.ood1(LAN)

## VM 环境

### 硬件环境配置（通常可以有多套）
- vm_config.json (配置vm环境)
- vm_init.yaml 

### 基础软件环境
- 有一些配置依赖已经创建的VM的ip地址，因此顺序上需要等vm node instance先启动得到ip后才能继续
- 构造iptable规则
- 预安装的ca证书 也可以生成 

## 部署软件（开发环境相关)
### 理解app_list.json

### Step1. 构建
### Step2. 根据node-name，构造配置(rootfs)
### Step3. 推送到目标node

结束环境构造，此时得到一组运行中的虚拟机 （处于Init状态)

main.py $group_name clean_vms
main.py $group_name create_vms


----------------- 开发循环 ----------------
`利用虚拟机的快照优势提高开发速度`

1. 创建未部署软件的快照点 init
main.py $group_name snapshot init
 
2. 部署最新版本的软件，测试用例和配置 installed
main.py $group_name install --all
main.py $group_name snapshot installed
3. 按测试需要启动软件 started
main.py $group_name start --all
main.py $group_name snapshot started

loop:
    4.1 回滚到快照started
    main.py $group_name restore started
    4.2 执行测试用例
    main.py #groupname run $node_id /opt/testcases/xxx.py


### 更新软件
main.py $group_name update --all 

### 更新配置（重装）
main.py $group_name restore init
main.py $group_name install --all
main.py $group_name snapshot installed


## 构造并运行测试用例
- 不同的测试用例有不同的基础软件需求

## 收集日志
main.py #$group_name clog

## 查看app状态
main.py $group_name info