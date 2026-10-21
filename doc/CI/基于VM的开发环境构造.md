# 基于虚拟机的分布式开发环境基础设施

本文档介绍位于 `buckyos-devkit` 下的开发环境基础设施。这套工具基于 Multipass 虚拟机，旨在快速构建、部署和测试 BuckyOS 的分布式环境（如 2zone + SN）。

**关于 IP 与快照**：Multipass 的 `stop`/`start`、`snapshot`/`restore` 不会改变 VM 的 IP。只有 `clean_vms` + `create_vms` 重建 VM 时 IP 才会变。SN 的配置和 alice 的 DNS 设置都写入了当时的 SN IP，所以重建 VM 后必须重新 `install`，不能沿用重建前的快照。

## 1. 核心概念与目录结构

整个基础设施围绕 **Workspace Group**（工作区组）的概念组织。每个 Group 代表一种典型的分布式网络拓扑（例如 `full`），包含一组虚拟机定义和应用配置。Group 定义在 `src/dev_configs/<group>.json`，应用定义在 `src/dev_configs/apps/*.json`，VM 模板在 `src/dev_configs/templates/*.yaml`。`sntest` 的 `web3-gateway` 应用定义引用同级 cyfs-gateway 仓库的 `src/dev_configs/apps/web3-gateway.json`。

### 典型环境：sntest
这是目前最常用的开发环境，模拟了一个包含 SN 和 OOD 的最小化 BuckyOS 网络：
- **SN**（VM `sn`）：Super Node，运行 `web3-gateway`（内含 `cyfs-sn`）、`bns_dv` 和 Anvil 链，提供 DNS、DID 解析、BNS 代理和 RTCP 中转。
- **Alice.ood1**（VM `alice-ood1`）：模拟 NAT 后面的 OOD1。入站 TCP 80/443/2980 被拒绝，规则由 `sntest-nat-firewall.service` 在每次开机时设置。对外访问依赖它到 SN 的 RTCP keep-tunnel，由 SN 中转。
*注：宿主机通常作为无 SN 的 WLAN 节点参与网络。*

`sntest` 的 `alice-ood1` 必须把 DNS server 指向 SN。`/etc/hosts` 只能解析 `sn.devtests.org` 这类固定主机名，不能参与 DNS TXT 查询；激活流程里查询 zone 相关 TXT 记录时，需要通过 SN 的 DNS 服务返回结果。

alice 的 rootfs 由 `make_config.ts` 预生成身份，其中包含 `etc/alice.bns.did.zone.json`。node_daemon 发现该文件后会使用本地 Zone 文档，跳过网络发现，日志中会出现 `debug zone document override ... skip network discovery`。因此 sntest 不覆盖 OOD 经 DNS/BNS 发现自身 Zone 的启动路径；它覆盖的是 OOD 向 SN 注册、keep-tunnel、SN 中转和 SN 侧解析。

## 2. 前置准备

