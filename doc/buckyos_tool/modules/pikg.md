# PIKG 模块需求

> 状态：Draft  
> 对应 module：`pikg`  
> 适用范围：普通 App 的本地开发和发行候选构造

## 1. 目标

`pikg` 模块在开发者机器上管理 PIKG 的构造、封装、严格离线验证和中间产物清理。
它落实 PIKG-first 边界：开发目录可以包含 AppDoc 模板和多个 subpackage 构建输出，但交给
测试、安装或后续发布阶段的唯一发行候选是一个 PIKG。

标准开发流程是：

```text
<project>/
    | buckyos pikg init
    v
dapp_meta/
    | buckyos pikg build
    v
dapp_dist/
    | buckyos pikg pack
    v
<app>-<version>.pikg
    | buckyos app install --pikg <path>
    v
Installer
```

`dapp_dist` 是可再生成、由工具管理的中间快照，不是第二种对外发行物。本模块必须在
BuckyOS 未启动、未配置 Zone/identity、没有正式发行密钥且禁止网络时完成整个本地闭环。
上层 App 生命周期中的概念性 `build` 阶段在 CLI 中对应 `pikg build` + `pikg pack`；只有后者
产生的 PIKG 可以交给测试、安装或发布阶段，任何下游模块都不能以 `dapp_dist` 作为公开输入。

## 2. 边界

### 2.1 负责

- 通过最小交互问答或机器可读输入生成 `dapp_meta/app.json` 和 `dapp_meta/pikg.json`；
- 读取 `dapp_meta/app.json` 和 `dapp_meta/pikg.json`；
- 解析相对/绝对路径的 subpackage 输出，将目录或已完成的 `tar.gz` 规范化为 payload；
- 将本地已存在的 Docker image 按不可变 image ID 导出为 payload；
- 生成 PackageMeta、Object ID、`APPDOC.json`、`PACKAGE_META.json` 和内容索引；
- 从受管理的 `dapp_dist` 快照封装完整 PIKG，并使用与 Installer 同源的 verifier 回读验证；
- 对任意指定 PIKG 输出机器可读的结构、身份、subpackage、digest 和验证摘要；
- 安全删除当前 `dapp_meta` 所声明且可证明由本工具生成的 `dapp_dist`。

### 2.2 不负责

- `init` 不创建源码、Dockerfile、CI 配置或 App 业务目录，不扩展为通用工程脚手架。
- 不执行 App 源码的 npm/cargo/make 或任意 build script；subpackage 路径必须已经指向构建结果。
- 不构建 Docker image，不访问 registry，不执行 pull/login/push；Docker image 必须已存在于本地。
- 不安装或运行 App；本地/测试 Zone 安装由 [App 模块](app.md) 通过标准 Installer 完成。
- 不使用 Owner key 签名 AppDoc，不上传 PIKG，不更新 Repo/BNS，不判定某个版本是否已公开发布。
- 首版不输出仅含某个 subpackage 的部分 PIKG，也不提供 `pack-subpkg`、`sign-package-meta` 或
  `publish-package-meta` 等对象级工作流。
- 不解析或下载普通 App 的第三方 package 依赖。

## 3. 开发目录与输入模型

### 3.1 标准目录

```text
<project>/
├── dapp_meta/
│   ├── app.json
│   └── pikg.json
├── web/dist/                    # 示例：已完成的构建输出
└── dapp_dist/                   # 生成，不应提交到源码库
    ├── .buckyos-pikg-dist.json
    ├── APPDOC.json
    ├── PACKAGE_META.json
    ├── web.tar.gz
    ├── amd64_docker_image.tar.gz
    └── demo-0.1.0.pikg          # `pack` 的最终输出
```

`app.json` 是面向开发者的 App 定义，不是 AppDoc 模板，也不使用 AppDoc schema。
`pikg.json` 定义构建输出与 subpackage source。`build` 把两个开发输入和实际 payload 组合成
`dapp_dist/APPDOC.json`、`PACKAGE_META.json` 及 subpackage 归档。两个源文件都不得被 `build`
就地改写。

