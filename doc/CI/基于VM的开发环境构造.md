# 基于虚拟机的分布式开发环境基础设施

本文档介绍位于 `buckyos-devkit` 下的开发环境基础设施。这套工具基于 Multipass 虚拟机，旨在快速构建、部署和测试 BuckyOS 的分布式环境（如 2zone + SN）。

**注意：由于multipass无法固定ip,所以现在暂时不用快照机制（免得ip dance)**

## 1. 核心概念与目录结构

整个基础设施围绕 **Workspace Group**（工作区组）的概念组织。每个 Group 代表一种典型的分布式网络拓扑（例如 `full`），包含一组虚拟机定义和应用配置。


### 典型环境：2zone_sn
这是目前最常用的开发环境，模拟了一个包含 3 个节点的最小化 BuckyOS 网络：
- **SN**：Super Node，提供网络发现服务。
- **Alice.ood1**：模拟 OOD1 设备（配置了端口映射）。
- **Bob.ood1**：模拟 OOD1 设备（LAN 环境）。
*注：宿主机通常作为无 SN 的 WLAN 节点参与网络。*

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
buckyos-devtest 2zone_sn clean_vms

# 2. 创建虚拟机
# 这会根据 nodes.json 创建 VM，并在启动后执行初始化脚本（如设置 iptables、安装 CA 证书）
buckyos-devtest 2zone_sn create_vms

# 3. 创建纯净快照 'init'
buckyos-devtest 2zone_sn snapshot init
```

### 阶段二：软件部署 (Install)
将当前代码库中的 BuckyOS 组件构建并部署到虚拟机。

```bash
# 0. 本地Build,Install
buckyos-build 
buckyos-install

# 1. 编译并安装所有配置的 App
# 脚本会自动执行 build -> push -> install 流程
buckyos-devtest 2zone_sn install

# 2. 创建已安装快照 'installed'
buckyos-devtest 2zone_sn snapshot installed
```

### 阶段三：运行与测试 (Runtime)
启动服务并运行测试用例。

```bash
# 1. 启动 (为了方便观察，也可以登录vm的ssh启动)
buckyos-devtest 2zone_sn start app=$appname

# 2. 创建运行态快照 'started'（可选，用于快速恢复服务运行状态）
buckyos-devtest snapshot started

# 3. 执行测试用例
# 在指定节点（如 alice）上运行测试脚本
buckyos-devtest  2zone_sn run alice "python3 /opt/testcases/test_demo.py"
```

### 阶段四：快速迭代循环
在开发过程中，通常不需要从头构建环境，而是利用快照快速回滚。

**场景 A：修改了代码，需要更新软件**
```bash
buckyos-build
buckyos-update
# 增量更新（执行 update 流程，通常比完整 install 快）
buckyos-devtest 2zone_sn update

# 或者回滚到 init 状态全新安装（更干净）
buckyos-devtest restore init
buckyos-devtest install
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
        "echo '{{sn.ip}} sn.devtests.org' >> /etc/hosts"
      ],
      "apps": {                       // 该节点安装的应用及参数
        "buckyos": {
          "node_group": "alice.ood1"  // 传递给 app 的自定义参数，通常与 make_config.py 联动
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

#### 5.2.1 构造 `build_all` 与 `make_config.py`
BuckyOS 的构建流程依赖 `src/make_config.py` 脚本来生成特定于节点的配置文件。

**核心逻辑：**
1. **编译 (Compile)**：构建所有二进制可执行文件。
2. **布局 (Layout)**：将二进制文件和基础资源复制到 `source` 目录。
3. **配置 (Config)**：调用 `make_config.py`，根据传入的 `group_name`（如 `alice.ood1`）在 `source` 目录中生成专属配置文件（身份文件、证书、网络配置等）。

这解释了为什么 `full.json` 中需要配置 `node_group` 参数：它被传递给 `build_all` 脚本，进而传给 `make_config.py` 来决定生成哪台机器的配置。


#### 完整应用配置示例
```json
{
  "source": "src/apps/my_service/dist",     // 本地构建产物目录 (包含二进制 + make_config 生成的配置)
  "target": "/opt/buckyos/my_service",      // VM 上的安装目录
  
  "source_bin": "src/target/release/my_service", // 仅包含二进制文件 (用于快速 update)
  "target_bin": "/opt/buckyos/my_service/bin",   // VM 上的二进制目录
  
  "commands": {
    // Install 流程：编译 -> 组装文件 -> 生成专属配置
    "build_all": "cd src/apps/my_service && make build && python3 ./make_config.py --group {{buckyos.node_group}} --rootfs dist",
    
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
