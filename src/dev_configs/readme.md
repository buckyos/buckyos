# Overview of the `dev_configs` Directory

Each typical environment is a directory that holds all information and configuration needed for a typical "distributed test environment".
- `nodes.json` defines how many VMs exist in this environment. Format: `vm_name -> vm_config`. A `vm_config` can reference a VM template (Multipass format).
- `apps` directory (important): contains `$appname.json` files that define basic behavior for a set of apps.
- `$vm_template.yaml` is the template used for initialization.
Note: the current development machine is `ood1@test.buckyos.io`, and it always acts as a WAN node without SN.

- `2zone_sn`: the most commonly used environment, with 3 VM nodes: SN + Alice.ood1 (port mapping) + bob.ood1 (LAN)

## VM Environment

### Hardware environment config (usually multiple sets can exist)
- `vm_config.json` (VM environment config)
- `vm_init.yaml`

### Base software environment
- Some configs depend on IPs of already-created VMs, so you must wait until VM node instances start and get IPs before continuing
- Build iptables rules
- Pre-installed CA certificates (can also be generated)

## Deploy Software (dev-environment related)
### Understanding `app_list.json`

### Step1. Build
### Step2. Build config (rootfs) based on node-name
### Step3. Push to the target node

Environment setup is complete; you now have a set of running VMs (in Init state).

```
main.py $group_name clean_vms
main.py $group_name create_vms
```


----------------- Dev Loop ----------------
`Use VM snapshots to speed up development`

1. Create a snapshot before software is deployed: `init`
```
main.py $group_name snapshot init
```

2. Deploy the latest software, test cases, and config: `installed`
```
main.py $group_name install --all
main.py $group_name snapshot installed
```
3. Start software as needed for tests: `started`
```
main.py $group_name start --all
main.py $group_name snapshot started
```

loop:
```
    4.1 Restore to the `started` snapshot
    main.py $group_name restore started
    4.2 Run test cases
    main.py #groupname run $node_id /opt/testcases/xxx.py
```


### Update software
```
main.py $group_name update --all
```

### Update config (reinstall)
```
main.py $group_name restore init
main.py $group_name install --all
main.py $group_name snapshot installed
```


## Build and Run Test Cases
- Different test cases have different base software requirements

## Collect Logs
```
main.py #$group_name clog
```

## Check App Status
```
main.py $group_name info
```


## Typical User Designs (cover typical cases with as few users as possible)

### ood1@test.buckyos.io (owner did:bns:devtest)
- Development machine (not deployed inside a VM)
- Public WAN node, fully independent of SN
- netid: wan

### sn_server@sn.devtests.org (owner did:bns:devtests)
- SN service; does not run a full BuckyOS stack

### ood1@devtests.org (owner: did:bns:devtests), commonly called `sn_web`
- Standard OOD for `devtests`
- Provides Repo source service

### node1@test.buckyos.io (owner did:bns:devtest)
- Non-OOD node
- Behind NAT (`netid:nat`)
- Requires SN

### ood1@alice.web3.devtests.org (owner did:bns:alice)
- Standard LAN NAT node (all traffic forwarded), `netid:nat`
- Configures `sn: sn.devtests.org` in `zone_boot`
- Requires SN

### ood1@bob.web3.devtests.org (owner did:bns:bob)
- Standard port mapping for 443, 80, and 2980 (D-DNS)
- No SN configured in `zone_boot`
- In ood1's `device_doc`, configures `ddns_sn_url: sn.devtests.org`
- ood1's netid is `wan_dyn`
- Requires SN

### ood1@charlie.me (owner did:bns:charlie)
- Uses a custom domain and custom port mapping on 2981 (D-DNS; RTCP traffic is not forwarded, other traffic is)
- Configures `sn: sn.devtests.org` in `zone_boot`
- Sets `rtcp_port` to 2981 in `device_mini_config`
- ood1's netid is `portmap`
- Requires SN

### Missing case:
OOD netid is WAN, but it uses an SN second-level domain (an extreme case: someone has a VPS but no domain).
Just call `register_device_to_sn` / `update_device_info_to_sn` at the appropriate time.

## Zone Access Logic
### Access via HTTPS
- DNS resolution: decide based on whether the domain's NS records point to SN (if it is a `*.web` second-level domain, it always goes through SN)
- SN decides whether this zone needs HTTP traffic relay (`net_id` is `wan` → return device IP; otherwise return SN IP)
- If DNS returns an SN address, traffic is relayed; otherwise it is a public IP or port mapping

### Access via RTCP (not implemented yet)
- `resolve_did` to get `zone_boot_config`
- When OOD `net_id` is `wan` or `portmap`, connect directly: `(rtcp://device_did/xxxx)`
- When OOD `net_id` is not `wan` and SN exists, relay: `(rtcp://sn/device_did/xxxx)`

### Access any node in the zone via RTCP (not implemented yet)

## Code Related to the Communication Model
```rust
// node_daemon decides whether it needs to keep a tunnel to SN
let mut need_keep_tunnel_to_sn = false;
if sn.is_some() { // sn comes from zone_config
    need_keep_tunnel_to_sn = true;
    if device_doc.net_id.is_some() {
        let net_id = device_doc.net_id.as_ref().unwrap();
        if net_id == "wan" {
            need_keep_tunnel_to_sn = false;
        }
    }
}

if need_keep_tunnel_to_sn {
    let zone_info = load_sn_zone_info()?;
    let relay_node = zone_info
        .and_then(|info| info.relay_sn)
        .or_else(|| sn.clone());
    params = relay_node.into_iter().collect();
} else {
    params = Vec::new();
}
```
```rust
// node_daemon decides whether it needs to report device_info
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
// active-server builds device_info
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
            let device_info_json = serde_json::to_value(&device_info).unwrap();
            let device_ip = device_info
                .all_ip
                .first()
                .or_else(|| device_info.ips.first())
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string());
            
            let sn_result = sn_register_device_online(
                sn_url.as_str(),
                user_rpc_token,
                SnDeviceOnlineReportReq {
                    device_name: "ood1".to_string(),
                    device_did: Some(device_did.to_string()),
                    device_ip,
                    device_info: device_info_json,
                    endpoints: Vec::new(),
                    report_seq: None,
                    ttl: None,
                },
            ).await;
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!("Failed to register device to sn: {}",sn_result.err().unwrap())));
            }
        }
```