### 3.2 `app.json`

`app.json` 使用独立的 `buckyos.dapp-meta.v1` 开发态 schema。它只描述不依赖构建结果的 App 级语义，
例如：

```json
{
  "schema_version": 1,
  "did": "did:bns:demo.root",
  "name": "demo",
  "version": "0.1.0",
  "owner": "did:bns:root",
  "author": "did:bns:root",
  "show_name": "Demo",
  "categories": ["dapp"],
  "permissions": [],
  "selector_type": "single",
  "service_config_tips": {}
}
```

该 schema 可以与最终 AppDoc 使用不同的字段结构和默认值；具体转换由 PIKG 共享协议核心负责。
为了避免把内部对象图再次暴露给开发者，`app.json` 必须拒绝：

- `pkg_list`、`pkg_id`、`pkg_objid` 和 PackageMeta；
- `content`、`size`、payload digest 和 content index；
- 普通 App 不支持的第三方 `deps` 或 `source_url`；
- AppDoc Object ID、签名、BNS revision 和其它只有构建/发布后才能确定的字段。

`build` 必须对 `app.json` 采用严格 allowlist 解析，未知或只属于 AppDoc 的字段直接拒绝，
不得将整个 JSON 透传到 `APPDOC.json`。

### 3.3 `pikg.json`

`pikg.json` 是本地构造 manifest。首版 schema 如下：

```json
{
  "schema_version": 1,
  "output_dir": "../dapp_dist",
  "pikg_file": "demo-0.1.0.pikg",
  "sub_pkgs": {
    "web": {
      "required": true,
      "source": {
        "type": "path",
        "path": "../web/dist"
      }
    },
    "script": {
      "required": true,
      "source": {
        "type": "path",
        "path": "/opt/build/demo-script.tar.gz"
      }
    },
    "amd64_docker_image": {
      "selector": {
        "os": "linux",
        "arch": "x86_64"
      },
      "required": true,
      "source": {
        "type": "docker-image",
        "image": "local/demo:0.1.0-amd64"
      }
    }
  }
}
```

字段规则：

- `schema_version`：必填，首版固定为 `1`；未知版本直接拒绝。
- `output_dir`：可选，默认为 `../dapp_dist`；相对路径以 `dapp_meta` 目录为基准，也可显式使用
  绝对路径。该目录同时容纳最终 PIKG。
- `pikg_file`：可选，默认为 `<app-name>-<version>.pikg`；必须是 `.pikg` 结尾的安全单段文件名，
  不得包含路径分隔符或 `..`。
- `sub_pkgs`：开发者声明的逻辑 subpackage 集合，key 将成为生成 `AppDoc.pkg_list` 的 key，
  必须满足 `[A-Za-z0-9._-]+` 的安全单段命名规则。
- `selector`：可选的目标 OS/Arch 等选择条件；与已知 key 的默认选择规则冲突时直接拒绝。
- `required`：可选，省略时为 `true`；该语义会进入生成的 `pkg_list` entry。
- `source.type=path`：`path` 可以是相对路径或绝对路径；相对路径以 `dapp_meta` 为基准。
- `source.type=docker-image`：`image` 是本地 Docker daemon 中已存在的 image name/tag 或 image ID。
- 所有声明的 subpackage 在首版都必须被完整收纳到 PIKG，不支持以 `source_url` 或第三方依赖
  替代当前 payload。

`build` 根据 App identity/version、`sub_pkgs` key 和 namespace 规则生成 `pkg_id`，根据实际 payload 生成
PackageMeta、`pkg_objid` 和 digest，再将 `selector`、`required` 及 Docker 派生字段组合成最终
`AppDoc.pkg_list`。首版不允许开发者覆盖 `pkg_id` 或 `pkg_objid`。

### 3.4 路径 subpackage

