# Files 模块需求

> 状态：Draft  
> 对应 module：`files`

## 1. 目标与边界

提供面向用户的可变层级文件系统 API，支持 DFS 逻辑视图以及被系统显式暴露的设备文件视图。
内部 API 不由 WebDAV 协议反向定义；WebDAV 可以作为后续 adapter。

NamedObject 内容操作属于 [Object 模块](object.md)，外部目录 mount 属于
[Storage 模块](storage.md)。

## 2. 资源模型

- canonical URL：例如 `dfs://`，后续可扩展 `device://`；
- folder、file、reference；
- revision/etag、ACL 和 capabilities；
- Folder/View/Collection 三种 location，View 只读、Collection 管引用而非数据本体。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `files list <url>` | read | 服务端分页、排序和 filter |
| `files stat <url>` | read | 获取 metadata、capabilities 和 revision |
| `files read <url>` | read | range read，支持 raw/output-file |
| `files write <url>` | write | 从 input/file/stdin 写入，使用 expected revision |
| `files mkdir <url>` | write | 创建目录 |
| `files copy <url>` | write | 复制到明确 target |
| `files move <url>` | write | 移动或重命名 |
| `files delete <url>` | destructive | 删除实体或引用，语义由 location kind 决定 |
| `files search` | read | 服务端搜索，结果保持真实路径 |
| `files watch <url>` | read | jsonl 输出变化事件 |
| `files acl-get <url>` | read | 获取资源 ACL |
| `files acl-set <url>` | write | revision/CAS 修改 ACL |

## 4. 当前状态与验收

File Browser 已有 UI Reader 和 Folder/View/Collection 数据抽象，但真实 DFS backend 和正式 RPC
仍未完成。Files CLI 实现必须等待稳定 backend，不得把 UI mock 当生产数据源。

- 所有写操作检查 location capabilities。
- Collection delete 只移除引用，Folder delete 才可能销毁数据。
- 设备/裸盘视图必须由节点显式暴露，不能让 CLI 任意浏览 Host 文件系统。
- 大目录必须分页，禁止 CLI 全量加载后排序。
