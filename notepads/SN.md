## 用户什么时候需要SN？ 
    没有自己的顶级域名：需要靠SN获得子域名，并依靠SN的域名解析服务
    Zone没有公网IP
    Zone没有固定公网IP



## SN是一个Zone内可访问的ZoneConfig，（不需要配置在域名中）
配置后,系统里的ZoneGateway要定时向SN上报信息，以
        支持DDNS
        支持ACMC挑战，获得证书
系统里的ZoneGateway有需要的时候要与SN keep-tunnel

## cyfs-gateway的dns服务可以关闭，但默认是打开的
打开后可以支持正确的，自己的DNS信息

### 通过配置文件继承机制改进一下gateway的配置生成（如果要改就一起改了）


TODO：
1. 首先，明确SN的配置是在激活产生的start_config里的 OK 
sn_url的意义是sn_api_url
sn_url : 之前是 "http://web3.buckyos.io/kapi/sn"; , 新的是"http://web3.buckyos.ai/kapi/sn";

修改active_page 通过页面设置修改SN地址 OK 

2. 检查系统判断自己和SN的逻辑 OK
if ZoneConfig.sn_url.is_some() {
    if ood1.device_config.net_id != "WLAN" {
        keep_tunnel_to_sn()
    }
    report_ood_info_to_sn()
}

未设置SN_URL,且netid为WLAN的OOD1，不会有任何SN逻辑

report_ood_info_to_sn() {
    1.与SN通信后，SN可以正确的得到ood的当前WAN地址
    2.上报一些设备信息
}


3. SN-Server修改
resolve-did的时候能基于pkx直接得到验证后的zone_config OK 
    同时支持zone_config和device_config的逻辑还是要想一下
    建立tunnel需要device私钥而不是zone owner的私钥，这个会用到auth_key_list
SN服务器应在域名中正确配置自己的PKX
init_without_buckyos里有写死自己的deivce_config(应该从路径中加载)

4.cyfs-dns修改
sn provicer能正确的产生包含pkx的DNS 记录 OK 
cyfs-gateway的标准dns也能正常工作（目前是没有启动的，需要完成Review)
    解析自己能得到正确的DID 字段
    能解析 devicename.zoneid


一些通用套路
1）目前TXT记录里有2条，1条是zoneconfig的jwt,1条是公钥
PKX=0:owner_pk;PKX=1:gateway_device_pk;
如果配置了owner_pk,那么会对zoneconfig的jwt进行校验
2）SN返回这两条记录的方法

## SN的首次部署:

- 上传 web3_bridge 文件夹到服务器的 /opt/web3_bridge
- 安装web3_bridge服务

```shell
cp web3_gateway.service /etc/systemd/system/
systemctl daemon-reload
```

- 上传真实环境的真实配置（注意运维环境配置和开发环境配置的隔离,下面文件都不会在GIT上看到

  - SN的zone config
  - SN的设备身份私钥（device_key.pem),注意配置device_did ，在SN的域名注册商上要配置2条记录，一条zoneconfigjwt,一条包含2个公钥（owner和gateweay device public key)。SN自己也是DNS服务器，因此当解析目标是自己时，也能正确返回这些DNS TXT Record
  - https证书文件:(看配置文件就知道)
  - sn_db.sqlite3  （记录了激活码，激活码使用情况，用户注册情况的数据库），该数据库要定期备份

- 启动服务

```
systemctl enable web3_gateway
systemctl start web3_gateway
```

## SN的更新

记得先停止服务。
大部分情况下，只需要更新二进制文件即可。
