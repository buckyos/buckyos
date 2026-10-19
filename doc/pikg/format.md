# PIKG 容器与内容图格式

> 当前权威：PIKG 的容器、PackageMeta/content index、内容 identity 和 reader 校验层次。开发态输入只在 WebSDK PIKG 模块维护；AppDoc schema、签名协议与安装事务只在 App 安装协议维护。

本文依据 [TypeScript protocol](https://github.com/buckyos/buckyos-websdk/blob/main/cli/modules/pikg_protocol.ts)、[archive](https://github.com/buckyos/buckyos-websdk/blob/main/cli/modules/pikg_archive.ts) 和 [Rust reader](../../src/frame/control_panel/src/pikg.rs) 整理。两端独立实现，不能把同一协议误称为同一份 verifier。本文只记录当前格式，不为未实现的发布流程添加 wire 字段。

## 1. 容器与 entry

PIKG 扩展名为 `.pikg`，实际为 ZIP/ZIP64；reader 检查 ZIP magic、中央目录、entry 唯一性和边界，不按扩展名判定有效性。

| entry | 当前规则 |
| --- | --- |
| `APPDOC.json` | 未签名 AppDoc candidate body |
| `APPDOC.jwt` | reader 支持的 JWT 形式，解码 claims 为 AppDoc；不是 detached signature envelope |
| `PACKAGE_META.json` | 必须存在，固定 `@schema=buckyos.pikg.package-meta.v1` |
| `<sub_pkg_name>.tar.gz` | CLI 完整包的子包 payload；content index 指向这些 entry |
| `objects/<objid>.json` | reader 支持的附加对象，Object ID 必须与 JSON 内容相符 |

AppDoc 两种 entry 至少存在一种，同时存在时 canonical body 必须一致。旧 `APPDOC.wt` 不支持。CLI pack 当前生成 JSON candidate、PackageMeta 和完整 payload，不生成 JWT 签名。

entry 名使用 UTF-8 与 `/`，拒绝空名、绝对路径、`..`、反斜杠、NUL、重复 entry、链接/特殊文件以及损坏或重叠的 ZIP 结构。payload 内 tar 的安全解包由消费方继续校验，外层 ZIP 通过不表示任意 tar 路径都可落盘。

| 限制 | 当前值 |
| --- | --- |
| entry 数量 | 4096 |
| 单个 AppDoc entry | 1 MiB |
| 单个其他 metadata entry | 8 MiB |
| metadata 总量 | 64 MiB |

这些是 metadata/结构限制，不能写成整包或 payload 的 32 MiB 上限；staging 的单块传输限制和配额属于安装协议。

## 2. `PACKAGE_META.json`

根对象只包含下列字段：

| 字段 | 内容 |
| --- | --- |
| `@schema` | `buckyos.pikg.package-meta.v1` |
| `app_doc_id` | 包内 AppDoc 的 canonical `appdoc:<64hex>` Object ID |
| `package_objects` | `pkg:<64hex>` → PackageMeta JSON body |
| `content_index` | `sha256:<hex>` → content index entry |

content index entry 的字段为 `sub_pkg_name/path/format/size/digest`。CLI 完整包要求 `path=<sub_pkg_name>.tar.gz`、`format=tar.gz`；`size` 与 SHA-256 针对 ZIP entry 解压后得到的整个 `.tar.gz` 文件，不能改为其解开后的目录摘要。

## 3. 内容引用与 identity

```text
AppDoc.pkg_list[key].pkg_objid
  → package_objects[pkg_objid]
  → PackageMeta.content
  → content_index 中绑定的 payload
  → 真实 .tar.gz 字节
```

- `appdoc:` 和 `pkg:` Object ID 使用 Named Object 的规范 JSON/JCS 算法；详细 AppDoc default/omit 规则和黄金值见 [App 安装协议](../App%20安装协议.md)。
- App namespace 为 AppDID 派生的 canonical AppId。PackageId 的 qualifier、namespace、版本和 exact identity 规则只在安装协议 §3 维护，展示名不参与 namespace。
- `package_objects` 的 key 必须等于 body 重算的 Object ID；PackageMeta 的 name/version 必须与 AppDoc 的 PackageId 一致。
- CLI 完整包还检查 owner/author 一致、自包含性、无非空第三方 deps、无 `source_url`、一子包一 payload，以及没有未引用 PackageMeta 或多余 content index 项。
- PackageMeta.size/content 与 index 以及真实 payload 必须一致；Docker runtime image digest 独立于 payload digest。

整文件 `pikg_digest=SHA256(.pikg 文件字节)` 用于 staging 固定、缓存和审计；它不能代替 AppDoc/PackageMeta Object ID、payload digest 或发布权威。仅重封装 ZIP 也可能改变整包 digest。

`.buckyos-pikg-dist.json` 是本地快照所有权清单，不是包 entry、App 身份或发布证明，其 schema 由 [PIKG CLI 文档](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) 维护。

## 4. 三种不同的校验

| 层 | 当前责任与结果 |
| --- | --- |
| 结构与内容 | CLI `info` / Rust `PikgReader` 核对容器、对象与字节；包内 AppDoc 仍为 candidate |
| 签名与身份权威 | Resolve/Verify 负责；有 `APPDOC.jwt` 不等于密码学验签或 Owner 信任已成立 |
| 当前发布状态 | BNS/resolver 权威链与版本状态；Repo 存在内容或本地 info 成功均不能代替 |

当前 TypeScript `inspectPikg` 有 JWT 时只解码 claims 并验证 canonical body，固定返回 `signature_validation=not-resolvable-offline`；无 JWT 为 `not-present`，`publication_validation` 均为 `not-checked`。`app_doc_form=signed` 只标识 entry 形式。

安装协议 §2.3 定义 detached Ed25519 signature envelope；它并不是 reader 已接收的 `APPDOC.jwt` entry 格式。签名发布闭环尚未完成，不能按旧设计稿自行构造另一套正式签名包。历史的签名、托管和发布顺序建议见 [历史设计](history/app-config-and-publication.md)。

## 5. 消费方差异

| 消费方 | 当前公开输入与边界 |
| --- | --- |
| CLI build/pack/info | 完整、自包含 PIKG；普通 App、Agent 模板和独立 `agent_skills` 包 |
| Rust 普通 App Installer | 受控 immutable staging 文件，重新验证普通 App/Agent 模板；上层执行信任、目标和权限校验 |
| Agent skill CLI | TypeScript 验证后，仅将 `agent_skills` 中认可目录的文件提交到 Agent `skills.install` |

独立 `app_type=agent_skills` 是 TypeScript/Agent 路径支持的包类型，不是 Rust `AppType` 的新增枚举值。Agent 模板的 `app_type=agent` 与之不同，公共 App submit 为模板注册。内部 Rust builder 的部分包用例不代表 CLI 提供部分打包或普通 App 第三方依赖安装能力。

## 6. 回归入口

- [PIKG CLI tests](https://github.com/buckyos/buckyos-websdk/blob/main/cli/tests/pikg_test.ts)：离线闭环、篡改、Docker image 固定与命令 schema。
- [Rust `pikg.rs`](../../src/frame/control_panel/src/pikg.rs) 的 `pikg::tests`：容器拒绝、对象/hash/digest、staging 和解包安全。
- [AppDoc golden fixtures](../fixtures/) 及 `buckyos-api` / TypeScript 的 golden identity tests：canonical identity。
- [Installer DV](../../test/app_installer_test/README.md)：真实安装与运行证据，不能用离线 info 代替。