- 路径指向目录时，`build` 将目录内容归档为 `<sub_pkg_name>.tar.gz`。归档必须使用稳定文件
  排序和规范化 metadata，不得包含源路径、用户名或宿主机绝对路径。
- 路径指向普通文件时，首版只接受已完成的 `.tar.gz`，并将其最终压缩字节原样复制为
  `<sub_pkg_name>.tar.gz`，不解压后二次打包。
- 输入不存在、类型不支持、包含设备文件/FIFO，或 symlink 解析后逃出已声明的输入根时，
  `build` 必须失败。
- `output_dir` 经过 canonicalize 后不得与 `dapp_meta` 或任意 subpackage 输入目录重合，避免 build/clean
  覆盖或删除源输入。

### 3.5 Docker subpackage

`build` 对 `docker-image` source 执行以下固定流程：

1. 使用 Docker image inspect 解析本地 image reference，取得不可变 image ID/digest；
2. 在任何大文件写入前，校验该 digest 的格式并把 image reference 固定为该不可变 identity；
3. 使用解析后的不可变 image ID 执行 Docker image save，而不再使用可变 tag；
4. 将导出结果封装为 `<sub_pkg_name>.tar.gz`，计算最终压缩字节的 size 和 SHA-256；
5. 在输出 `APPDOC.json` 的对应 `pkg_list` entry 中写入规范化 `docker_image_name`、
   `docker_image_digest` 和新生成的 `pkg_objid`。

`docker_image_digest` 表达运行时 image identity，subpackage 归档的 SHA-256 表达 PIKG payload identity，两者
不得混用。如果 image 不存在、Docker daemon 不可用、digest 无法固定或导出期间 image identity
发生变化，必须失败且不得隐式从 registry 拉取。

## 4. `dapp_dist` 模型

`pikg build` 在同级临时目录中完成全部写入和验证，成功后再原子替换 `output_dir`。失败或
中断时不得留下可被 `pack` 接受的半成品。

`dapp_dist` 至少包含：

- `.buckyos-pikg-dist.json`：工具所有权和构建快照 manifest，记录 schema version、tool version、
  canonical `dapp_meta` 路径的单向标识 `meta_root_id`、源指纹、生成文件 allowlist、subpackage digest、
  AppDoc Object ID 和 `pikg_file`；该文件不进入 PIKG。
- `APPDOC.json`：由 `app.json`、`pikg.json.sub_pkgs` 和实际 payload 构造的完整未签名 AppDoc
  candidate，包含最终 `pkg_list`。
- `PACKAGE_META.json`：包含 PackageMeta objects 和 content index，与 PIKG 协议中的同名 entry 一致。
- 每个 `<sub_pkg_name>.tar.gz`：最终 payload 字节，其 hash 计算对象是 `.tar.gz` 文件本身。
- `pikg_file` 指定的 `.pikg`：仅在 `pack` 成功后存在。

如果 `output_dir` 已存在但缺少有效 ownership manifest，或 manifest 属于另一个 `dapp_meta`，`build`
必须拒绝覆盖。重新 build 可以替换同一 `dapp_meta` 的旧快照，并删除由旧 manifest 记录的旧
PIKG。实际目录中出现 allowlist 以外的用户文件时必须拒绝替换，不得删除、搬运或默默保留这些
文件到新快照。

## 5. 命令清单

| 命令 | 主要输入 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- | --- |
| `pikg init [<project-dir>]` | 工程目录，默认 `.` | write | sync/local | 以最少必要输入生成 `dapp_meta` |
| `pikg build [<meta-dir>]` | `dapp_meta`，默认 `./dapp_meta` | write | sync/local | 构造受管理的 `dapp_dist` 快照 |
| `pikg pack [<dist-dir>]` | `dapp_dist`，默认 `./dapp_dist` | write | sync/local | 封装完整 PIKG、回读验证并原子落盘 |
| `pikg info <pikg-path>` | 本地 `.pikg` | read | sync/local | 严格离线验证并输出内容/信任摘要 |
| `pikg clean [<meta-dir>]` | `dapp_meta`，默认 `./dapp_meta` | destructive | sync/local | 删除该 manifest 所属的整个 `dapp_dist` |

