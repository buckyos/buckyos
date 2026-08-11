# sys_config_service TODO

## ZoneDidResolver: 让 `/1.0/identifiers/<did>` 真的能 resolve 到 device 自报 IP

**背景** —— `cyfs-gateway` 通过 node_daemon 启动后，会被注册一个本机 name provider
`http://127.0.0.1:3180/`（`HttpsProvider`，详见
`buckyos-base/name-client/src/https_provider.rs`，doc：
`buckyos/doc/arch/gateway/readme.md` 中"name provider 注册"小节）。
3180 上 `/1.0/identifiers/*` 会被 boot_gateway.yaml 转发到 system_config
service 的 `ZoneDidResolver`（`src/zone_did_resolver.rs`，挂在 `/`）。

意图是 `cyfs-gateway` 在 `name_client::resolve_ips(<remote>)` 时，能根据
device DID 拿到 device 自己上报的 `DeviceInfo.all_ip`。目前这条链路在
system_config 这一端有两个断点，注册过的 provider 只会一直返回 NotFound，
device 自报 IP 实际上根本进不到 gateway 的 `merged_ips`。

### 断点 1：`do_query_did` 没把 DID 归一化成 device 短名

`src/zone_did_resolver.rs` 中 `do_query_did` 把请求里的 `did_str` 原样
当成 SYS_STORE 的 key（`devices/<key>/doc`）。但 cyfs-gateway 侧 `name_lib`
会把任意主机名 fallback 成 `did:web:<host>`（见
`buckyos-base/name-lib/src/did.rs` 的 `from_host_name`，最后一行
`DID::new("web", host_name)`）。所以实际进来的 did_str 形如：

- `did:web:ood2`（短名直接被包成 web DID）
- `did:web:ood2.test.buckyos.io`（FQDN 被包成 web DID）
- `did:bns:<id>` / `did:dev:<id>` 等真正的 DID

而 device 在 SYS_STORE 里都是按短名存的：

- `devices/<short_name>/doc` —— scheduler 一次性写入的签名 JWT
  （`scheduler/src/system_config_builder.rs::add_device_doc`）
- `devices/<short_name>/info` —— node_daemon 周期性写入的运行时 DeviceInfo
  （`node_daemon/src/node_daemon.rs::update_device_info`，包含 `all_ip`）

修法：在 `do_query_did` 里先做归一化再去 SYS_STORE 查：

- `did:web:<id>` → 取 `<id>`，如果 `<id>` 含 `.` 且后缀是当前 zone host
  （从 `boot/config` 里读 zone_did + `to_host_name()` 得到），剥掉 zone 后缀
  得到短名；否则 `<id>` 本身就是短名。
- 其它 method 维持现在 agent / device / owner 三级 fallback，但 key 用
  规范化后的短名/id 而不是整个 did_str。
- 找不到再回 NotFound。

### 断点 2：`?type=info` 被丢掉，永远不会返回 DeviceInfo

`name_client::resolve_ips` 的 fallback 路径：

```rust
async fn resolve_device_info_ips(&self, name: &str) -> NSResult<Vec<IpAddr>> {
    let did = DID::from_str(name)?;
    let doc = self.resolve_did(&did, Some("info")).await?;
    Self::extract_device_info_ips(doc)  // 期望 doc 反序列化成 DeviceInfo
}
```

`HttpsProvider` 把这个 `doc_type` 编进 query：
`GET /1.0/identifiers/<did>?type=info`。

但 `do_query_did(did_str, typ)` 函数体内**完全没读 `typ`**，所有分支都走
`load_device_doc`/`load_agent_doc`/`load_owner_doc`。即使断点 1 修了，
`?type=info` 也只会拿回 `/doc` 的签名 JWT，反序列化成 `DeviceInfo` 会失败，
device 自报 IP 路径整个瘫掉。

而 `do_query_info`（同文件，下方）已经实现了"加载 `devices/<name>/info` 并
序列化成 DeviceInfo JSON"——但它从未被 HTTP handler 调到。

修法：`do_query_did` 头上判断 `typ.as_deref() == Some("info")`，命中就
走 `do_query_info(<归一化短名>)` 并直接把 JSON 字符串返回。其它 typ
（`None`、`"owner"` 等）维持现状。

### 验收

- 在 OOD 上 `curl http://127.0.0.1:3200/1.0/identifiers/did:web:<this_node_short_name>?type=info`
  应当返回 node_daemon 写入的 DeviceInfo JSON（含 `all_ip`）。
- 同一 zone 内另一台节点上跑
  `cyfs-gateway` 后通过本机 3180 转一次也应能拿到。
- `cyfs-gateway` 端 `name_client::resolve_ips("<peer-did-or-host>")` 能拿
  到 peer 自报的 IP，验证可以在 `gateway_tunnel_probe` 触发 RTCP 建链时
  看 cyfs-gateway 日志中 `https provider querying ...` 后是否成功解析。

### 相关引用

- 注册侧实现：`buckyos/src/kernel/node_daemon/src/gateway_name_provider.rs`
- name_client 调用方：`buckyos-base/src/name-client/src/name_client.rs`
  （`resolve_ips`、`resolve_signed_device_document_ips`、`resolve_device_info_ips`）
- HttpsProvider URL 构造：`buckyos-base/src/name-client/src/https_provider.rs`
  （`build_url` → `<base>/1.0/identifiers/<did>?type=<doc_type>`）
- 网关 forward：`buckyos/src/rootfs/etc/boot_gateway.yaml`
  `get_service_info_from_req` 把 `/1.0/identifiers/*` 命中到 `system_config`
- doc：`buckyos/doc/arch/gateway/readme.md`（"name provider 注册"小节）
