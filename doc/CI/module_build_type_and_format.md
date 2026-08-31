# BuckyOS Module 编译类型与产物格式设计

## 1. 状态与范围

本文定义 `bucky_project.yaml` 中 module 的编译类型（`type`）与产物格式
（`format`），并作为后续 devkit 与 BuckyOS 项目配置一次性切换的实现依据。

本文只描述 module 从源码到可安装产物的构建模型，不改变：

- `apps.<app>.modules` 的覆盖安装语义；
- `apps.<app>.data_paths` 的保留已有数据语义；
- Linux、macOS、Windows 平台安装包的布局；
- pikg 包内部 AppDoc 的格式与安装端解析语义。

本次切换不提供 `type: web-pikg` 兼容别名，不设置弃用期，也不保留两套
执行分支。devkit 修改与 BuckyOS 配置迁移必须协调发布。
现有 `src/apps/sys_test/build.mjs` 和 `src/apps/jarvis_runtime/build.mjs` 仅作为
迁移前行为基线，不属于目标架构；一次性实现完成后，两份脚本都必须从项目中
删除。

## 2. 背景与问题

当前 `type: web-pikg` 同时表达了两个不同维度：

1. module 使用 Web/pnpm 工具链完成依赖安装和编译；
2. module 的最终产物是 pikg 包。

这会产生以下问题：

- 每增加一种“编译方式 × 产物格式”的组合，都可能需要增加新的 `type`；
- Web 依赖安装策略可能因为是否生成 pikg 而分叉；
- 构建器容易通过硬编码 module 名称发现 pikg；
- 将通用 pikg 打包逻辑从各项目的 `build.mjs` 移入 devkit 时，配置模型无法保持稳定；
- 未来 Rust 或其他编译类型需要产生 pikg 时无法复用同一打包策略。

因此必须将编译策略与产物策略拆开。

## 3. 总体模型

module 构建分为编译、产物落位和安装三个独立阶段。这里需要区分两类
rootfs：

- **staging rootfs**：仓库中的 `apps.<app>.rootfs`，是安装输入；
- **target rootfs**：`default_target_rootfs` 或命令行指定的位置，是最终安装目标。

```text
源码
  │
  ├─ type 编译策略
  ▼
编译结果
  │
  ├─ format 产物策略
  ▼
app staging rootfs 中的可安装产物
  │
  ├─ buckyos-install 按 apps.<app>.modules 复制
  ▼
target rootfs
```

三个阶段的职责如下：

| 阶段 | 配置入口 | 职责 |
| --- | --- | --- |
| 编译 | `modules.<key>.type` | 选择工具链、安装编译依赖、执行项目 build 并产出完整可打包结果 |
| 产物 | `modules.<key>.format` | 定义编译结果如何落入 app staging rootfs，并执行对应校验 |
| 安装 | `apps.<app>.modules` | 声明 staging rootfs 中的来源相对路径、target rootfs 中的目标相对路径及覆盖更新语义 |

`type` 不得隐式决定 `format`；`format` 也不得改变依赖安装和编译工具链。
执行层先由 builder 完成构建并返回结构化 `BuildResult`，再由 formatter 根据
`format` 消费该结果。复杂的构建来源和平台路径由 builder 解析后作为参数
返回，formatter 不要求开发者在项目配置中重复填写。

`apps.<app>.modules.<key>` 的值在 staging rootfs 与 target rootfs 中使用相同的
相对路径。本次修改不改变这一现有约定。

## 4. 配置模型

### 4.1 最终配置示例

```yaml
modules:
  node_active:
    type: web
    name: node_active
    src_dir: kernel/node_active

  buckyos_systest:
    type: web
    format: pikg
    name: buckyos_systest
    src_dir: apps/sys_test

  jarvis_runtime:
    format: pikg
    name: jarvis_runtime
    src_dir: apps/jarvis_runtime

apps:
  buckyos:
    modules:
      node_active: bin/node-active/
      buckyos_systest: data/cache/buckyos-systest.buckyos.bns.did-0.5.1.pikg
      jarvis_runtime: data/cache/jarvis.buckyos.bns.did-0.7.0.pikg
```

`buckyos_systest` 和 `jarvis_runtime` 都使用默认的
`pikg.meta_dir: dapp_meta`，因此可以省略整个 `pikg` 配置块。systest 显式使用
`type: web`，项目级组装由其 `pnpm run build` 调用 `prepare.mjs` 完成。Jarvis
省略 `type`，按默认的 `none` builder 处理；它的 source 已经是可工作的
`rootfs/bin/buckyos_jarvis`，因此不执行项目 build，也不再保留 `build.mjs`、
`package.json` 或 `deno.json`。