首版不提供 subpackage-only pack option。新增该能力前必须先设计部分 PIKG 的目标完备性和安装语义，
不能只加一个跳过校验的命令行开关。

## 6. 命令语义

### 6.1 `init`

`init` 只创建 PIKG 开发元数据，不构建源码、不创建 `dapp_dist`、不导出 Docker image，也不生成
AppDoc/PackageMeta。目标工程目录必须已存在且可写；本命令只在其下创建 `dapp_meta`。
未通过 option/`--input` 提供全部必要字段时，只有交互 TTY 且未设置 `--non-interactive` 才进入问答；
其它情况立即返回缺失字段。

交互模式只询问无法安全推导的必要信息：

| 输入 | 引导策略 | 派生结果 |
| --- | --- | --- |
| App name | 默认使用工程目录名规范化后的值；只有无法合法规范化时必须重新输入 | `app.json.name`、默认 `show_name` 和 PIKG 文件名 |
| Owner DID | 无默认值，必须输入；本地命令不从 profile 或设备身份推测 | `owner`、默认 `author` 和 App DID/namespace |
| App kind | 必须从 `static-web`、`script`、`docker` 中选择一项 | 初始 subpackage key、category 和 selector |
| Source | path 型输入构建输出目录/`.tar.gz`；Docker 输入本地 image name/ID | `pikg.json.sub_pkgs.*.source` |

其它字段不作为初始引导问题：

- `version` 默认 `0.1.0`；
- `show_name` 默认为 App name，`author` 默认为 Owner DID；
- `permissions` 默认为空，`selector_type` 默认为 `single`，服务端点和 mount/database 声明默认为空；
- `output_dir` 默认 `../dapp_dist`，`pikg_file` 由 name + version 生成；
- `static-web` 生成 `web`，`script` 生成 `script`，Docker 使用本地 image inspect 推导 arch 并生成
  `<arch>_docker_image`；`required` 默认为 `true`；
- `pkg_id`、`pkg_objid`、`pkg_list`、digest 和 Object ID 不询问，它们始终由 `build` 生成。

`init` 只生成一个主 subpackage，不追问“是否继续添加”。需要 Web + Agent、多平台 Docker 等组合时，
用户在 init 完成后直接编辑 `pikg.json.sub_pkgs`；后续可以另行设计非必要的 add-subpackage 能力，
但不增加首次引导的问题数。

因此，工程目录名可用时，一条最小交互路径只需 Owner DID、App kind 和 Source 三个回答。
引导结束后应显示所有派生值的摘要，而不再逐项要求用户确认默认字段。

path 型 Source 在 `init` 输入中的相对路径以 `<project-dir>` 为基准，写入 `pikg.json` 时必须转换为
以 `dapp_meta` 为基准的相对路径；绝对路径保持绝对形式。该路径在 init 时可以尚未生成，此时输出
warning 但仍可创建元数据；`build` 时必须存在。Docker Source 必须在 init 时已存在于本地，
否则无法安全推导 arch 并必须失败。

交互入口示例：

```bash
buckyos pikg init .
```

非交互模式使用同一 input schema，可通过主 PRD 的 `--input <file-or->` 或等价的显式 option 传入：

```json
{
  "name": "demo",
  "owner": "did:bns:root",
  "kind": "static-web",
  "source": "./web/dist"
}
```

命令 option 至少提供 `--name`、`--owner`、`--kind`、`--source` 和可选 `--version`。同一字段同时
出现在 option 与 `--input` 时按主 PRD 返回参数冲突。`--non-interactive` 下任一必填输入缺失都必须立即
返回 `MISSING_REQUIRED_INPUT`，不读取 stdin 问答。若 Owner DID 不能按当前 BNS App namespace 规则推导 App DID，
机器可读 input 可以额外提供 `app_did`，交互模式则只在此情况追问一次。

