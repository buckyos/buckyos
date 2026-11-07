## 为什么会有这个 Bug

容器访问宿主机 `cyfs-gateway` 依赖 `BUCKYOS_HOST_GATEWAY`，默认值是 `host.docker.internal`。现有实现有两个缺口：

1. `node_daemon` 只有部分容器启动路径加了 `--add-host=host.docker.internal:host-gateway`，普通 Docker AppService 没加，Agent 的预览命令里有但真实 `docker run` 里漏了。
2. `buckyos-api` 在容器内直接把 `host.docker.internal` 拼进 URL，遇到 Docker Desktop 返回 IPv6 优先的解析结果时，访问宿主机 `cyfs-gateway` 可能失败。

所以问题不是单个平台不可用，而是“容器别名注入不完整 + 解析结果未优先收敛到 IPv4”叠加导致的跨平台不稳定。

## 我是如何修复的

1. 在 `src/kernel/node_daemon/src/app_loader.rs` 里统一抽出 `append_host_gateway_run_args()`，让 Docker AppService、HostScript、Agent 三类 `docker run` 都注入 `--add-host host.docker.internal:host-gateway`，并让 preview 与真实执行保持一致。
2. 在 `src/kernel/buckyos-api/src/runtime.rs` 里新增容器网关主机解析逻辑：对 `BUCKYOS_HOST_GATEWAY` 优先解析 IPv4 字面量，成功就直接用 IPv4 访问 `cyfs-gateway`，失败才回退到原始主机名。

这样 Linux/macOS/Windows 上的容器都能统一拿到宿主机别名，而 BuckyOS runtime 访问本地 gateway 时会优先落到稳定的 IPv4 路径。