### 4.2 `type`

`type` 表示源码编译策略。当前需要支持：

- `none`：不执行构建命令，直接把 module 工作目录交给 formatter；
- `rust`：使用 Rust/Cargo 构建策略；
- `web`：使用 Node.js/pnpm 构建策略。

缺省的 `type` 为 `none`：

```yaml
type: none
```

配置序列化时省略默认的 `type: none`。`none` module 仍必须声明非空的
`src_dir`，该目录将解析为 `BuildResult.work_dir`。

`type` 只负责以下内容：

- 选择和准备编译工具链；
- 安装编译依赖；
- 选择目标平台和目标架构；
- 执行 module 的项目 build 命令，包括项目特有的生成与组装步骤；
- 返回 module work directory，以及 builder 自己能够解析的可选 artifacts 和
  parameters，不规定 formatter 所需的目录布局；
- 将依赖安装、编译或项目组装错误作为 module 构建错误返回。

`none` builder 不准备工具链、不安装依赖、也不执行任何命令。它只返回
`build_type: none`、解析后的 work directory 和空 artifact 列表。当前
`none` 的可用产物组合是 `format: pikg`；`files/none` 尚未实现，保留为以后
静态文件复制策略的扩展点。

Web module 的依赖安装和 build 命令由统一的 `web` 编译策略决定。所有 Web
module 均执行：

```text
pnpm install --no-lockfile
pnpm run build
```

`--no-lockfile` 表示本次构建既不读取也不生成 `pnpm-lock.yaml`。包括
`buckyos: latest` 在内的依赖声明都在每次构建时重新解析；项目不得提交
lockfile，也不再执行单独的 `pnpm update`。module 自己的 build script 不得
重复执行 install 或 update。

`format: files` 与 `format: pikg` 使用完全相同的依赖安装流程，`format` 不参与
命令拼接。`--skip-web` 也只依据 `type: web` 判断，因此会同时跳过
`web/files` 和 `web/pikg`。

项目特有的组装属于 `pnpm run build`。例如 systest 的 build 先生成 Web 产物，
再调用 `prepare.mjs` 组装 `dist/`；Web builder 不理解这些项目细节，只以命令
成功和 BuildResult 契约作为完成条件。formatter 不补做未完成的项目 build。

### 4.3 `format`

`format` 表示 module 的可安装产物格式。当前定义：

- `files`：普通文件或目录；
- `pikg`：由 devkit formatter 生成的单个 pikg 包文件。

缺省规则：

```yaml
format: files
```

即所有未显式声明 `format` 的现有 Rust/Web module 都按 `files` 处理。
配置序列化时省略默认的 `format: files`，非默认格式必须显式输出，以免对现有
项目文件产生无意义的批量改写。

`format` 是独立的配置维度，不应在解析层建立 `web + pikg` 之类的组合类型。
formatter 是产物阶段的主入口：它收到 `BuildResult.build_type`、规范化 artifact
和 builder parameters。需要解释 builder-specific artifact 的 formatter 可以在
内部按 build type 选择 strategy；不依赖 builder-specific artifact 的 formatter
则直接执行同一套通用行为。

本次 formatter 支持矩阵如下：

| `format` | `build_type` | formatter strategy |
| --- | --- | --- |
| `files` | `rust` | 使用 Rust BuildResult 中已解析的可执行文件路径、安装名和权限参数 |
| `files` | `web` | 从 Web BuildResult 的 work directory 取得 `dist/`，要求该目录存在并复制整个目录 |
| `pikg` | 任意 builder | 使用 BuildResult 的 work directory，以 `pikg.json` 为准执行 build/pack/info 并事务性落位 |

Web builder 的最小输出契约只有 work directory 和命令执行结果；它不要求
`dist/`，也不限制项目 build 还能产生什么。`dist/` 是 `files/web` strategy
自己的输入约定，缺失时由该 formatter 报错。`pikg` formatter 不读取或要求
`dist/`，只使用 BuildResult work directory 下 `pikg.json` 声明的 output 和
source。

`pikg` formatter 不声明 builder 支持列表，也不按 `build_type` 分支。
None、Web、Rust 以及未来新增的 builder 只要返回有效 work directory，并在
builder 阶段准备好 metadata 所引用的 source，就进入完全相同的 pikg 流程。

### 4.4 `pikg` 配置块

`pikg` 是 `format: pikg` 的专属可选配置块。当前字段如下：

| 字段 | 缺省值 | 说明 |
| --- | --- | --- |
| `meta_dir` | `dapp_meta` | 相对于 `BuildResult.work_dir` 的 metadata 目录，必须包含 `app.json` 和 `pikg.json` |