`init` 的文件系统规则：

- `dapp_meta` 不存在时，先在工程目录内写临时目录，两个 JSON 都验证通过后再原子 rename；
- `dapp_meta` 已存在且为空时可以写入；任一目标文件或其它 entry 已存在时返回 `ALREADY_EXISTS`；
- 不提供覆盖用户文件的 `--force`，不合并或修补现有半个 `dapp_meta`；
- 失败时只删除本次创建的临时文件，不删除预先存在的工程目录或空 `dapp_meta`。

成功结果至少输出 `project_dir`、`meta_dir`、生成文件、App DID/name/version、subpackage key/kind/source
摘要、所有派生默认值及 `next_command="buckyos pikg build <meta-dir>"`。

### 6.2 `build`

`build` 必须：

1. 分别使用开发态 schema 验证 `app.json` 和 `pikg.json`，校验 App namespace 与 `sub_pkgs` key，
   并拒绝 `app.json` 中的 `pkg_list`/派生字段、普通 App 的第三方 `deps`、`source_url` 或其它
   非自包含输入；
2. 解析并固定所有 source，在打包期间检测可见的 TOCTOU 变化；
3. 为每个 subpackage 生成最终 `.tar.gz`、PackageMeta 及 Object ID；
4. 从 App 语义、subpackage 声明和构建结果生成完整 `pkg_list`，构造输出 AppDoc，再计算
   AppDoc Object ID；
5. 生成与所有 payload 字节一致的 `PACKAGE_META.json` 和 ownership manifest；
6. 对中间快照执行与 `pack` 相同的交叉引用和 digest 校验，成功后才替换输出目录。

`build` 的成功输出至少包含 `meta_dir`、`dist_dir`、App DID、AppDoc Object ID、subpackage 数量与
每项的 source kind/size/digest/pkg_objid，以及 `ready_for_pack=true`。绝对 source path 默认不在人类或机器
输出中展开；只输出相对显示或脱敏摘要。

### 6.3 `pack`

`pack` 只读取 `dapp_dist` 快照，不重新读取 `dapp_meta`、原始 subpackage 路径或 Docker tag。它必须：

1. 验证 ownership manifest、生成文件 allowlist 和所有快照 digest；
2. 验证 `APPDOC.json` → PackageMeta Object ID → payload digest 的完整引用链；
3. 将 `APPDOC.json`、`PACKAGE_META.json` 和所有 `<sub_pkg_name>.tar.gz` 写入临时 PIKG；
4. 使用与 Installer 同源的 PIKG reader/verifier 重新打开临时文件；
5. 校验回读的 AppDoc Object ID、PackageMeta Object ID、payload digest 与快照完全相同；
6. 原子替换 `dapp_dist/<pikg_file>`，并输出 `pikg_path`、size、`pikg_digest`、AppDoc Object ID 和
   `validation=passed`。

任何失败都不得覆盖上一个成功 PIKG。PIKG 整文件 digest 只用于 staging、缓存和审计，不替代内部
Object ID、payload digest 或 Owner/BNS 信任。

### 6.4 `info`

`info` 不是 ZIP 列表工具。它必须在输出成功 envelope 前完成：

- 容器 magic、entry 唯一性、名称/路径安全、数量/大小/解压限额和禁止文件类型校验；
- `APPDOC.json` / `APPDOC.jwt` 存在性、schema 和 canonical 一致性校验；
- AppDoc Object ID、所有 PackageMeta Object ID、namespace 和交叉引用校验；
- `PACKAGE_META.json.content_index` 与每个 payload 的 path、format、size 和 digest 校验；
- 普通 App 的自包含性和禁止第三方 package 依赖校验。

成功结果至少输出：

