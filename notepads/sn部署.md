## SN的首次部署:
- 上传 web3_bridge 文件夹到服务器的 /opt/web3_bridge
- 安装web3_bridge服务
```shell
cp web3_gateway.service /etc/systemd/system/
systemctl daemon-reload
```
- 上传真实环境的真实配置（注意运维环境配置和开发环境配置的隔离,下面文件都不会在GIT上看到）
0.SN的zone config
1. SN的设备身份私钥（device_key.pem),注意配置device_did ，在SN的域名注册商上要配置2条记录，一条zoneconfigjwt,一条包含2个公钥。SN自己也是DNS服务器，因此当解析目标是自己时，也能正确返回这些DNS TXT Record
2. https证书文件:(看配置文件就知道)
3. sn_db.sqlite3  （记录了激活码，激活码使用情况，用户注册情况的数据库），该数据库要定期备份

- 启动服务
```
systemctl enable web3_gateway
systemctl start web3_gateway
```

## SN的更新
记得先停止服务。
大部分情况下，只需要更新二进制文件即可。