`pikg` 配置块不提供 prepare、copy、clean 或任意命令 hook。formatter 开始时，
`pikg.json` 中声明的 source 必须已经由项目 build 产出或原本就是可工作的静态
输入。项目特有的产物选择和目录组装留在项目自己的 build 流程中。

`pikg.json` 是以下信息的唯一权威来源，`bucky_project.yaml` 不重复声明：

- `output_dir`；
- `pikg_file`；
- `sub_pkgs` 及各自的 source。

formatter 在运行命令前解析 `pikg.json`。对每个 `source.type: path` 的路径，
formatter 将其解析为绝对路径，并作为独立的 `--allow-read` 参数传给
`npx --yes buckyos@latest`。因此 Jarvis 指向
`rootfs/bin/buckyos_jarvis` 的路径无需在
`bucky_project.yaml` 中重复配置。

本次实现不对 `meta_dir`、`output_dir` 或 metadata source 做“必须位于 module
源码目录内”的 containment 安全边界检查。formatter 仍会解析路径并验证输入
是否存在、类型是否正确，但不会因为路径位于源码目录外而拒绝；需要读取的
source 按解析后的路径逐个传入 `--allow-read`。安全边界收紧留作后续工作。

### 4.5 配置校验

读取项目配置、建立构建计划以及 formatter 开始执行前，必须在对应阶段完成以下
校验：

- `type` 省略时规范化为 `none`，显式值必须对应已注册的 builder；
- `none` module 必须具有非空的 `src_dir`；
- `files/none` 当前不在 files formatter 的 builder 支持范围内；
- `format` 省略时规范化为 `files`；
- `format` 必须对应已注册的 formatter；
- formatter 如果声明了 builder 支持范围，所选 builder 必须在范围内；
- `pikg` formatter 不声明 builder 支持范围；
- `type: web-pikg` 必须直接报配置错误，不做兼容转换；
- 只有 `format: pikg` 可以包含 `pikg` 配置块；
- `format: pikg` 的 module 必须至少存在一条同 key 的 app module 安装项；
- pikg 安装路径必须是非空相对路径，不能越出对应的 app staging rootfs；
- pikg 安装路径必须以 `.pikg` 结尾，且不能以 `/` 结尾；
- 同一 pikg module 的所有安装声明都必须能解析为明确的 staging 文件；
- `meta_dir` 必须能够解析到包含 `app.json` 和 `pikg.json` 的 metadata 目录；
- metadata source 在 build 后必须存在且类型正确；本期不做 module 源码目录
  containment 校验；
- `pikg info` 必须能从产物内部解析并验证 AppDoc；不得依赖 staging rootfs 中
  另行落位的 AppDoc 文件补全 pikg。

module 的身份使用 `modules` map 中的 key，而不是可选的 `name` 字段。例如
`buckyos_systest` 必须通过同 key 的
`apps.<app>.modules.buckyos_systest` 关联 pikg 安装声明。

字段级校验在读取 module 时完成；涉及 `apps` 的交叉引用校验在整个项目配置
解析结束后、任何清理和构建开始前完成。每条安装声明的期望 staging 路径为：

```text
resolve(project.base_dir, apps.<app>.rootfs, apps.<app>.modules.<module_key>)
```

解析后还必须验证结果仍位于该 app staging rootfs 内，不能只做字符串拼接。

## 5. 构建与产物发现流程

### 5.1 module 选择

执行 `buckyos-build --app=<app>` 时：

1. 读取 `apps.<app>.modules` 的 key；
2. 使用 key 查找 `modules.<key>` 构建声明；
3. 没有构建声明的安装项继续按静态文件处理；
4. 有构建声明的项目进入统一 module 构建计划，同一 key 只生成一个构建任务；
5. 为任务附加引用该 module 的所有 app 及其 staging 路径；
6. 构建计划显示规范化后的 `type` 和 `format`。

不带 `--app` 的全量构建和 `--select` 构建仍从顶层 `modules` 选择任务，但
`format: pikg` 的产物路径仍由同 key 的 app module 声明提供。静态安装项不
进入独立构建任务，但可由另一个 formatter 通过显式 key 生成，之后仍由
`buckyos-install` 安装。

因此新 pikg 的发现依据是 `format: pikg` 声明，而不是：

- 在脚本中维护 systest、Jarvis 等 module 名称列表；
- 扫描固定目录寻找所有 `.pikg`；
- 根据 `src_dir` 或文件名猜测 module 类型。

### 5.2 总体执行顺序

选中 module 后按以下顺序执行：

