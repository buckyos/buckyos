# AGENTS

## 常用命令

**注意下面脚本都推荐在src目录下运行**

### 开发命令:

```bash
# 修改代码后重新构建buckyos
uv run buckyos-build.py
# 指定模块构建
un run buckyos-build.py -s <module-name>
# 跳过web UI,只做Rust构建
uv run src/buckyos-build.py --skip-web
# 重新构建cyfs-gateway 并安装
cd ../../cyfs-gateway/src/ && uv run buckyos-build.py && uv run buckyos-install.py
# 使用上一次build的结果（如果有)覆盖安装后,启动buckyos
uv run start.py
# 使用devtest环境，全新安装并启动. 
uv run start.py --all
# 使用生产环境，全新安装并启动（会进入待激活状态)
uv run start.py --reinstall release

# 检查当前buckyos的运行状态
uv run check.py

# 以开发模式启动Jarvis
./debug_jarvis.sh

# 停止bcukyos
uv run stop.py

```

### 单元测试命令
```bash
cargo test
```

### 单点测试(DV Test)命令

**注意下面脚本都在根目录下运行**
```bash
# 首先确保 DV环境已经启动，下面脚本应返回已经激活并正常可用
uv run src/check.py 

# 列出可用的DV Test Case
uv run test/run.py --list

# 执行某个具体的DV Test Case
uv run test/run.py -p <test_name>
```


## 关键目录用途

- **`src/`**：**日常开发目录,包含全部源码**（见上文 Commands）。
- **`src/kernel/`**：BuckyOS内核服务（如 `node_daemon`、`scheduler`、`kmsg`、`task_manager` 等）。
- **`src/frame/`**：BuckyOS系统服务（如 `control_panel`、`opendan`、`repo_service`、`msg_center` 等），与 `kernel/` 分工协作。
- **`src/apps/`**：系统自带app（`sys_test`）。
- **`src/rootfs/`**:、配置开发基本在此目录。buckyos-build后，会合并得到一个包含可执行文件的,待部署的buckyos rootfs.
- **`src/bucky_project.yaml`** buckyos的工程文件，说明了buckyos-install是如何从rootfs中安装到目标目录的 
- **`src/tools/`**：BuckyOS的CLI工具
- **`src/dev_configs/`**: VM 测试环境的配置
- **`src/test/`**: 可以独立运行，不依赖DV环境的测试
- **`src/test/`**：在DV环境(test.buckyos.io)里运行的测试
- **`doc/`**：工程文档目录，描述了系统的设计与实现（实现部分偶尔会落后于代码)
- **`doc/arch`**：
- **`doc/sdk`**：
- **`doc/<module_name>`**
- **`product/`**：产品需求与规划文档，给人看的，基本不与具体实现相关
- **`proposals/`**：重要:正在推进中的开发任务，包含了书面记录的任务Spec和分工
- **`harness/`**：给AI Agent看的文档，保存了各种规则和提示词
- **`harness/SKILLS`**: 给AI Agent看到可用Skills目录，应优先在此目录查找合适的skill

上述目录中如果存在README.md, 应在查询资料时优先阅读了解更具体的子目录用途。

### BuckyOS 运行时目录 $BUCKYOS_ROOT
BuckyOS运行时目录默认在 /opt/buckyos/ 或 %APPDATA%\buckyos (Windows). 可以通过node_daemon进程的实际位置来确认

- **`$BUCKYOS_ROOT/logs`**:日志目录
- **`$BUCKYOS_ROOT/etc`**:配置目录
- **`$BUCKYOS_ROOT/data/home/devtest/.local/share/jarvis`**: Agent Jarvis在DV环境下的的root dir

通过阅读 `doc/path_usage.md` 可以得到更详细的信息

## 处理规则

必须处理：开始实现前，要先对任务进行分类，优先使用这一类任务的处理规则。

### 书面任务

`harness/ruels/proposal.md`

### 解Bug

`harness/rules/fixbug.md`

### 优化现有实现

`harness/rules/improve.md`

### 偿还技术债务

`harness/rules/refactor.md`

