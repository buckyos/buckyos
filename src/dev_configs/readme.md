# dev_config目录介绍

每个典型环境是一个目录，该目录下保存了一个典型的“分布式测试环境“所需要的全部信息和配置。
- nodes.json 定义了该环境下有多少个vm,格式是vm_name->vm_config,vm_config中可以引用vm模版(mutilpass格式)
- apps目录 重要，里面有$appname.json ,定义了一组app的基本行为
- $vm_template.yaml 用于初始化的模版
注意当前开发机是ood1@test.buckyos.io, 总是以无SN WAN节点的角色存在的

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


## 一些典型的用户设计（尽量用最少的用户覆盖典型情况）

### ood1@test.buckyos.io (owner did:bns:devtest) 
- 开发机（不会被部署在虚拟机中)
- 公网节点，完全不依赖SN

### sn_server@sn.devtests.org (owner did:bns:devtests)
- SN 服务，并不运行完整的buckyos

### ood1@devtests.org (owner: did:bns:devtests),这个节点通常叫sn_web
- devtests的标准OOD，
- 提供Repo source服务

### node1@test.buckyos.io (owner did:bns:devtest)
- 非OOD节点
- 在NAT后

### ood1@alice.web3.devtests.org (owner did:bns:alice)
- 标准的内网nat节点 （流量全转发）

### ood1@bob.web3.devtests.org (owner did:bns:bob)
- 打开了443、80、2980 的标准端口映射 （D-DNS）

### ood1@charlie.me (owner did:bns:charlie)
- 使用自有域名，使用自定义的2981端口映射 （D-DNS,rtcp流量不转发，其它转发）


## 访问zone的逻辑
### 通过https访问
- DNS解析，通过域名的NS记录是否指向SN判断 （如果是*.web的二级域名，必然走SN）
- SN 判断是该zone 是否需要中转http流量（net_id是wan，返回设备的ip,否则返回sn ip）
- DNS解析返回的地址，如果是SN，则走流量中转，否则就是 公网IP或端口映射

### 通过rtcp访问 （目前还没实现)
- resolve_did,得到zone_boot_config
- 当OOD net_id是wan或portmap时，直连：(rtcp://device_did/xxxx)  
- 当OOD net_id不是waln,且有SN时，中转(rtcp://sn/device_did/xxxx)

### 通过rtcp访问zone内任意节点（目前未实现）

## 一些和通信模型有关的代码
```rust
// node_daemon 判断是否需要和sn keep-tunnel
let mut need_keep_tunnel_to_sn = false;
if sn.is_some() {
    need_keep_tunnel_to_sn = true;
    if device_doc.net_id.is_some() {
        let net_id = device_doc.net_id.as_ref().unwrap();
        if net_id == "wan" {
            need_keep_tunnel_to_sn = false;
        }
    }
}

if need_keep_tunnel_to_sn {
    let device_did = device_doc.id.to_string();
    let sn_host_name = get_real_sn_host_name(sn.as_ref().unwrap(),device_did.as_str()).await
        .map_err(|err| {
            error!("get sn host name failed! {}", err);
            return String::from("get sn host name failed!");
        })?;
    params = vec!["--keep_tunnel".to_string(),sn_host_name.clone()];
} else {
    params = Vec::new();
}
```
```rust
//node_daemon 判断是否需要上报device_info
async fn report_ood_info_to_sn(device_info: &DeviceInfo, device_token_jwt: &str,zone_config: &ZoneConfig) -> std::result::Result<(),String> {
    let mut need_sn = false;
    let mut sn_url = zone_config.get_sn_api_url();
    if sn_url.is_some() {
        need_sn = true;
    } else {
        if device_info.ddns_sn_url.is_some() {
            need_sn = true;
            sn_url = device_info.ddns_sn_url.clone();
        }
    }
    if !need_sn {
        return Ok(());
    }
}
```

```rust
// active-server构造device_info
   async fn handel_do_active(&self,req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let gateway_type = req.params.get("gateway_type");
        let sn_url_param = req.params.get("sn_url");
        let mut sn_url:Option<String> = None;
        if sn_url_param.is_some() {
            sn_url = Some(sn_url_param.unwrap().as_str().unwrap().to_string());
        }
        //create device doc ,and sign it with owner private key
        //create device doc ,and sign it with owner private key
        match gateway_type {
            "BuckyForward" => {
                net_id = None;
            },
            "PortForward" => {
                net_id = Some("wan".to_string());
            },
            _ => {
                return Err(RPCErrors::ReasonError("Invalid gateway type".to_string()));
            }
        }

        let mut device_config = DeviceConfig::new_by_jwk("ood1",device_public_jwk);
        device_config.net_id = net_id;
        device_config.ddns_sn_url = ddns_sn_url;
        device_config.support_container = is_support_container;
        device_config.iss = user_name.to_string();
        
        let device_doc_jwt = device_config.encode(Some(&owner_private_key_pem))
            .map_err(|_|RPCErrors::ReasonError("Failed to encode device config".to_string()))?;
        
        if sn_url.is_some() {
            if sn_url.as_ref().unwrap().len() > 5 {
                need_sn = true;
            }
        }
        
        if need_sn {
            let sn_url = sn_url.unwrap();
            info!("Register OOD1(zone-gateway) to sn: {}",sn_url);
            let rpc_token = ::kRPC::RPCSessionToken {
                token_type : ::kRPC::RPCSessionTokenType::JWT,
                nonce : None,
                session : None,
                userid : Some(user_name.to_string()),
                appid:Some("active_service".to_string()),
                exp:Some(buckyos_get_unix_timestamp() + 60),
                iss:Some(user_name.to_string()),
                token:None,
            };
            let user_rpc_token = rpc_token.generate_jwt(None,&owner_private_key_pem)
                .map_err(|_| {
                    warn!("Failed to generate user rpc token");
                    RPCErrors::ReasonError("Failed to generate user rpc token".to_string())})?;
            
            let mut device_info = DeviceInfo::from_device_doc(&device_config);
            device_info.auto_fill_by_system_info().await.unwrap();
            let device_info_json = serde_json::to_string(&device_info).unwrap();
            let device_ip = device_info.ip.unwrap().to_string();
            let mini_config_jwt = "todo".to_string();
            
            let sn_result = sn_register_device(sn_url.as_str(), Some(user_rpc_token), 
                user_name, "ood1", &device_did.to_string(), &device_ip, device_info_json.as_str(),&mini_config_jwt).await;
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!("Failed to register device to sn: {}",sn_result.err().unwrap())));
            }
        }
```