1. 解析并校验全部 module 声明；
2. 建立 module 构建任务及其 app 安装引用；
3. 按 `type` 选择 builder，并由 builder 执行依赖准备和源码编译；
4. builder 返回结构化 `BuildResult`，其中包含 build type、work directory、
   可选 artifacts 和 builder parameters；此时项目 build 与所需组装必须已经
   完成，但 formatter-specific 输出约定尚未校验；
5. 按 `format` 选择全局 formatter，由 formatter 校验 BuildResult 类型并选择
   对应 strategy；
6. formatter 产生、验证并将同一 module 的全部产物落入 staging rootfs；
7. 输出本次构建的产物记录；
8. `buckyos-install` 按 `apps.<app>.modules` 安装到 target rootfs。

formatter 必须从本次命令返回的确定路径取得新产物，不得用 glob 或“目标文件
原来就存在”判定成功。新产物完全生成并验证之前，不修改 staging rootfs；
构建失败时保留旧 staging 文件，且整个 `buckyos-build` 返回非零，因此后续
安装或平台打包不会继续执行。

### 5.3 `files` formatter

`files` 是默认 formatter。复杂的 source path 由 builder 解析，复制与目标
路径语义由 files formatter 负责：

- Rust builder 根据 Cargo target、目标平台和是否交叉编译，返回已经包含
  `.exe` 等平台后缀的单文件 artifact，并附带 `install_name` 和
  `executable: true`；
- Web builder 返回 module work directory；不要求或制造固定目录 artifact；
- files formatter 的 Rust strategy：安装路径以 `/` 结尾时追加
  `install_name`，否则视为明确文件路径；复制后按参数设置 `0755`；
- files formatter 的 Web strategy：从 `BuildResult.work_dir` 解析 `dist/`，
  要求它在 build 后存在且是目录，再把 app module 路径作为目标目录，沿用
  先删除目标目录再复制整个目录的行为；
- format 阶段不额外创建归档，`buckyos-install` 继续使用 overwrite module
  语义。

formatter 不重新计算 Cargo target path，也不从平台名称猜测扩展名；它只使用
Rust BuildResult 给出的最终 source path 和参数。Web 的 `dist/` 约定则只存在于
`files/web` strategy 中，Web builder 和 `pikg` formatter 都不依赖它。因此
不需要在 `bucky_project.yaml` 中增加 Rust target path、target triple、Web
dist path 等字段。

把默认 format 显式化不能改变普通 Rust/Web module 的构建目录、文件名、
可执行权限或复制结果。所有现有 `type: rust` 和 `type: web` module 均可继续
省略 `format`，项目文件无需迁移。

### 5.4 builder-independent 的 `pikg` formatter

pikg formatter 不注册 Web、Rust 或其他 builder-specific strategy。每个
`format: pikg` module 都只消费通用的 `BuildResult.work_dir`，并按以下相同
顺序处理：

1. 解析并校验 `pikg` 配置块、`app.json` 与 `pikg.json`；
2. 从 `pikg.json` 解析全部 path source，验证 builder 完成后的 source 已存在且
   类型正确，并生成最小的重复 `--allow-read` 参数；不做源码目录 containment
   校验；
3. 在 `BuildResult.work_dir` 下执行：

   ```text
   npx --yes buckyos@latest [--allow-read <source> ...] --output json pikg build <meta_dir>
   ```

4. 从 build 响应的 `data.dist_dir` 取得本次生成目录，并验证其规范路径与
   `pikg.json.output_dir` 解析出的路径一致；
5. 执行并解析：

   ```text
   npx --yes buckyos@latest --output json pikg pack <dist_dir>
   ```

6. 从 pack 响应的 `data` 取得 `pikg_path`、`size`、`pikg_digest` 与
   `app_doc_object_id`，并执行：

   ```text
   npx --yes buckyos@latest --output json pikg info <pikg_path>
   ```

7. 交叉验证 build、pack 与 info 的 `data`：`pikg_path` 必须位于本次
   `dist_dir`，basename 必须同时等于 `pikg.json.pikg_file` 和 app module
   安装路径的 basename；info 的规范 `pikg_path`、`size`、`pikg_digest` 必须
   与 pack 一致，build 与 pack 的 `app_doc_object_id` 必须同时等于
   `info.app.app_doc_object_id`，`info.valid` 必须为 true，`info.app.did` 与
   `info.app.version` 必须分别等于 `app.json` 的 DID 与 version；
8. 将 pikg 复制到临时文件，备份旧 staging 文件，使用同目录 rename 完成
   替换；任一步失败时回滚该 module 的全部 staging 文件；
9. 对所有 app 安装引用生成结构化产物记录。

三个 CLI 命令的 stdout 都必须是且只能是一个 JSON success envelope：
`schema_version` 为 1，`ok` 为 true，且存在对象类型的 `data` 与 `meta`。
formatter 只从 `data` 读取命令结果。子命令退出非零、stdout 混入其他文本、
envelope 缺字段或字段类型错误均视为失败；原始 stderr 保留在错误上下文中。

