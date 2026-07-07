# 基于虚拟机的分布式开发环境基础设施

本文档介绍位于 `buckyos-devkit` 下的开发环境基础设施。这套工具基于 Multipass 虚拟机，旨在快速构建、部署和测试 BuckyOS 的分布式环境（如 2zone + SN）。

**注意：由于multipass无法固定ip,所以现在暂时不用快照机制（免得ip dance)**

## 1. 核心概念与目录结构

整个基础设施围绕 **Workspace Group**（工作区组）的概念组织。每个 Group 代表一种典型的分布式网络拓扑（例如 `full`），包含一组虚拟机定义和应用配置。


### 典型环境：sntest
这是目前最常用的开发环境，模拟了一个包含 SN 和 OOD 的最小化 BuckyOS 网络：
- **SN**：Super Node，提供网络发现服务。
- **Alice.ood1**：模拟 OOD1 设备（LAN 环境）。
*注：宿主机通常作为无 SN 的 WLAN 节点参与网络。*

`sntest` 的 `alice-ood1` 必须把 DNS server 指向 SN。`/etc/hosts` 只能解析 `sn.devtests.org` 这类固定主机名，不能参与 DNS TXT 查询；激活流程里查询 zone 相关 TXT 记录时，需要通过 SN 的 DNS 服务返回结果。

## 2. 前置准备

1. **安装 Multipass**：确保系统已安装 Multipass 且有权限创建/启动虚拟机。
2. **Python 环境**：需要 Python 3,最好安装venv
3. **buckyos-devkit**: 使用 `pip install "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"` 安装
3. **工作目录**：建议在项目src目录下执行命令。

## 3. 标准开发工作流

本套系统利用**虚拟机快照**机制来加速开发循环，避免重复的编译和部署等待。

### 阶段一：环境初始化 (Init)
构建基础虚拟机环境，安装操作系统和基础依赖。

```bash
# 1. 清理旧环境（可选）
uv run buckyos-devtest sntest clean_vms

# 2. 创建虚拟机
# 这会根据 nodes.json 创建 VM，并在启动后执行初始化脚本（如设置 hostname、目录权限、hosts 记录、DNS server）
uv run buckyos-devtest sntest create_vms

# 3. 创建纯净快照 'init'
uv run buckyos-devtest sntest snapshot init
```

### 阶段二：软件部署 (Install)
将当前代码库中的 BuckyOS 组件构建并部署到虚拟机。

```bash
# 0. 本地 Build Linux 版本，并准备 VM 专用 BuckyOS rootfs。
# build_for_vm_test.py 会按当前 CPU 架构选择 Linux target，
# 通过 buckyos-devkit 依次构建同级 cyfs-gateway 与 BuckyOS，
# 并把 BuckyOS rootfs 与 cyfs-gateway 组件安装到本机基础 staging 目录 /opt/buckyosvm/base。
# web3-gateway 是 SN 独立 app，仍由 devtest 部署到 /opt/web3-gateway。
uv run ./build_for_vm_test.py

# 1. 编译并安装所有配置的 App。
# 脚本会自动执行 host build_all -> push -> remote install 流程。
# buckyos.build_all 会从 /opt/buckyosvm/base 复制出 /opt/buckyosvm/current，
# 再调用 make_config.ts 为当前节点生成专属配置；
# devtest 随后会把 current push 到 VM 默认 BUCKYOS_ROOT=/opt/buckyos。
# web3-gateway.build_all 会调用同级 cyfs-gateway 仓库的 src/make_sn_config.ts 生成
# SN 配置，并按需生成 alice/bob/charlie 的开发用户环境供 SN DB 注册使用。
# 目前带 --legacy 过渡（devtest VM 尚无 EVM 链）；VM 环境构造纳入 DV 链后去掉 --legacy。
uv run buckyos-devtest sntest install

# 2. 创建已安装快照 'installed'
uv run buckyos-devtest sntest snapshot installed
```

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

如果 `sn_server` 阶段仍报 `Failed to read user_config.json`，说明 `web3-gateway.build_all` 中的用户环境 bootstrap 未执行成功。可先单独重试：

```bash
uv run buckyos-devtest sntest exec web3-gateway.build_all --device sn
```

`web3-gateway` 的文件会直接 push 到 VM 的 `/opt/web3-gateway`。如果 push 阶段出现 `cannot write to remote file` / `No space left on device`，优先检查 SN VM 根分区空间。`sntest` 配置已将新建 SN VM 磁盘设为 8G；对已经创建的旧 VM，需清理空间或重建 VM 后重试 install：

```bash
uv run buckyos-devtest sntest clean_vms
uv run buckyos-devtest sntest create_vms
uv run buckyos-devtest sntest install
```