### 其它口头开发任务

`harness/rules/others.md`

### 分析类任务

无开发工作的的分析类任务要基于已有信息来思考结论。当信息源导出了冲突的结论时，一定要提醒用户进行判断或补充信息，而不是自己做结论。
**分析的结果通常支持后续决策，非常重要！**

## 通用处理原则

**明确拒绝非书面的大块开发任务**

- 通过用更具体的文字复述任务需求实现对任务的确认
- 组合优于发明，总是选择新增代码更少的实现方式：优先复用已有组件、类型、脚本、依赖和既有模式。
- 引入新的依赖或通用组件时，必须和用户确认
- 优先确定修改范围，绝不修改范围外的任何文件
- 只通过分析当前仓库文件来确定“当前实现"
- 直接修改代码！除非明确要求，不做兼容性处理，不添加注释。
- 不要破坏基础系统的可用性： cargo test能过,buckyos-build能过
- 改协议、字段、命名时，必须检查前后端和文档是否联动

### 完成任务后，至少应能回答：

- 改了什么
- 为什么这样改
- 跑了什么验证
- 还有什么风险或未验证项

如果是较大任务，还应说明：

- 主要改动入口文件
- 是否影响文档、协议、共享类型、依赖


## 常见术语

- **Zone**：BuckyOS 为了区别于传统“集群/云”而使用的术语，指用户拥有的逻辑云/集群，是系统管理和调度的基本范围。
- **OOD (Owner Online Device)**：Zone 内的核心节点形态，承载 `system-config`、`scheduler` 等关键能力；单 OOD 或 `2n+1` OOD 都属于同一套模型。
- **Node**：Zone 内的一台设备节点。OOD 也是一种特殊 Node，普通 Node 主要负责运行应用和系统服务。
- **ZoneGateway**：Zone 的对外访问入口，负责公网访问、HTTPS/TLS 终止、子域名路由和暴露策略控制。
- **NodeGateway**：每台节点本地的统一网关能力，通常就是本机 `cyfs-gateway`，常见一致入口为 `127.0.0.1:3180`。（docker内不同)
- **SN (Super Node)**：公网协助节点，为 Zone 提供 DDNS、证书挑战和转发/中继能力。
- **DID**：BuckyOS 的身份基础设施。仓库里最常见的是 `User(Owner) DID`、`Device DID`、`Zone DID`,`Agent DID` 四类。
  - **Config 在身份语境中的特殊含义**：受 CYFS 历史命名影响，代码里的 `UserConfig`、`DeviceConfig` 往往本质上就是 DID Document，而不是普通运行配置。
- **system-config**：Zone 的 KV 真相源，不只是配置中心；用户、设备、服务、RBAC、调度结果都会写在这里。
- **scheduler**：确定性的状态推导器。它从 `system-config` 读取系统现状，推导出 `node_config`、`service_info`、`rbac`、`gateway_config` 等结果，再写回 `system-config`。
- **node-daemon**：节点收敛器。它读取本机 `node_config`，负责安装、部署、启动、停止、升级，把节点拉到目标状态。
- **service_info**：服务发现事实，由 scheduler 根据实例上报生成；客户端和 runtime 应以它为准决定“访问哪个实例”，而不是猜固定端口。
- **verify-hub**：统一登录和 token 签发中心。服务通常依赖它签发的 `session_token` 做身份验证，再结合 RBAC 做授权。
- **kRPC / KRPC**：BuckyOS 系统服务之间最常见的 RPC 方式，很多系统服务都暴露在 `/kapi/<service_name>` 下。
- **Info / Settings / Config / Doc**：仓库里反复使用的数据分类：
  - `Info`：运行时上报信息，通常由上报方写，其它角色只读。
  - `Settings`：用户可调配置，系统不会自动改。
  - `Config`：系统自动构造的运行配置，用户不应手改。
  - `Doc`：带签发人、可验证、通常只读的文档对象，如 `DeviceDoc`、`AppDoc`、`ServiceDoc`。


## 开发Tips: 避免常见错误
- 用buckyos-devkit 库来获得多平台支持