三个命令都固定以 `npx --yes buckyos@latest` 开头。PIKG formatter 自己负责
解析并启动 npmjs 上的 latest 工具，不要求 module 的 `package.json` 声明
`buckyos`，也不检查 module 本地 `node_modules/buckyos` 的版本；`--yes` 禁止
npx 在无人值守构建中等待安装确认。

该命令必须通过 devkit 的统一跨平台 helper 调用。helper 接收已验证的 argv
值，按当前平台正确引用后组成一条命令字符串，并以 `shell=True` 执行；工作
目录固定为 `BuildResult.work_dir`，stdout 单独捕获用于 JSON 解析，stderr
原样保留。Linux、macOS 和 Windows 使用同一 argv 形式，由 shell 在 Windows
上解析 `npx.cmd`。

`pikg build` 本身以受管理的临时目录替换 `dapp_dist`，`pikg pack` 也以临时
文件替换最终 pikg。formatter 再对 staging rootfs 做一次事务性替换。因此
不再需要在编译前删除旧 staging pikg，也不会把旧文件误判成本次产物。

formatter 输出的结构化产物记录至少包含：

```text
module_key
type
format
source_path
install_app
install_path
size
digest
```

如果同一 module 被多个 app 安装，builder 只构建一次，formatter 也只处理一次；
formatter 将同一个已验证产物事务性落位到每个去重后的 staging 路径，并为
每条 app 安装声明生成记录。

## 6. 包内 AppDoc 规则

AppDoc 是 pikg 自描述信息的一部分，不是 `format: pikg` 产生的第二个安装
文件。`pikg build` 根据 metadata 生成 AppDoc，`pikg pack` 将其收入包内；
formatter 不把 `<dist_dir>/APPDOC.json` 单独复制到 app staging rootfs，也不在
`apps.<app>.modules` 中为它增加安装项。

`pikg info` 必须验证包内存在可解析的 `APPDOC.jwt` 或 `APPDOC.json`，并验证其
身份与 pikg metadata、文件名相符。缺失或无效的包内 AppDoc 必须使 formatter
在更新 staging pikg 前失败。

当前预安装配置只向 Control Panel 提供 `pikg_path`。预安装 reconciler 使用
`PikgReader` 打开 pikg，从 inspection 中取得 `app_doc` 与
`app_doc_object_id`，后续安装流程再将已验证的 AppDoc 写入 NamedStore。因此
`rootfs/local/did_docs` 中的 systest AppDoc 副本不是安装输入，应从项目配置和
构建流程中删除。

## 7. 内部实现边界

devkit 内部使用两个平级 registry，并以 `BuildResult` 作为唯一交接面：

```text
builders[type] -> Builder
Builder.plan(module, context) -> BuildResultSpec
Builder.build(module, context) -> BuildResult

formatters[format] -> Formatter
Formatter.supports(BuildResultSpec.build_type)
Formatter.materialize(module, BuildResult, install references)
```

`BuildResultSpec` 在执行昂贵构建前提供 build type 与输出契约，使具有
builder 支持范围的 formatter 能够提前拒绝不支持的组合；pikg formatter 没有
这个限制。`BuildResult` 至少包含：

```text
module_key
build_type
work_dir
artifacts[] = { role, path, kind, install_name?, executable? }
parameters = { ...builder-specific values... }
```

None BuildResult 只包含解析后的 `work_dir`，并使用空 artifacts/parameters。
Web BuildResult 只强制要求 `work_dir`；`artifacts` 可以为空，也可以携带项目
选择返回的附加产物，不规定名称或目录布局。Rust BuildResult 返回的 artifact
`path` 必须由 builder 完成平台与 target 解析，并携带最终可执行文件后缀。
formatter 不读取 Cargo target 目录结构，也不自行猜测源文件平台路径；目标安装
名沿用 artifact 的安装名与后缀规则。这些值都不要求出现在项目配置。
`parameters` 是 builder 与 formatter strategy 之间的内部契约，不直接映射成
`bucky_project.yaml` 的自由字段。

需要处理 builder-specific artifact 的 formatter 可以按 build type 注册内部
strategy：

```text
formatters["files"].strategies["rust"]
formatters["files"].strategies["web"]
formatters["pikg"]  # 单一通用流程，无 builder strategy
```

`files` 同时使用 `format` 与 `build_type` 只是在执行层选择 strategy；
`pikg` 完全不根据 `build_type` 选择行为。配置始终是独立的 `type` 和
`format`，不是 `web-pikg` 之类的组合 module type。