如果 push 阶段报 `cannot create remote directory /opt/web3-gateway: Permission denied`，说明当前 SN VM 没有执行到最新的目录初始化权限配置。可以重建 VM，或先对现有 VM 补一次目录授权后重试：

```bash
multipass exec sn -- bash -c "sudo mkdir -p /opt/web3-gateway && sudo chown -R ubuntu:ubuntu /opt/web3-gateway"
uv run buckyos-devtest sntest install
```

#### 自签发 CA 证书

开发环境默认使用 `src/make_config.ts` 生成的自签发 CA 来签发测试 TLS 证书。默认 CA 目录在本机的 `~/buckycli/ca/`，常见文件名为 `buckyos_test_ca_ca_cert.pem` 和 `buckyos_test_ca_ca_key.pem`。每个 BuckyOS 节点生成配置后，节点 rootfs 内会包含：

- `etc/zone_cert.cert`：该 Zone 的服务端证书。
- `etc/zone_cert_key.pem`：该 Zone 的服务端私钥。
- `etc/ca.cert`：需要被客户端信任的 CA 证书。

在 `sntest` 环境里，`uv run buckyos-devtest sntest install` 会把本机 `/opt/buckyosvm/current` 下的节点 rootfs 推送到 VM 的默认 `/opt/buckyos`，并通过 `src/dev_configs/apps/buckyos.json` 的 install 命令在 VM 内执行：

```bash
sudo cp /opt/buckyos/etc/ca.cert /usr/local/share/ca-certificates/buckyos_ca.crt
sudo chmod 644 /usr/local/share/ca-certificates/buckyos_ca.crt
sudo update-ca-certificates
```

这只解决 VM 内系统进程、curl、Python requests 等访问测试域名时的信任问题。宿主机浏览器或宿主机命令行要直接访问 `https://sn.devtests.org`、`https://alice.web3.devtests.org` 等测试域名时，也需要把同一个 CA 证书导入宿主机信任库。

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
uv run buckyos-devtest sntest run alice-ood1 "openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt /opt/buckyos/etc/zone_cert.cert"
uv run buckyos-devtest sntest run alice-ood1 "curl -v https://sn.devtests.org/kapi/sn"
```

如果 `web3_gateway` 日志出现 `AlertReceived(UnknownCA)`，先在发起访问的客户端机器上执行严格验证。若 `curl` 或 `openssl verify` 报 `invalid CA certificate`，说明本机 `~/buckycli/ca/` 下缓存的是旧格式测试 CA，缺少 `CA:TRUE` 约束。删除旧 CA 后重新运行 `uv run ./build_for_vm_test.py` 和 `uv run buckyos-devtest sntest install`，再按上面的步骤重新导入 CA。

#### SN DNS 配置检查

`sntest` 中 SN 会监听 53 端口，`alice-ood1` 会在 `create_vms` 的 `instance_commands` 阶段写入：

```text
/etc/systemd/resolved.conf.d/buckyos-dns.conf
```

内容应包含：

```ini
[Resolve]
DNS=<sn.ip>
Domains=~devtests.org
FallbackDNS=8.8.8.8 1.1.1.1
```

如果 DNS TXT lookup 失败，先确认 Alice 当前 DNS server 是否已经指向 SN：

```bash
uv run buckyos-devtest sntest run alice-ood1 "resolvectl dns"
uv run buckyos-devtest sntest run alice-ood1 "resolvectl domain"
uv run buckyos-devtest sntest run alice-ood1 "cat /etc/systemd/resolved.conf.d/buckyos-dns.conf"
```

生效后，`resolvectl domain` 应能看到 `~devtests.org`。这表示 `*.devtests.org` 会通过 SN DNS 查询，而不是走 VM 从 DHCP 获得的默认 DNS。如果缺少这段配置，重新执行 `uv run buckyos-devtest sntest clean_vms && uv run buckyos-devtest sntest create_vms`，或在 VM 内补写配置后执行 `sudo systemctl restart systemd-resolved`。只添加 `/etc/hosts` 不足以修复 TXT lookup。

### 阶段三：运行与测试 (Runtime)
启动服务并运行测试用例。

```bash
# 1. 启动 (为了方便观察，也可以登录vm的ssh启动)
uv run buckyos-devtest sntest start app=$appname

# 2. 创建运行态快照 'started'（可选，用于快速恢复服务运行状态）
uv run buckyos-devtest sntest snapshot started

# 3. 执行测试用例
# 在指定节点（如 alice）上运行测试脚本
uv run buckyos-devtest sntest run alice-ood1 "python3 /opt/testcases/test_demo.py"
```

### 阶段四：快速迭代循环
在开发过程中，通常不需要从头构建环境，而是利用快照快速回滚。

**场景 A：修改了代码，需要更新软件**
```bash
# 重新构建 Linux 版本，并刷新 /opt/buckyosvm。
uv run ./build_for_vm_test.py

