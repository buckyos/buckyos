# quick start


（cargo build的过程中要根据提示安装一些apt 依赖，待补充）
```bash
cd ./src
python3 reinstall.py
python3 build.py
python3 make_deb.py

apt install ./buckyos.deb
```
构建完成后还会在web3_bridge目录写得到web3.buckyos.io的服务（all in one）

如果目标系统是ARM64,则先准备openssl的编译（具体方法可以看build_arm.py的注释）
```
cd ./src
python3 build_arm.py
python3 make_deb_arm.py
```
随后会得到buckyos_aarch64.deb，安装即可


开发环境下，直接执行 /opt/buckyos/bin/node_daemon --eanble-active即可
执行build.py会把编译结果放到/opt/buckyos/bin/目录下
/opt/buckyos/etc/下的node_identity.toml 和 node_private_key.pem是默认身份的配置文件，可以方便测试。（OOD会直接启动），将这两个文件删掉就会可以进入ip:3180/index.html的激活服务，通过UI来创建身份文件。