- `pikg_path`、size、`pikg_digest`、协议/schema 版本和 `valid=true`；
- App DID、name、version、owner、AppDoc Object ID；
- `app_doc_form=unsigned|signed|both` 和 `canonical_match`；
- 每个 subpackage 的 key、selector、required、pkg_id、pkg_objid、payload path/size/digest；
- `offline_content_validation=passed`；
- `signature_validation=passed|not-present|not-resolvable-offline`；
- `publication_validation=not-checked`。

`valid=true` 只表示容器、对象图和内容完整性通过，不表示已签名、已发布或当前受 BNS 信任。
`info` 默认不访问网络；如果签名 key 无法从包内已验证材料离线解析，必须如实返回
`not-resolvable-offline`，不得将“存在 JWT”输出为“签名有效”。
如果签名 key 可离线解析但密码学校验失败，命令必须以 `SIGNATURE_INVALID` 失败，不得返回
`valid=true`。

容器或内容校验失败时命令以 `INVALID_PACKAGE` 失败，错误 details 必须指出失败阶段和安全的
entry/object 标识，不能返回 `ok=true, valid=false` 让 Agent 忽略失败。

### 6.5 `clean`

`clean` 先读取 `dapp_meta/pikg.json` 解析 `output_dir`，再读取该目录的 ownership manifest。只有以下条件
全部成立时才可删除整个目录：

- 目录不是文件系统根、用户主目录、当前 workspace/project 根、`dapp_meta` 或任意 subpackage 输入目录；
- manifest schema 可识别，其 `meta_root_id` 与本次 canonical `dapp_meta` 的单向标识完全一致；
- 目录中除 manifest 允许的生成文件和预留 `pikg_file` 外没有其它 entry；
- 解析路径未经 symlink 指向其它目标；
- 已经按主 PRD 完成 destructive 确认，非交互模式下同时给出 `--yes`。

缺失或不匹配 ownership manifest 时必须返回 `UNSAFE_CLEAN_TARGET`，不提供跳过保护的 `--force`。
目录不存在时按幂等成功处理并返回 `removed=false`。删除包括该 `dapp_dist` 中的最终 PIKG。

## 7. 权限与安全

- 五个命令均为 `execution=local`，不解析 profile/Zone/identity，不登录，不申请网络权限。
- `init` 只读目标工程目录并写其 `dapp_meta`；只有 `kind=docker` 时才申请 Docker CLI 权限，
  且只执行 image inspect。
- `build` 只获得 `dapp_meta`、解析后的 subpackage source、`output_dir` 以及 Docker CLI 的权限；
  Docker 调用必须使用 argv API，不经 shell。
- `pack` 只读 `dapp_dist` 快照并写其 `pikg_file`；`info` 只读显式 PIKG；`clean` 只访问解析后的
  meta/dist 目标。
- 归档和 PIKG 写入必须防御绝对路径、`..`、重复 entry、symlink/hardlink 逃逸、zip bomb、
  声明 size 欺骗和特殊文件。
- 不读取 Owner/BNS 私钥、session token 或 BuckyOS security root；输出不包含 Docker credential、完整宿主机
  绝对路径或其它 secret。

## 8. 输出与失败语义

- 所有命令使用主 PRD 的 JSON envelope；进度和 Docker/归档诊断只写 stderr。
- 所有生成路径在 JSON 中返回规范化路径；来自 workspace 外的 source 默认脱敏。
- 参数或 manifest schema 错误返回退出码 `2`；Docker/文件 I/O 不可用返回领域错误；PIKG 验证失败
  返回 `INVALID_PACKAGE`；不安全 clean 目标返回 `UNSAFE_CLEAN_TARGET`。
- 本地操作不生成伪 Task ID。中断只清理本次命令的临时文件，不删除上一个成功快照或 PIKG。

## 9. 服务与实现映射

本模块不调用 BuckyOS service。PIKG parser、canonicalization、Object ID、PackageMeta 构造和 verifier
必须与 Publisher/Installer 复用同一个协议核心，不允许在 TypeScript CLI 中实现一套仅对自己产物
宽松通过的规则。