配置模型应将 `format` 放在通用 module 信息上。`NoneModuleInfo`、
`WebModuleInfo` 和 `RustModuleInfo` 都持有通用 format 配置；
`WebPikgModuleInfo`、`add_web_pikg_module` 及其导出全部删除。三种 builder
产生的 BuildResult 都可以交给同一个 pikg formatter。

本次 builder 边界为：

- None builder 不执行命令，只返回解析后的 module work directory；
- Rust builder 只负责 Cargo 参数、target triple、交叉编译和构建，并返回
  解析完成的可执行文件 artifact；
- Web builder 负责 `pnpm install --no-lockfile` 与 `pnpm run build`；项目 build
  必须完成源码编译以及项目特有的产物组装，之后 builder 返回 work directory；
  Web builder 不要求 `dist/`，也不规定或解析标准 Web artifact；
- builder 不读取 app install path，不向 staging rootfs 复制文件，也不执行
  pikg 命令；
- formatter 负责目标路径解释、复制或打包、校验、事务落位和产物记录；formatter
  不执行项目特有的 prepare 或组装；
- 公共层提供构建计划、app 引用解析、app staging 安全路径、事务复制和命令执行 helper。

交互式选择和日志使用 `type/format`（例如 `none/pikg`）显示任务。
`--skip-web` 及 install 的同名选项仍只检查配置中的 `type`。

禁止继续出现以下设计：

- `WEB_PIKG` 之类的组合枚举或 module class；
- `if type == "web-pikg"` 分支；
- `PIKG_MODULES = ["buckyos_systest", "jarvis_runtime"]` 名单；
- 把 Rust target path 或平台后缀写入 `bucky_project.yaml`；
- formatter 自行重建某个 builder 的 target/source path；
- builder 直接复制到 staging rootfs 或调用 pikg CLI；
- 由 module 自己的 `build.mjs` 调用 pikg CLI 或复制最终产物；
- 仅通过 glob 找到某个 `.pikg` 就判定构建成功；
- 为项目差异提供任意 shell hook。

未来增加新 builder 时，只需定义新的 BuildResult contract；需要支持某种 format
时，再为相应 formatter 注册 strategy。公共 formatter 行为和项目配置模型均
不需要随 builder 数量做笛卡尔积扩张。

## 8. 错误处理

以下情况必须使 `buckyos-build` 返回非零，且不得更新任何 staging 文件：

- 未知 `type` 或 `format`，或具有 builder 支持范围的 formatter 不支持
  builder 声明的 BuildResult 类型；
- 继续使用 `type: web-pikg`；
- pikg module 没有同 key 的安装声明；
- pikg 安装路径不是安全的相对路径、扩展名错误或以 `/` 结尾；
- `pikg` 配置块或 metadata 路径无效；
- 项目 build 成功后，`pikg.json` 声明的 source 仍缺失或类型错误；
- Web 编译命令失败；
- `files/web` 的 `<work_dir>/dist` 缺失或不是目录；
- `npx` 无法解析或启动 `buckyos@latest`；
- `npx --yes buckyos@latest` 的 pikg `build`、`pack` 或 `info` 返回非零、stdout
  不是唯一合法的 success envelope，或 envelope 报告失败；
- CLI 返回的 dist/pikg 路径、文件名、`size`、`pikg_digest`、AppDoc 身份或
  object id 交叉校验不一致；
- 新 pikg 不存在、为空、不是普通文件或是符号链接；
- pikg 内部缺少有效 AppDoc，或 AppDoc 身份与 metadata 不一致；
- staging 事务失败且无法完整回滚；
- builder 返回的 BuildResult 不满足其声明的 spec 或 formatter 输入契约。

错误信息必须包含 module key、`type`、`format`、源目录和相关产物路径。涉及
app 引用时还必须包含 app name 与原始 install path；涉及子命令时必须包含
失败阶段和退出码，但不得吞掉原始 stderr。

## 9. 一次性迁移范围

实现时需要在同一轮变更中完成最终形态，不落地“新配置 + 旧 build.mjs”中间态：

1. devkit 配置模型增加 `none` type、`format` 和 `pikg` 配置块；默认 type 为
   `none`，默认 format 为 `files`；
2. devkit 建立包含 None/Web/Rust builder 的 registry、formatter registry、
   BuildResultSpec 与 BuildResult 交接契约，不引入项目级 build source 配置；
3. 将当前 Rust/Web 复制逻辑移入 files formatter 的对应 strategy，保持行为
   不变，现有 files module 配置不做迁移；
4. 删除 `WebPikgModuleInfo`、`add_web_pikg_module`、专用 build 函数、导出和
   `web-pikg` 执行分支；
