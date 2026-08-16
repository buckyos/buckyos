# Contact 模块需求

> 状态：Draft  
> 对应 module：`contact`

## 1. 目标与边界

管理当前用户的外部联系人、外部 endpoint binding、联系人关系和 MessageHub 入站准入策略。

联系人组和 tag 用于通讯录组织，不能作为 BuckyOS 系统 RBAC、App availability 或管理员角色
来源。ContactMgr 不负责替出站消息自动选择 tunnel；发送目标由 [Message 模块](message.md)
显式指定。

## 2. 资源模型

- canonical contact DID；
- external endpoint DID；
- binding、alias 和 merge tombstone；
- access level：Block/Stranger/Temporary/Friend；
- groups/tags 以及带 expiry 的 temporary grant。

外部 endpoint 再次发送消息时可能重建 shadow contact，因此“删除”应拆成 forget/archive/block。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `contact list` | read | 分页、过滤联系人 |
| `contact get <did>` | read | 获取 canonical contact 与 bindings |
| `contact create` | write | 手工创建联系人 |
| `contact import` | write | 批量导入并返回逐项结果 |
| `contact update <did>` | write | 修改名称、备注、groups、tags、关系 |
| `contact merge <target-did>` | write | 将 source 合入 target，保留 alias |
| `contact forget <did>` | destructive | 删除本地联系人投影，不等同 block |
| `contact archive <did>` | write | 从常用列表隐藏但保留历史 |
| `contact block <did>` | write | 阻止入站消息 |
| `contact unblock <did>` | write | 恢复为指定关系级别 |
| `contact grant-temporary <did>` | write | 按 context + expiry 临时准入 |
| `contact endpoint-list <did>` | read | 列出可显式选择的外部 endpoint |

## 4. 实现基础与验收

当前 ContactMgr 已有 resolve、reverse lookup、alias、import、merge、update、get/list、block 和
temporary grant。缺失或不明确的 forget/archive/unblock 语义需要先在服务端协议化。

- 所有数据按当前用户隔离。
- list/import 支持分页或批次结果。
- merge 输出 canonical DID 和保留的 alias。
- 任何命令都不能把“preferred binding”作为出站自动路由承诺。
