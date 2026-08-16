# Object 模块需求

> 状态：Draft  
> 对应 module：`object`

## 1. 目标与边界

管理 NamedData、NamedObject 和 RepoService 中内容的本地导入、读取、验证、持有与导出。

本模块不负责：

- 可变文件系统目录操作，见 [Files 模块](files.md)；
- 人类可读名字、分享和版本发布，见 [Content 模块](content.md)；
- 运行目录发现和 mount，见 [Storage 模块](storage.md)；
- 数据库 connection string，见 [App 模块](app.md)；
- 系统一致性备份，见 [Backup 模块](backup.md)。

## 2. 资源模型

- ObjId / ChunkId / FileObject；
- NamedStore location；
- Repo record：collect、pin/store、proof、serve、announce；
- availability：`local-only`、`zone-available`、`replicated`、`public`。

`local-link` 只是节点本地引用，不承诺远程可用、备份持久性或内容复制。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `object ingest <path>` | write | task/either | `--mode <store-or-local-link>`，返回 ObjId |
| `object stat <obj-id>` | read | sync | 返回类型、大小、location 和 availability |
| `object inspect <obj-id>` | read | sync | 解析已知 NamedObject 元数据 |
| `object verify <obj-id>` | read | sync | 校验 hash、结构和可选签名 |
| `object read <obj-id>` | read | sync | range/read stream，不默认落盘 |
| `object export <obj-id>` | write | task/either | materialize 到显式输出路径 |
| `object list` | read | sync | 分页列出 Repo record |
| `object pin <obj-id>` | write | task/either | 保证本节点持有内容 |
| `object unpin <obj-id>` | write | sync | 解除 pin，不等同立即删除 bytes |
| `object collect <obj-id>` | write | sync | 只收录 metadata |
| `object uncollect <obj-id>` | destructive | sync/task | 移除 Repo record |
| `object resolve <name>` | read | sync | 使用 Repo 的内容名/proof 解析 |
| `object announce <obj-id>` | write | sync | 宣告可服务能力 |

## 4. 输出与安全

- `inspect` 只对已知结构输出 JSON；未知对象输出 type/metadata，不伪造 JSON。
- `read --output raw` 和 `export --output-file` 遵循主 PRD 的 stdout 规则。
- 输入本机 path 会触发显式 Deno file permission；远程 Agent 不应默认获得任意 Host path。
- pin/unpin/uncollect 必须输出对 GC 与可用性的实际影响。

## 5. 实现基础

RepoService 已有 store/collect/pin/unpin/uncollect/proof/resolve/list/stat/serve/announce。NamedStore
已有 store 与 LocalLink 模式。需要新增的是稳定的 TS facade、文件 ingest/export 和统一 ResourceRef。
