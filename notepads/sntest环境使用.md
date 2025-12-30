## sn虚拟机测试环境使用
- 环境名为sntest
- 需要经常在buckyos (ood) + cyfs-gateway(sn) 两个工程中切换
- 测试的时候，开发机要设置一下sn的host到虚拟机的sn节点。我有一个快速设置脚本src/set_host.py $sn_ip

## 构建

在buckyos工程中
```
buckyos-build
buckyos-install
```

然后在cyfs-gateway工程中
```
buckyos-build
buckyos-install
```

上面任何项目的代码更新了，都要在相关项目中build & install

## 运行测试
所有虚拟机的控制都在buckyos工程下，切记！

```
buckyos-devtest sntest create_mvs
buckyos-devtest sntest install
```

## 进行测试
1. 启动SN 

```
buckyos-devtest sntest start --app=web3-gateway
```
不过我更喜欢ssh到sn vm上去
```
sudo python3 /opt/web3-gateway/start.py
```

2. 然后去alice-ood1上删除node-ideneity文件,并启动node_daemon
```
sudo rm /opt/buckyos/etc/node_identity.json
sudo /opt/buckyos/bin/node-daemon/node_daemon --enable_active
```

3. 在开发机上启动浏览器，输入alice-ood1的ip:3182,进行激活

## 更新代码的操作
0. 记得先停止
1. 在代码更新的repo `buckyos-build, buckyos-install`
2. 回到buckyos目录，根据更新的项目，选择app
```
buckyos-devtest sntest update --app=web3-gateway 
buckyos-devtest sntest update --app=buckyos
```
