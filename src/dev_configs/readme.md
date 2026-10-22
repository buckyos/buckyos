# Overview of the `dev_configs` Directory

This directory holds the VM-based distributed test environments driven by `buckyos-devtest` (from `buckyos-devkit`). The full workflow, troubleshooting notes and test cases are in [doc/CI/基于VM的开发环境构造.md](../../doc/CI/基于VM的开发环境构造.md).

- `<group>.json`: one workspace group (a typical topology). `nodes` maps node id -> VM config (template, disk, init/instance commands, apps and their params); `app_configs` can pull app definitions from sibling repos.
- `apps/<app>.json`: how an app is built on the host, pushed, installed, started and stopped on a node.
- `templates/<vm_template>.yaml`: Multipass cloud-init templates.
- `aicc.json`, `msg_center.json`: service configuration samples, not devtest groups.

Groups:

- `sntest`: the most commonly used environment, 2 VMs: SN (`web3-gateway` from the sibling `cyfs-gateway` repo) + `alice-ood1` (OOD behind a simulated NAT, reached through the SN relay).
- `full`: SN + `web` + `alice-ood1` + `bob-ood1` + `charlie-ood1`.
- `ssotest`: SN + three Zones for cross-Zone tests such as DID federation: `alice-ood1` (`alice.web3.devtests.org`, NAT, reached only through the SN relay), `bob-ood1` (`bob.web3.devtests.org`, `wan_dyn`, DNS points to its reported address) and `charlie-ood1` (`charlie.me`, portmap with RTCP on 2981, DNS points to its reported address). OODs get 2 CPU / 4G / 16G. The PVE backend defaults to a 900 s guest-agent timeout; the group needs no override. Owners sign in with `bucky2025`. Materialization rejects an old seed's `admin_password_hash` if it does not match that owner and password: generate OOD and SN together with a fresh `--env_root`. Tests: `test/test_hybrid_sso/test_hybrid_sso_federation_dv.ts` (backend) and `src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts` (browser); run commands are in their headers.

On a PVE host select the backend first; the image is the Ubuntu cloud image and VMs bridge to `vmbr0`:

```
export BUCKYOS_VM_BACKEND=pve
export BUCKYOS_PVE_VM_PARAMS='{"image": "/var/lib/vz/template/iso/noble-server-cloudimg-amd64.img", "storage": "local-lvm"}'
```

The host does not use the SN for DNS: point clients at the SN (`dig @<sn ip> <host>`, Node `dns.Resolver`) or map hosts to IPs (Chromium `--host-resolver-rules`). To exercise relay paths, explicitly map Alice and Charlie to the SN IP; normal Bob/Charlie DNS answers exercise direct access. Trust the development CA for browser tests: certificate-error retries can greatly increase the number of TLS connections and trigger RTCP admission limits. RTCP retries temporary admission failures within the stream-open deadline.

Note: the current development machine is `ood1@test.buckyos.io`, and it always acts as a WAN node without SN.

## Dev Loop

Run from `buckyos/src`. Snapshots keep VM IPs; recreating VMs changes them, so reinstall after `create_vms`.

```
uv run buckyos-devtest $group_name clean_vms
uv run buckyos-devtest $group_name create_vms
uv run buckyos-devtest $group_name snapshot init

uv run ./build_for_vm_test.py
uv run buckyos-devtest $group_name install
uv run buckyos-devtest $group_name snapshot installed

uv run buckyos-devtest $group_name start
```

loop:
```
uv run buckyos-devtest $group_name restore installed
uv run buckyos-devtest $group_name start
uv run buckyos-devtest $group_name run $node_id "<command>"
```

Update software after `uv run ./build_for_vm_test.py`:
```
uv run buckyos-devtest $group_name update
```

Update config (reinstall):
```
uv run buckyos-devtest $group_name restore init
uv run buckyos-devtest $group_name install
uv run buckyos-devtest $group_name snapshot installed
```

Collect logs into `/tmp/clogs/<node_id>/`:
```
uv run buckyos-devtest $group_name clog
```

Show VM IPs:
```
uv run buckyos-devtest $group_name info_vms
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
- Uses a custom domain with mapped HTTP/HTTPS ports and RTCP on 2981 (D-DNS)
- Configures `sn: sn.devtests.org` in `zone_boot`
- Sets `rtcp_port` to 2981 in the device document and mini document; the local gateway listens on the same port
- ood1's netid is `portmap`
- Requires SN

### Missing case:
OOD netid is WAN, but it uses an SN second-level domain (an extreme case: someone has a VPS but no domain).
Just call `register_device_to_sn` / `update_device_info_to_sn` at the appropriate time.

## Zone Access Logic
### Access via HTTPS
- DNS resolution: decide based on whether the domain's NS records point to SN (if it is a `*.web` second-level domain, it always goes through SN)
- SN uses the explicit network type: `wan`, `wan_dyn`, and `portmap` return the reported device IP when available; NAT, unknown network types and missing addresses return the SN IP
- If DNS returns an SN address, traffic is relayed; otherwise it is a public IP or port mapping

### Access via RTCP
- `resolve_did` to get `zone_boot_config`
- Direct candidates use the device DID and its declared `rtcp_port` (`2980` if omitted).
- `wan_dyn` uses SN for DDNS only and omits `sn` from its boot and Zone documents. NAT and `portmap` retain an explicit SN relay candidate; the SN derives the device identity from the Zone's own DID, including `did:web` custom domains.

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
