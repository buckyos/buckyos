# rbac (Role Based Access Control)
本模块基于标准的RBAC模型提供了基础的权限控制组件，其核心是
```rust
pub enforce(app_id,user_id,res_path,action_name) -> bool;
```
用来判断当前用户是否有权限对资源进行某个动作。

## 用户身份的确认

usename->user-did->user-did-document->user-public_key

不管身份是一个user,还是一个device,必然都对应一个秘钥对。

## sudo模式

任何秘钥都必须妥善管理。用户的秘钥永远不会被保存在系统里。设备在指定的本地路径保存自己的秘钥，并且不会被任何网络行为获取。
当使用基于秘钥构造的token时，该用户进入sudo模式，可以获得其最高权限。平时用户使用session_token，为常规权限。
service本身没有sudo模式。

## buckyos的基本用户组（权限由高到底）

owner(zone_owner,系统管理员)：拥有一切权限，不建议日常使用。是特定用户的sudo模式。对小型系统通常只有一个系统管理员，该账号能进行大量危险操作应避免日常使用。
kernel_service(system_service)：运行在内核态的服务，一般是node_daqemo相关，这些服务可以访问保存在device本地路径的秘钥、
frame_service:框架服务，本身拥有较高的权限，frame_service互相之间的维护性操作是可以互相许可的
sudo user:常规的管理员权限，该权限可以管理用户的所有数据，但不能修改系统数据
user:普通用户，能访问自己的所有数据，但无法进行敏感操作
app_service:应用通常工作在这个模式，只能访问appid权限允许和user权限允许的交集的数据。如果用户以sudo模式运行app,那么可以访问到其sudo模式可以访问的的数据，但依旧无法超出给app的授权范围。user_service一般工作在容器中。
limit_user:受限用户，没有sudo能力，只能使用指定应用，没有有sudo能力用户的安装应用的能力
guest(匿名用户):只能访问公开的数据

## 根据上述逻辑的默认配置文件

我们目前有3类资源，其URL如下：
1. 保存在SystemConfig里的配置文件，其路径为 kv://xxxx
2. 保持在dfs上的文件，其路径为 dfs://xxxx
3. 特定device上的额文件，其路径为 fs://$device_id:/xxxx

根据上面定义，其权限配置如下：
``` model.conf
[request_definition]
r = sub, obj, act

[policy_definition]
p = sub, obj, act, domain

[role_definition]
g = _, _, _  # sub, role, domain

[policy_effect]
e = some(where (p.eft == allow))

[matchers]
m = g(r.sub, p.sub, r.dom) && keyMatch2(r.obj, p.obj) && (r.act == p.act || keyMatch(r.act, p.act))


```

```policy.csv
# 定义常见的权限集
# ReadWrite 权限包括 read 和 write
p, owner, kv://*, ReadWrite, zone_id
p, owner, dfs://*, ReadWrite, zone_id
p, owner, fs://$device_id:/, ReadWrite, zone_id

p, kernel, kv://*, ReadWrite, zone_id
p, kernel, dfs://*, ReadWrite, zone_id
p, kernel, fs://$device_id:/, ReadWrite, zone_id

p, frame, kv://*, ReadWrite, zone_id
p, frame, dfs://*, ReadWrite, zone_id
p, frame, fs://$device_id:/, ReadWrite, zone_id

p, sudo_user, kv://*, ReadWrite, zone_id
p, sudo_user, dfs://*, ReadWrite, zone_id
p, sudo_user, fs://$device_id:/, ReadWrite, zone_id

p, user, dfs://homes/$userid, ReadWrite, zone_id

p, app_service, dfs://homes/$userid, ReadWrite, zone_id

p, limit_user, dfs://homes/$userid, ReadOnly, zone_id

p, guest, dfs://public, ReadOnly, zone_id

# 定义操作集
g, alice, owner, zone_id
g, bob, sudo_user, zone_id
g, charlie, user, zone_id
g, app, app_service, zone_id



```