# 增量更新（执行 update 流程，通常比完整 install 快）
uv run buckyos-devtest sntest update

# 或者回滚到 init 状态全新安装（更干净）
uv run buckyos-devtest sntest restore init
uv run buckyos-devtest sntest install
```

## 4. 命令参考手册

通用语法：`buckyos-devtest  <group_name> <command> [args]`

### 虚拟机管理
- **`create_vms`**：创建所有虚拟机。
- **`clean_vms`**：销毁所有虚拟机。
- **`start_vms`**：启动所有虚拟机（仅启动 VM，不启动业务进程）。
- **`stop_vms`**：停止所有虚拟机。
- **`info_vms`**：显示虚拟机状态列表（包含 IP 地址）。

### 快照管理
- **`snapshot <name>`**：对组内所有 VM 创建同名快照。
- **`restore <name>`**：将组内所有 VM 恢复到指定快照。

### 应用部署与管理
- **`install [device_id] [--apps ...]`**：
    - 完整安装。如果不指定 `device_id`，则安装所有设备。
    - 流程：Host Build -> Push Files -> Remote Install。
- **`update [device_id] [--apps ...]`**：
    - 增量更新。如果不指定 `device_id`，则更新所有设备。
    - 流程：Host Build -> Push Binaries -> Remote Update。
- **`start`**：执行 App 的启动命令（启动 BuckyOS）。
- **`stop`**：执行 App 的停止命令。

### 调试与运维
- **`run <node_id> <cmd1> [cmd2 ...]`**：
    - 在指定节点执行 Shell 命令。支持多条命令顺序执行。
    - 执行前会加载 workspace 环境参数并解析命令中的模板变量。正式开发流程必须继续通过 `buckyos-devtest <group> run ...` 执行节点命令，不能让使用者直接调用 `multipass exec`，以保持 VM 后端透明。
- **`clog`**：
    - 收集日志。将所有节点的日志目录（在配置中定义）拉取到本地临时目录（默认 `/tmp/clogs`）。

## 5. 高级配置说明

### 5.1 {group_name}.json 配置
该文件定义了环境中的虚拟机节点及其属性。

```json
{
  "nodes": {
    "sn": {
      "node_id": "sn",                // 内部引用的 ID
      "vm_template": "ubuntu_basic",  // 引用 templates/ 下的 YAML 文件名
      "init_commands": [              // VM 创建后立即执行的初始化命令（root 权限）
        "sudo hostnamectl set-hostname sn"
      ],
      "directories": {                // 定义特殊目录，如日志收集目录
        "logs": "/opt/buckyos/logs"
      }
    },
    "alice-ood1": {
      "instance_commands": [          // 所有 VM 创建完成后，按顺序执行的命令
        // 支持使用变量 {{node_id.attribute}} 引用其他节点信息
        "sudo sh -c \"echo '{{sn.ip}} sn.devtests.org' >> /etc/hosts\"",
        "sudo mkdir -p /etc/systemd/resolved.conf.d",
        "sudo sh -c \"echo '[Resolve]\\nDNS={{sn.ip}}\\nDomains=~devtests.org\\nFallbackDNS=8.8.8.8 1.1.1.1' > /etc/systemd/resolved.conf.d/buckyos-dns.conf\"",
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
   - `{{system.base_dir}}`: 仓库根目录的绝对路径。
   - 以及宿主机的其他环境变量。

2. **节点变量** (格式 `{{node_id.attribute}}`)：
   - `{{sn.ip}}`: 引用名为 `sn` 的节点的 IP 地址。
   - `{{alice-ood1.ip}}`: 引用 `alice-ood1` 节点的 IP。

3. **应用参数** (仅在执行 App 命令时可用)：
   - `{{app_name.param_key}}`: 引用 `nodes.json` 中 `apps` 字段下定义的参数。
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
   - **传输机制**：`push` 操作默认使用 `sftp/scp` 协议，通常以 `ubuntu` 用户身份执行。
   - **目标目录权限**：如果 `target` 目录（如 `/opt/buckyos`）由 `root` 创建，`ubuntu` 用户可能没有写入权限，导致 push 失败。
   - **解决方案**：在 `nodes.json` 的 `init_commands` 中预先放开权限。
     ```json
     "init_commands": [
       "sudo mkdir -p /opt/buckyos",
       "sudo chown -R ubuntu:ubuntu /opt/buckyos" // 推荐：将所有权交给部署用户
     ]
     ```

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