当前可复用实现基础：

- `src/frame/control_panel/src/pikg.rs` 中的 `PikgBuilder` / `PikgReader`、entry 限额和安全校验；
- `src/frame/control_panel/src/app_installer.rs` 中现有的 subpackage 归档、PackageMeta 构造和 AppDoc 填充逻辑；
- `src/kernel/buckyos-api/src/app_doc.rs` 中的 AppDoc/SubPkgDesc 类型；
- [App 安装协议](../../App%20安装协议.md) 中的 PIKG 结构、App namespace、Object ID 和安全规则。

现有 `app.publish` 从源目录打包到 repo 的复合流程只是迁移时的实现参考，不是新 CLI 的边界。
实现前需先把上述协议核心抽取为可被本地工具和 Installer 共享的组件；TS 访问该组件的具体
边界由实现设计确定，但不得分叉协议规则。

## 10. 与 App 和发布阶段的关系

- `pikg pack` 的结果可直接作为 `app install --pikg <path>` 或 `app upgrade ... --pikg <path>` 的输入。
- App 模块必须把已打包 PIKG 作为不可信输入重验，不调用 `pikg build/pack` 也不改写其内容。
- 本版不设计 `pikg sign/publish`、`app publish`、`repo publish` 或 BNS 发布命令。
- 后续发布应把“PIKG 上传/可下载”与“BNS AppDoc 权威更新”分属 Repo/Provider 和 BNS 权限边界。
- 在 `cyfs-dir-server` 上传 PIKG 的手册延后到正式 URL 结构和 AppDoc 到 PIKG 的映射确定之后。

## 11. 验收标准

- 在一个已存在的空工程中，`pikg init` 能用不超过 Owner DID、App kind 和 Source 三个必要回答
  生成 schema 有效的 `app.json` 和 `pikg.json`；工程目录名无法转换为合法 App name 时才多询问一项。
- `buckyos --non-interactive --input <file> pikg init` 产生与等价交互输入完全相同的两个 JSON；
  缺少必填字段时不等待问答。
- `init` 不询问 version、show name、author、permissions、output dir、PIKG 文件名、`pkg_id`、
  `pkg_objid` 或 digest，但会在成功结果中显示派生值。
- 目标 `dapp_meta` 非空时 `init` 不修改任何文件；Docker 初始化只执行本地 inspect，不执行
  save/pull/login/push。
- 无 BuckyOS、无 identity、无网络和无 Owner key 时，path 型 subpackage 可完成 build、pack 和 info。
- 已存在的本地 Docker image 可通过 image name 构造 PIKG，过程不产生任何 registry 网络请求。
- 相对 source path 以 `dapp_meta` 为基准，绝对 source path 得到同样的输入类型和安全校验。
- `build` 产生可审计的 `dapp_dist`；`pack` 不回读或重新构建任何 source。
- `pack` 的成功产物可被同一 verifier 重新打开，AppDoc Object ID、PackageMeta Object ID 和 payload digest
  与 `dapp_dist` 完全一致。
- 任意 payload 或 metadata 单字节篡改会使 `pack` 或 `info` 以 `INVALID_PACKAGE` 失败。
- `info` 清晰区分内容有效、签名可验和 BNS 当前已发布，不把三者压缩成一个模糊状态。
- `clean` 只删除归属匹配的受管理 `dapp_dist`；针对根目录、工程根、源目录、symlink 目标和
  无 ownership manifest 目录的删除测试全部失败。
- 文档和 `command describe pikg <verb>` 产生的 schema/帮助示例一致。

## 12. 待决策项

- Rust PIKG 协议核心向 TS/Deno 工具暴露的具体形式（子进程边界、FFI 或其它共享组件）。
- 目录归档的可重现 metadata 精确值（mtime、uid/gid、mode 和 gzip header）在实现 Spec 中固定。
- Docker image ID、RepoDigest 和加载后 runtime digest 的精确对应规则需与 App Loader 完成协议联调。