5. 在通用 pikg formatter 中实现 metadata/source、allow-read、
   `npx --yes buckyos@latest`、CLI JSON envelope、跨命令结果、包内 AppDoc 和
   事务落位校验，不实现项目组装，也不限制 builder type；
6. 保证 build/install 的 `--skip-web` 继续按 `type: web` 生效；
7. 将 `buckyos_systest` 改为 `type: web, format: pikg`，增加由
   `pnpm run build` 调用的 `prepare.mjs` 完成所需组装；删除独立 systest
   AppDoc 安装项、对应 `.gitignore` 特例，以及工作树中遗留但未跟踪的
   `src/rootfs/local/did_docs/buckyos-systest.buckyos.bns.did.doc.json`；
8. 将 `jarvis_runtime` 改为省略 type 的 `format: pikg`，由 NoneBuilder 直接
   把 `apps/jarvis_runtime` 交给 formatter；
9. 删除两个 module 的 `build.mjs`，并删除 Jarvis 不再需要的 `package.json`
   与 `deno.json`；
10. systest 的 `package.json` 使用 npmjs 的 `"buckyos": "latest"`，build
    执行 Web 编译后调用 `prepare.mjs`，且不提交 lockfile；
11. pikg formatter 自行通过 npx 解析 `buckyos@latest`，Jarvis 不声明 npm
    依赖；
12. 更新 devkit 的公开配置文档、测试和导出；
13. 将 devkit `pyproject.toml` 的版本从 `0.7.32` 升到 `0.7.33`；
14. 更新 PVE 构建配置，使 devkit 与 BuckyOS 配置使用匹配的提交或分支。

devkit 预计涉及的现有文件至少包括：

- `src/project.py`、`src/build.py`、`src/build_none.py`、`src/build_web_apps.py`；
- `src/prepare_rootfs.py`、`src/install.py`、`src/__init__.py`；
- 新增 BuildResult contract、files/pikg formatter 与事务落位帮助代码；
- `tests/test_web_pikg_modules.py`、`tests/test_build_options.py`；
- `PROJECT_CONFIG_EXAMPLE.md`、`pyproject.toml`。

BuckyOS 预计涉及：

- `src/bucky_project.yaml`、`.gitignore`；
- `src/apps/sys_test/package.json`、`src/apps/sys_test/prepare.mjs`；
- 删除两个 module 的 `build.mjs`，并删除 Jarvis 的 `package.json` 与
  `deno.json`；
- 清理旧 systest AppDoc 的未跟踪工作树副本。

PVE 只更新协同版本/分支配置，不增加 pikg 名单、pikg 命令或复制逻辑。

由于不提供兼容策略，不能让“只支持新配置的 devkit”和“仍使用旧配置的
BuckyOS”进入同一构建，也不能用旧 devkit 构建已经迁移的 BuckyOS 配置。
跨仓库发布必须通过固定提交、配套分支或等价方式保证二者同时切换。

## 10. 测试与验收标准

### 10.1 配置测试

- 缺省 `format` 被解析为 `files`；
- 缺省 `type` 被解析为 `none`，序列化时省略；显式 `type: none` 同样有效；
- `none` module 缺少有效 `src_dir` 或使用尚未支持的 `files/none` 时失败；
- 默认 `files` 序列化时省略，`pikg` 配置块能正确往返；
- `type: web, format: pikg` 解析成功；
- `type: rust, format: pikg` 解析成功；
- `type: web-pikg` 解析失败；
- 未知 format、受限 formatter 不支持 BuildResult type，或 format/options 不匹配时失败；
- pikg module 缺少同 key 的 app module 时失败；
- pikg 安装目标不安全、不是 `.pikg` 或以 `/` 结尾时失败；
- meta_dir 不存在、缺少 metadata 文件或 metadata 格式错误时失败；
- 没有被 formatter 引用的静态安装项不要求顶层 module；
- 本地 overlay 覆盖 module 时仍能正确规范化 format/options。

### 10.2 builder/formatter 单元测试

- Rust builder 为本机构建和交叉编译返回正确的最终 artifact path、
  `install_name` 与 executable 参数；
- Web builder 只要求正确的 work directory；build 未产生 `dist/` 也可成功，
  artifacts 为空或包含额外项目产物都不影响 builder；
- files formatter 不自行计算 Rust target path，并能按 Rust BuildResult 保持
  现有文件复制行为；其 Web strategy 从 work directory 取得 `dist/`，缺失或
  不是目录时失败；
- pikg formatter 不要求 `dist/`，只按 `pikg.json` 解析 output 和 source；
- None builder 不执行命令，返回解析后的 work directory 和空 artifacts；
- pikg formatter 不声明 builder 支持范围；None、Web、Rust 或测试用新 builder
  的 BuildResult 均进入同一 build/pack/info 路径；
