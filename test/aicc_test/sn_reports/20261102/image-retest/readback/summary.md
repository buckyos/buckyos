# NDN 读回与 Gateway 连接行为

本轮没有模型调用。全部 8 个生成对象通过原生 NDN 经 Gateway 读取成功，PNG 结构检查通过；首次运行的 7 个对象逐一比对 AICC 持久化 size/digest，一致；补全对象两次读取的大小及 SHA-256 一致。

| Task | HTTP | 字节数 | SHA-256 |
|---|---:|---:|---|
| `t-9f30ef9f12dd4d5da1ec757491997cd7` | 200 | 1666041 | `f83eeeebf58ef79ddabe8db357e21a8beff341b5518fa2faeeede25263a4bb17` |
| `t-914dc369c2c64943828f47b0fba33253` | 200 | 1122604 | `be8b765e03ed8603148dcfc72bf6ec09fcb56128e342574aff66964c956b596a` |
| `t-869fe30b69164969b10c3ca9822db84a` | 200 | 590361 | `c1a38a80370b5378e963cc7a16b9b92292730ce3e0938b5d11c1ebb5208bb7fa` |
| `t-fcc72bfa80f3462386299717014b06ae` | 200 | 1165063 | `81768d6956a1e2d84020788a6f9cb78874159b9e40c8ddfbe6d66b79384b3b2e` |
| `t-2f3cb83a02f94101ab24b80195284656` | 200 | 1178290 | `28687c34930d176b4e5eb0d71aa6131dcac32e79ddeb3cf9105de0406354dcdb` |
| `t-4af88487b52e4ade89084e0e0983c20b` | 200 | 518814 | `38e80069e483bce2d5e79a7c297960b827bedb3a51a0d6b02cb2d26101d55afc` |
| `t-c56259fc65574358b691bd6d1b34ac5c` | 200 | 1171512 | `e8e51bf499fce4030d6f9f76a1463184bf3b9190684caec84d871fbea86bf0bc` |
| `t-b806e60d37a34d0d81738e70e14d3ccb` | 200 | 1783598 | `973518a5bc15409c9f5faa2f429202a4cb94d8a70b6153897916d93e0de510ee` |

对同一个已生成对象进行零模型调用的受控验证：

| 客户端 | HTTP | 字节数 |
|---|---:|---:|
| 路由加入前建立的 keep-alive 连接 | 403 | 0 |
| 路由加入后创建的新连接池 | 200 | 1171512 |
| 独立 localhost origin 连接池 | 200 | 1171512 |

Gateway 日志证据：`cyfs_gateway.1453782.log`，04:10:11、04:10:18、04:10:43 的旧 TCP 连接 `127.0.0.1:43344` 对 NDN GET 输出 `process_chain echo: reject`；成功请求有临时 route prefix 的 `starts-with matched`。这属于临时路由对旧连接未生效，发生在进入临时 cyfs-dir 读取之前；不是对象生成失败、用户数据归属失败或 NDN 无法下载。独立 origin 是本地临时配置措施，未改 Gateway/NDN 实现。

所有临时读回路由均已移除。原始诊断 JSON、下载制品保留本地，不提交。