1. **安装 Multipass**：确保系统已安装 Multipass 且有权限创建/启动虚拟机。Linux 上的 Multipass 是 snap 包，只能直接读写宿主机 `$HOME` 下的文件；devkit 的 Multipass 后端通过 `multipass transfer` 的 stdin/stdout 传输 tar 包，所以 staging 目录放在 `/opt`、`/tmp` 下也能推送。旧版 devkit 直接把宿主机路径交给 `multipass transfer`，在 Linux 上会报 `cannot access ...: No such file or directory`。
2. **Python 环境**：需要 Python 3.12+ 和 [uv](https://docs.astral.sh/uv/)。在仓库内用 `uv run` 执行命令，`buckyos-devkit` 由根目录 `pyproject.toml` 引入（GitHub `main` 分支），具体提交锁定在 `uv.lock`；需要更新 devkit 时执行 `uv lock --upgrade-package buckyos-devkit`。只有在仓库外使用时才需要 `pip install "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main"`。
3. **构建工具链**：Rust、Node.js/pnpm、Deno 等，与本机构建 BuckyOS 相同；同级目录需要有 `cyfs-gateway` 仓库。
4. **磁盘空间**：release 构建缓存在 `/tmp/rust_build/buckyos` 和 `/tmp/rust_build/cyfs-gateway`（合计约 30G），两台 VM 约 10G。
5. **工作目录**：在 `buckyos/src` 下执行命令（devtest 也会查找仓库根目录下的 `src/dev_configs`）。

Multipass 当前默认镜像是 Ubuntu 26.04 LTS。该镜像的 `/tmp` 是 tmpfs，大小约为内存的一半（1G 内存的 VM 只有约 476M），不要在 VM 的 `/tmp` 里放大文件。

`ubuntu_sn` 模板会从 Node.js 官方发布目录安装并校验当前 Node.js 24 LTS，同时提供 npm。BNS 的 Hardhat 部署要求 Node.js 22.13 或更高版本。已有 SN VM 不会自动重新执行 cloud-init；模板更新后需要重建 VM，或先为现有 VM 补齐相同版本的 Node.js。

## 3. 标准开发工作流

本套系统利用**虚拟机快照**机制来加速开发循环，避免重复的编译和部署等待。

### 阶段一：环境初始化 (Init)
构建基础虚拟机环境，安装操作系统和基础依赖。

```bash
# 1. 清理旧环境（可选）
uv run buckyos-devtest sntest clean_vms

# 2. 创建虚拟机
# 这会根据 sntest.json 创建 VM，并在启动后执行 init_commands 和 instance_commands
# （hostname、目录权限、alice 的 NAT 防火墙、hosts 记录、DNS server、容器 DNS）
uv run buckyos-devtest sntest create_vms

# 3. 创建纯净快照 'init'
uv run buckyos-devtest sntest snapshot init
```

`create_vms` 不检查 init/instance 命令的退出码，创建后可以用下面的命令确认关键设置已经生效：

```bash
uv run buckyos-devtest sntest run alice-ood1 "sudo iptables -S INPUT" "resolvectl domain" "cat /etc/docker/daemon.json"
```

### 阶段二：软件部署 (Install)
将当前代码库中的 BuckyOS 组件构建并部署到虚拟机。

```bash
# 0. 本地 Build Linux 版本，并刷新 VM 专用 BuckyOS 基础 rootfs。
# build_for_vm_test.py 会按当前 CPU 架构选择 Linux target，
# 通过 buckyos-devkit 依次构建同级 cyfs-gateway 与 BuckyOS，
# 并把 BuckyOS rootfs 与 cyfs-gateway 组件安装到本机基础 staging 目录 /opt/buckyosvm/base。
# web3-gateway（SN）不在这一步处理，见下一步。
uv run ./build_for_vm_test.py

# 1. 编译并安装所有配置的 App（host build_all -> push -> remote install）。
# buckyos.build_all 会从 /opt/buckyosvm/base 复制出 /opt/buckyosvm/current，
# 再调用 make_config.ts 为当前节点生成专属配置；
# devtest 随后会把 current push 到 VM 默认 BUCKYOS_ROOT=/opt/buckyos。
# buckyos 的 install 命令还会在 VM 里 docker pull paios/aios 与 paios/exttool 的 latest-<arch> 镜像。
# web3-gateway.build_all 会调用同级 cyfs-gateway 仓库的 dev_configs/web3_gateway_staging.py：
# 在 /tmp/buckyos-vmtest/cyfs-gateway/web3-gateway 从零重建 staging（按宿主机架构构建，
# 只把 web3-gateway 装进 staging），复制 BNS Hardhat 工程，运行 make_sn_config.ts --seed-v2
# 生成 SN 配置，并按需生成 alice/bob/charlie/dave 的开发用户环境（~/buckycli）供 SN DB 与 BNS 种子使用；
# devtest 再把 staging push 到 SN VM 的 /opt/web3-gateway。
uv run buckyos-devtest sntest install

# 2. 创建已安装快照 'installed'
uv run buckyos-devtest sntest snapshot installed
```

开发安装没有 paios 镜像清单（`bin/paios-images/paios_images.json`），node_daemon 只使用本地已有的 `paios/aios:latest-<arch>`、`paios/exttool:latest-<arch>`，从不拉取。缺 `paios/aios` 时，`buckyos-systest` 等容器应用会反复报 `is not in the local container store and BuckyOS never pulls it`；缺 `paios/exttool` 时应用会在没有 ExtTool 挂载的情况下启动。所以 install 时预先拉取这两个镜像，这一步需要 VM 能访问 Docker Hub。

`build_for_vm_test.py` 默认使用 `/opt/buckyosvm` 作为 VM test staging 根目录，其中：

- `/opt/buckyosvm/base`：基础 rootfs，只由 `uv run ./build_for_vm_test.py` 刷新。
- `/opt/buckyosvm/current`：当前节点 rootfs，由 devtest 的 `buckyos.build_all` 从 base 复制并写入节点配置。

首次使用时如果当前用户没有 `/opt` 写权限，先创建并授权：

```bash
sudo mkdir -p /opt/buckyosvm
sudo chown -R "$USER" /opt/buckyosvm
```

如果需要改 staging 目录，可以使用：

```bash
uv run ./build_for_vm_test.py --rootfs /path/to/buckyosvm
```

同时需要让 `src/dev_configs/apps/buckyos.json` 中的 `source`、`target` 和启动命令保持同一个目录。

`build_for_vm_test.py` 和 web3-gateway 的 staging 构建都只写 staging 目录，不会更新宿主机上正在运行的 `/opt/buckyos`。

如果 SN 侧的 staging 生成失败（例如 `make_sn_config.ts` 报身份交接文件无效），修正后只重装 SN 即可：

```bash
uv run buckyos-devtest sntest install sn --apps web3-gateway
```

不要用 `exec web3-gateway.build_all --device sn` 重试：`exec` 总是在 VM 内执行命令，而 `build_all` 需要在宿主机的 cyfs-gateway 源码目录里运行。

`web3-gateway` 的文件会直接 push 到 VM 的 `/opt/web3-gateway`。如果 push 阶段出现 `No space left on device`，优先检查 SN VM 根分区空间。`sntest` 配置已将新建 SN VM 磁盘设为 8G；对已经创建的旧 VM，需清理空间或重建 VM 后重试 install：

```bash
uv run buckyos-devtest sntest clean_vms
uv run buckyos-devtest sntest create_vms
uv run buckyos-devtest sntest install
```

目录推送前，devtest 会在 VM 内 `sudo mkdir -p` 目标目录并把它 `chown` 给 VM 默认用户，不需要再手动授权 `/opt/web3-gateway`、`/opt/buckyos`。

#### 自签发 CA 证书

开发环境默认使用 `src/make_config.ts` 生成的自签发 CA 来签发测试 TLS 证书。默认 CA 目录在本机的 `~/buckycli/ca/`，常见文件名为 `buckyos_test_ca_ca_cert.pem` 和 `buckyos_test_ca_ca_key.pem`。每个 BuckyOS 节点生成配置后，节点 rootfs 内会包含：

- `local/identity/<host>/server.fullchain.pem`（以及 `server.cert.pem`、`server.chain.pem` 等）：按 Zone 主机名签发的服务端证书，alice 有 `alice.bns.did`、`*.alice.bns.did`、`alice.web3.devtests.org`、`*.alice.web3.devtests.org` 四组。
- `etc/ca.cert`：需要被客户端信任的 CA 证书。

在 `sntest` 环境里，`uv run buckyos-devtest sntest install` 会把本机 `/opt/buckyosvm/current` 下的节点 rootfs 推送到 VM 的默认 `/opt/buckyos`，并通过 `src/dev_configs/apps/buckyos.json` 的 install 命令在 VM 内执行：

```bash
sudo cp /opt/buckyos/etc/ca.cert /usr/local/share/ca-certificates/buckyos_ca.crt
sudo chmod 644 /usr/local/share/ca-certificates/buckyos_ca.crt
sudo update-ca-certificates
```

这只解决 VM 内系统进程、curl、Python requests 等访问测试域名时的信任问题。宿主机浏览器或宿主机命令行要直接访问 `https://sn.devtests.org`、`https://alice.web3.devtests.org` 等测试域名时，也需要把同一个 CA 证书导入宿主机信任库，或者在命令里显式传 `--cacert ~/buckycli/ca/buckyos_test_ca_ca_cert.pem`。

macOS 宿主机可执行：

```bash
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain \
  ~/buckycli/ca/buckyos_test_ca_ca_cert.pem
```

Linux 宿主机可执行：

```bash
sudo cp ~/buckycli/ca/buckyos_test_ca_ca_cert.pem /usr/local/share/ca-certificates/buckyos_test_ca.crt
sudo update-ca-certificates
```

如果需要重新生成一套 CA，先删除 `~/buckycli/ca/` 下旧的 `*_ca_cert.pem` 和 `*_ca_key.pem`，再重新运行 install 流程。已经创建过的 VM 快照不会自动继承新 CA，应从 `init` 快照重新 install，或在目标 VM 内重新复制 `/opt/buckyos/etc/ca.cert` 并执行 `sudo update-ca-certificates`。

验证 VM 内 CA 是否已生效：

```bash
uv run buckyos-devtest sntest run alice-ood1 "openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt /opt/buckyos/local/identity/alice.web3.devtests.org/server.fullchain.pem"
uv run buckyos-devtest sntest run alice-ood1 "curl -sS -o /dev/null -w '%{http_code}\n' https://sn.devtests.org/kapi/sn"
```

第二条命令返回 `405` 即说明 TLS 校验通过（该接口只接受 POST）；证书不受信任时 curl 会直接报错。

如果 `web3_gateway` 日志出现 `AlertReceived(UnknownCA)`，先在发起访问的客户端机器上执行严格验证。若 `curl` 或 `openssl verify` 报 `invalid CA certificate`，说明本机 `~/buckycli/ca/` 下缓存的是旧格式测试 CA，缺少 `CA:TRUE` 约束。删除旧 CA 后重新运行 `uv run ./build_for_vm_test.py` 和 `uv run buckyos-devtest sntest install`，再按上面的步骤重新导入 CA。

#### SN DNS 配置检查

`sntest` 中 SN 会监听 53 端口，`alice-ood1` 会在 `create_vms` 的 `instance_commands` 阶段写入：

```text
/etc/systemd/resolved.conf.d/buckyos-dns.conf
/etc/docker/daemon.json
```

`buckyos-dns.conf` 内容应包含：

```ini
[Resolve]
DNS=<sn.ip>
Domains=~devtests.org
FallbackDNS=8.8.8.8 1.1.1.1
DNSStubListenerExtra=172.17.0.1
```

`daemon.json` 内容为 `{"dns": ["172.17.0.1"]}`。

如果 DNS TXT lookup 失败，先确认 Alice 当前 DNS server 是否已经指向 SN：

```bash
uv run buckyos-devtest sntest run alice-ood1 "resolvectl dns"
uv run buckyos-devtest sntest run alice-ood1 "resolvectl domain"
uv run buckyos-devtest sntest run alice-ood1 "cat /etc/systemd/resolved.conf.d/buckyos-dns.conf"
```

生效后，`resolvectl domain` 应能看到 `~devtests.org`。这表示 `*.devtests.org` 会通过 SN DNS 查询，而不是走 VM 从 DHCP 获得的默认 DNS。如果缺少这段配置，重新执行 `uv run buckyos-devtest sntest clean_vms && uv run buckyos-devtest sntest create_vms`，或在 VM 内补写配置后执行 `sudo systemctl restart systemd-resolved`。只添加 `/etc/hosts` 不足以修复 TXT lookup。

容器不经过 systemd-resolved 的分域路由。Docker 默认从 `/run/systemd/resolve/resolv.conf` 生成容器的 DNS 列表，SN 会排在第一位。SN 只对自己的域名权威，对 `jsr.io` 这类外部域名会返回 NXDOMAIN，容器收到 NXDOMAIN 后不会再问下一个服务器，于是容器应用下载依赖时失败（例如 `buckyos-systest` 报 `JSR package manifest for '@std/http' failed to load`）。为此，sntest 让 systemd-resolved 额外在 docker0（`172.17.0.1`）上提供 stub，并把它设为 Docker 的默认 DNS，这样容器与 VM 走同一套分域解析。验证：

```bash
uv run buckyos-devtest sntest run alice-ood1 "sudo docker run --rm --entrypoint sh paios/aios:latest-amd64 -c 'getent hosts jsr.io sn.devtests.org'"
```

### 阶段三：运行与测试 (Runtime)
启动服务并运行测试用例。

```bash
# 1. 启动。不带参数时按节点顺序启动全部应用，也可以指定节点和应用：
#    uv run buckyos-devtest sntest start [device_id] [--apps app1 app2]
# SN 的 start 每次都会执行 init_anvil.py --fresh 重建 Anvil 链并重新部署 BNS 合约，
# 再后台启动 web3-gateway，等 443/53 端口和 /kapi/sn 就绪后返回（最多 90 秒）；
# alice 的 start 在后台启动 node_daemon，服务全部起来大约还需要 1 分钟。
uv run buckyos-devtest sntest start

# 2. 创建运行态快照 'started'（可选，用于快速恢复服务运行状态）
uv run buckyos-devtest sntest snapshot started
```

测试用例（在宿主机执行，SN 的 IP 用 `info_vms` 查看）：

```bash
uv run buckyos-devtest sntest info_vms
SN_IP=<sn 的 ip_v4>

# a. SN VM 冒烟（S1–S8）：DNS A/TXT、BNS 与 user_domain 解析、纯 Web3 用户、
#    SN 代付的 BNS 代理写入，以及 alice 到 SN 的 RTCP keep-tunnel。
#    S7 需要在 SN 内部访问 loopback 的 admin 接口，S8 需要查看 SN 端连接，
#    脚本会自动找到同级 buckyos/src 并通过 buckyos-devtest sntest run sn 执行。
(cd ../../cyfs-gateway/src && ./web3-gateway/scripts/sn-dev-smoke.sh --vm \
  --expected-a "$SN_IP" --dns-server "$SN_IP" --http-origin "http://$SN_IP")

# b. 经 SN 中转访问 NAT 后面的 alice（宿主机直连 alice 的 80/443 会被拒绝）
curl -sS -o /dev/null -w '%{http_code}\n' --cacert ~/buckycli/ca/buckyos_test_ca_ca_cert.pem \
  --resolve alice.web3.devtests.org:443:$SN_IP https://alice.web3.devtests.org/

# c. 容器应用自测：buckyos-systest 的 AppService 全量自测
uv run buckyos-devtest sntest run alice-ood1 \
  'port=$(sudo docker port buckyos-app-buckyos-systest 3000 | head -1 | cut -d: -f2); curl -sS -X POST http://127.0.0.1:$port/sdk/appservice/selftest'
```

`sn-dev-smoke.sh` 应输出 `10 passed, 0 failed`；b 应返回 `200`。c 的结果里，除 `bash → xllm ... (#640)` 外都应为 `ok: true`。这一项需要真实的 LLM 模型，而 VM 里的 Zone 没有配置 AICC provider，会报 `no candidate for llm.chat`。

### 阶段四：快速迭代循环
在开发过程中，通常不需要从头构建环境，而是利用快照快速回滚。

**场景 A：修改了代码，需要更新软件**
```bash
# 重新构建 Linux 版本，并刷新 /opt/buckyosvm。
uv run ./build_for_vm_test.py

# 增量更新（执行 update 流程，通常比完整 install 快）
# buckyos 推送 /opt/buckyosvm/base/bin 到 VM 的 /opt/buckyos/bin；
# web3-gateway 重新构建 staging 里的 web3_gateway 并替换 VM 里的同名文件。
# update 只替换文件、不重启进程（正在运行的旧进程不受影响），之后重启生效：
uv run buckyos-devtest sntest update
uv run buckyos-devtest sntest stop
uv run buckyos-devtest sntest start

# 或者回滚到 init 状态全新安装（更干净）
uv run buckyos-devtest sntest restore init
uv run buckyos-devtest sntest install
```

`update` 不重新生成节点配置，也不刷新 SN 的 seed、BNS 合约和配置文件；改了这些内容时走 `restore init` + `install`。SN 的 `start` 每次都会重建 Anvil 链，重启后链上状态从种子重新开始。

**场景 B：只重跑测试**

```bash
uv run buckyos-devtest sntest restore installed
uv run buckyos-devtest sntest start
# 运行阶段三的测试用例
```

## 4. 命令参考手册

通用语法：`buckyos-devtest <group_name> <command> [args]`

命令可以在 `src/` 目录运行，也可以在仓库根目录运行；devtest 会依次查找 `dev_configs/` 和 `src/dev_configs/`。应用命令返回非零状态时，devtest 会立即停止并返回失败，不再继续启动后续节点。

### 虚拟机管理
- **`create_vms`**：创建所有虚拟机。
- **`clean_vms`**：销毁所有虚拟机（连同快照）。
- **`start_vms`**：启动所有虚拟机（仅启动 VM，不启动业务进程）。
- **`stop_vms`**：停止所有虚拟机。
- **`info_vms`**：显示虚拟机的 IPv4 地址。

### 快照管理
- **`snapshot <name>`**：对组内所有 VM 创建同名快照（会先停止 VM，打完快照再启动）。同名快照已存在时会失败，需先停止 VM 再用 `multipass delete --purge <vm>.<name>` 删除。
- **`restore <name>`**：将组内所有 VM 恢复到指定快照并重新启动。

### 应用部署与管理
- **`install [device_id] [--apps ...]`**：
    - 完整安装。如果不指定 `device_id`，则安装所有设备。
    - 流程：Host Build -> Push Files -> Remote Install。
- **`update [device_id] [--apps ...]`**：
    - 增量更新。如果不指定 `device_id`，则更新所有设备。
    - 流程：Host Build -> Push Binaries -> Remote Update。
- **`start [device_id] [--apps ...]`**：执行 App 的启动命令。
- **`stop [device_id] [--apps ...]`**：执行 App 的停止命令。
- **`uninstall [device_id] [--apps ...]`**：执行 App 的卸载命令。
- **`exec <app>.<cmd> [--device <device_id>]`**：在 VM 内执行 App 配置里的任意命令（如 `web3-gateway.init_anvil`）；不指定设备时对所有设备执行。

### 调试与运维
- **`run <node_id> <cmd1> [cmd2 ...]`**：
    - 在指定节点执行 Shell 命令。支持多条命令顺序执行。
    - 执行前会加载 workspace 环境参数并解析命令中的模板变量。正式开发流程必须继续通过 `buckyos-devtest <group> run ...` 执行节点命令，不能让使用者直接调用 `multipass exec`，以保持 VM 后端透明。
- **`clog`**：
    - 收集日志。将所有节点的日志目录（在配置中定义）拉取到本地 `/tmp/clogs/<node_id>/`。SN 的 `start.log`、`anvil.log` 在 `/opt/web3-gateway` 下，不在收集范围内。

## 5. 高级配置说明

### 5.1 {group_name}.json 配置
该文件定义了环境中的虚拟机节点及其属性。

```json
{
  "nodes": {
    "sn": {
      "node_id": "sn",                // 内部引用的 ID
      "vm_template": "ubuntu_sn",     // 引用 templates/ 下的 YAML 文件名
      "init_commands": [              // VM 创建后立即执行的初始化命令（以 VM 默认用户执行，需要权限时加 sudo）
        "sudo hostnamectl set-hostname sn"
      ],
      "directories": {                // 定义特殊目录，如日志收集目录
        "logs": "/opt/buckyos/logs"
      }
    },
    "alice-ood1": {
      "init_commands": [
        // 直接执行的 iptables 规则在 VM 重启（包括 snapshot/restore）后会丢失，
        // 需要持久的防火墙规则时写成开机执行的 systemd oneshot 单元
        "sudo tee /etc/systemd/system/sntest-nat-firewall.service >/dev/null <<'EOF'\n...\nEOF",
        "sudo systemctl daemon-reload && sudo systemctl enable --now sntest-nat-firewall.service"
      ],
      "instance_commands": [          // 所有 VM 创建完成后，按顺序执行的命令
        // 支持使用变量 {{node_id.attribute}} 引用其他节点信息
        "sudo mkdir -p /etc/systemd/resolved.conf.d",
        "sudo sh -c \"echo '[Resolve]\\nDNS={{sn.ip}}\\nDomains=~devtests.org\\nFallbackDNS=8.8.8.8 1.1.1.1\\nDNSStubListenerExtra=172.17.0.1' > /etc/systemd/resolved.conf.d/buckyos-dns.conf\"",
        "sudo systemctl restart systemd-resolved"
      ],
      "apps": {                       // 该节点安装的应用及参数
        "buckyos": {
          "node_group": "alice.ood1"  // 传递给 app 的自定义参数，通常与 make_config.ts 联动
        }
      }
    }
  },
  "instance_order": ["sn", "alice-ood1"] // instance_commands 的执行顺序
}
```

#### 变量插值机制 (Variable Substitution)
在 `instance_commands` 和应用命令中，支持使用 `{{object.attribute}}` 语法引用动态变量。系统会在执行命令前解析并替换这些变量。

**支持的对象与属性：**
1. **系统变量**：
   - `{{system.base_dir}}`: workspace 的基准目录，即 `dev_configs` 的上一级（sntest 中是 `buckyos/src`）。
   - `{{system.<ENV>}}`: 宿主机环境变量，如 `{{system.HOME}}`。

2. **节点变量** (格式 `{{node_id.attribute}}`)：
   - `{{sn.ip}}`: 引用名为 `sn` 的节点的 IP 地址。
   - 变量名只匹配字母、数字和下划线（`\{\{(\w+)\.(\w+)\}\}`），`{{alice-ood1.ip}}` 这类带连字符的节点名不会被替换，需要被引用的节点应使用不含连字符的 node id。

3. **应用参数** (仅在执行 App 命令时可用)：
   - `{{app_name.param_key}}`: 引用 `{group_name}.json` 中 `apps` 字段下定义的参数。
   - 例如在上面的配置中，`{{buckyos.node_group}}` 会被替换为 `alice.ood1`。

**使用场景示例：**
- **配置 Hosts**：在 Alice 节点上，需要知道 SN 节点的动态 IP 地址。
  `"echo '{{sn.ip}} sn.devtests.org' >> /etc/hosts"`
- **应用启动参数**：启动 BuckyOS 时，需要指定当前节点的组名。
  `"./start_buckyos.sh --group {{buckyos.node_group}}"`

### 5.2 App 开发指南 (apps/*.json)
应用开发者通过 JSON 文件定义应用的构建和部署逻辑。为了确保部署成功，**必须正确处理文件传输和权限问题**。

#### 关键字段与最佳实践
1. **Source 与 Target**：
   - `source` / `target`：用于 `install` 阶段，通常包含完整的依赖、配置和可执行文件。
   - `source_bin` / `target_bin`：用于 `update` 阶段，通常仅包含变化的可执行文件，以提高更新速度。

2. **权限管理 (Critical)**：
   - **传输机制**：Multipass 后端把本地目录打成 tar 包，经 `multipass transfer` 的 stdin 写到 VM 的 `/var/tmp`，再在 VM 内以默认用户（`ubuntu`）解包；单个文件同样经 stdin 传输。
   - **目标目录权限**：推送目录前，devtest 会 `sudo mkdir -p` 目标目录并 `chown` 给 `ubuntu`。单文件推送（如 `target_bin` 是文件）不会做这一步，目标所在目录必须对 `ubuntu` 可写。
   - **建议**：仍在 `init_commands` 中预先创建目标目录并放开权限，便于 install 命令和后续手工操作。
     ```json
     "init_commands": [
       "sudo mkdir -p /opt/buckyos",
       "sudo chown -R ubuntu:ubuntu /opt/buckyos" // 推荐：将所有权交给部署用户
     ]
     ```

3. **命令执行位置**：`build` / `build_all` 在宿主机执行；`install`、`update`、`start`、`stop` 以及通过 `exec` 调用的命令都在 VM 内执行。

#### 5.2.1 构造 `build_all` 与 `make_config.ts`
BuckyOS 的构建流程依赖 `src/make_config.ts` 脚本来生成特定于节点的配置文件。

**核心逻辑：**
1. **编译 (Compile)**：构建所有二进制可执行文件。
2. **布局 (Layout)**：将二进制文件和基础资源复制到 `source` 目录。
3. **配置 (Config)**：调用 `make_config.ts`，根据传入的 `group_name`（如 `alice.ood1`）在 `source` 目录中生成专属配置文件（身份文件、证书、网络配置等）。

这解释了为什么 `full.json` 中需要配置 `node_group` 参数：它被传递给 `build_all` 脚本，进而传给 `make_config.ts` 来决定生成哪台机器的配置。


#### 完整应用配置示例
```json
{
  "source": "src/apps/my_service/dist",     // 本地构建产物目录 (包含二进制 + make_config 生成的配置)
  "target": "/opt/buckyos/my_service",      // VM 上的安装目录
  
  "source_bin": "src/target/release/my_service", // 仅包含二进制文件 (用于快速 update)
  "target_bin": "/opt/buckyos/my_service/bin",   // VM 上的二进制目录
  
  "commands": {
    // Install 流程：编译 -> 组装文件 -> 生成专属配置
    "build_all": "cd src/apps/my_service && make build && deno task make_config {{buckyos.node_group}} --rootfs dist",
    
    // Update 流程：仅重新编译二进制
    "build": "cargo build --release --bin my_service",
    
    // VM 端安装
    "install": [
      "chmod +x /opt/buckyos/my_service/bin/my_service",
      "sudo /opt/buckyos/my_service/bin/install_service.sh"
    ],
    
    // VM 端更新：重启服务
    "update": [
      "sudo systemctl restart my_service"
    ],
    
    "start": "sudo systemctl start my_service",
    "stop": "sudo systemctl stop my_service"
  }
}
```