- BuildResult 不符合 spec、或 formatter 不支持 build type 时在落位前失败；
- 正确解析 `pikg.json` 的 output、文件名和多个 path source；
- 每个 path source 产生一个正确解析的重复 `--allow-read`；源码目录外的 source
  不因 containment 被拒绝，并使用其解析后的绝对路径授权；
- 项目 build 后任一 metadata path source 缺失或类型错误时失败；
- module 不存在 `package.json` 或本地 `node_modules/buckyos` 时，formatter
  仍直接进入通用 CLI 路径；
- Linux、macOS、Windows 都把经过平台引用、以
  `npx --yes buckyos@latest` 开头的单条命令字符串用 `shell=True` 执行；
- build/pack/info 任一失败、stdout 混入日志、JSON envelope 无效或 `ok` 非
  true 时失败；
- build/pack/info 的路径、文件名、`size`、`pikg_digest`、AppDoc DID/version
  或 `app_doc_object_id` 任一交叉校验不一致时失败；
- pikg 缺失包内 AppDoc、AppDoc 无法解析或身份不一致时，在落位前失败；
- formatter 不向 staging rootfs 单独复制 `APPDOC.json`；
- pikg 落位成功时原子替换，任一步失败时完整回滚；
- 同一 module 被多个 app 引用时只编译和打包一次；
- 产物记录包含 size 与 digest。

### 10.3 集成与回归测试

- `web/files` 与 `web/pikg` 都严格执行 `pnpm install --no-lockfile` 后再执行
  `pnpm run build`，不执行单独的 `pnpm update`；
- 构建不读取或生成 `pnpm-lock.yaml`，BuckyOS 不提交 Web module lockfile；
- 测试 registry 中发布较新的 `buckyos` 后，PIKG formatter 的下一次
  `npx --yes buckyos@latest` 调用会按 registry/cache 规则解析 latest；
- format 不改变 Web 编译命令；
- `--skip-web` 同时跳过 `web/files` 和 `web/pikg`；
- pikg formatter 不执行传统 web/files 的 `dist/` staging 复制，也不负责组装
  `dist/`；
- 增加一个测试用新 pikg module 后，无需修改构建器名单即可被发现；
- 现有 Rust module 不修改项目配置，仍按当前 target/platform 规则复制单个文件；
- 现有传统 Web module 不修改项目配置，仍从 `<src_dir>/dist` 复制整个目录；
- 不存在或不调用任何 module `build.mjs`；systest 的 `pnpm run build` 会调用
  `prepare.mjs`；Jarvis 使用 NoneBuilder，且不存在 `package.json`、`deno.json`
  或项目 build 命令；
- systest 的组装结果与 pikg 内容和迁移前等价，且不再产生独立 AppDoc staging
  文件；旧 AppDoc 安装声明、`.gitignore` 特例和遗留工作树文件均不存在；
- Jarvis 的 formatter 调用包含 `rootfs/bin/buckyos_jarvis` allow-read；
- `buckyos install --app=buckyos` 能从源码 staging rootfs 安装两个 pikg，
  预安装流程能从各自 pikg 内部读取 AppDoc；
- Linux、macOS、Windows 的本地安装包都从同一 staging rootfs 声明获得 pikg；
- devkit、BuckyOS 通用脚本与 PVE 中不存在 pikg module 名称硬编码；
- BuckyOS 仓库在构建后不会因 formatter 生成或改写受版本控制文件而变为 dirty。

## 11. 非目标与后续工作

本次实现不包含：

- 将 Rust/Web 的 build source path 暴露为项目配置字段；
- 让 builder 负责 staging rootfs 的 format-specific 复制；
- 修改 `dapp_meta/app.json` 或 `pikg.json` 的 schema；
- 将 pikg 内部 AppDoc 额外写入 `local/did_docs` 或其他 staging 路径；
- 改变 pikg 文件命名规则；
- 改变 `apps.<app>.modules` 安装语义；
- 让 formatter 执行项目特有的 prepare、copy、clean 或任意命令 hook；
- 新增 `allow-write` 等 buckyos CLI 权限参数；
- 对 pikg 的 `meta_dir`、`output_dir` 或 metadata source 增加 module 源码目录
  containment 安全边界；
- 为 `web-pikg` 或 module `build.mjs` 提供兼容或弃用周期。

后续若 pikg CLI 能直接返回全部外部读路径需求，devkit 可以停止自行解析
`sub_pkgs.*.source.path`，但 `bucky_project.yaml` 的 `type/format/pikg` 模型
保持不变。
