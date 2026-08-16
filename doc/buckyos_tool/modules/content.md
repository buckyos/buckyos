# Content 模块需求

> 状态：Draft  
> 对应 module：`content`

## 1. 目标与边界

提供内容“分享”和“发布”能力：

- Share：对仍会变化的文件、目录或资源授予访问权。
- Publish：把 ObjId 映射到稳定名字，管理版本、上下架和访问策略。
- Repo 持有与传输由 [Object 模块](object.md)负责。

是否有 OOD 不改变命令协议；区别体现在 availability 和后台存储策略，不在 CLI 中分叉成两套
命令。`cyfs_obj_dir` 可以作为 ingest 输入适配器，但不是生产发布协议。

## 2. 资源模型

- Published item：`name -> current_obj_id`；
- revision/sequence 和 CAS；
- public/token-required/encrypted 等策略；
- enabled/disabled 和不可变历史；
- Share grant/capability、subject、expiry 和 revocation。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `content publish` | write | 发布或更新 name -> ObjId，要求 expected sequence |
| `content resolve <name>` | read | 解析当前或指定 revision |
| `content get <name>` | read | 获取策略、head 和 availability |
| `content list` | read | prefix + 分页列出发布项 |
| `content history <name>` | read | 列出不可变 revision |
| `content enable <name>` | write | 上架并产生新 revision |
| `content disable <name>` | write | 下架、记录 reason 并产生 revision |
| `content share-create` | write | 给资源创建 ACL/capability |
| `content share-list` | read | 查询自己的分享 |
| `content share-update` | write | 修改 expiry/policy/subject |
| `content share-revoke` | destructive | 撤销分享，不删除原始内容 |

## 4. 实现状态

ShareContentMgr 已实现 publish、resolve、resolve-version、get、enable/disable、list、history、
统计和日志。Live file/directory share 的统一资源 URI、ACL 和 capability 仍需服务端设计，CLI
不得先用本地 path 约定代替协议。

## 5. 验收重点

- 发布前能检查 ObjId availability，local-only 必须产生明确 warning。
- CAS 冲突输出当前 sequence，不自动覆盖。
- revoke/disable 与删除对象严格区分。
- share subject 不能直接信任用户可编辑的联系人组名。
