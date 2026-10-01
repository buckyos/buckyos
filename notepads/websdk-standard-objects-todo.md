# WebSDK 标准对象实现收敛 TODO

> 背景：2026-09-30 实现 MsgObject v2（cymsg breaking change）时发现，TypeScript 侧没有一份统一的标准对象实现。Desktop 自己手写了一份 `msgobj.ts` 镜像，WebSDK 中的 `MsgObject` 只是 `Record<string, unknown>`，v2 字段只能改在 Desktop 里。
>
> 目标：CYFS 标准对象的 TypeScript 实现统一放在 buckyos-websdk，前端应用只从 `buckyos` 包引用，不再各自维护协议镜像。
>
> 协议权威：cyfs-ndn《CYFS 标准对象》（`cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md`），参考实现为 `cyfs-ndn/src/ndn-lib`。

## 1. 现状

| 对象 / 能力 | Rust ndn-lib | WebSDK `src/ndn_types.ts` | 其它位置 |
| --- | --- | --- | --- |
| ObjId / ChunkId、canonical JSON、ObjId 计算 | 有 | 有 | — |
| `cyfile` FileObject、`cypath` PathObject、`cyinc` InclusionProof、`cyrel` RelationObject、`cydir` DirObject、`cymap` SimpleObjectMap、`clist` | 有 | 有 | — |
| `cymsg` MsgObject（v2） | `msgobj.rs` | 只有常量 `OBJ_TYPE_MSG`；`msg_center_client.ts` 中 `MsgObject = JsonObject` | Desktop `src/frame/desktop/src/app/messagehub/protocol/msgobj.ts`（手写镜像，约 20 个文件引用） |
| `cyrece` ReceiptObj | `msgobj.rs` | 没有；常量 `OBJ_TYPE_MSG_RECE = 'cymsgr'` 已过时 | — |
| `cyact` ActionObject | `action_obj.rs` | 只有常量 | — |
| `pkg` PackageMeta | `package-lib` | 只有常量 | — |
| JWT 形式的 NamedObject（§5.2） | `build_named_object_by_jwt`、`named_obj_to_jwt` 等 | `loadNamedObjectFromObjStr` 明确不支持 JWT | — |
| MsgObject 校验与 JWT（§16.5） | `MsgObject::validate`、`from_json_value_checked`、`verify_msg_object_jwt` | 没有 | Desktop 有 `isValidMsgSessionId`、`randomMsgNonce` |
| dispatch 编码（JSON / JWT Content-Type） | `CyfsNamedObjectEncoding`、`validate_cyfs_dispatch_body` | 没有 | — |

其它问题：

- WebSDK 的 `tests/ndn_object.json` 是 Rust 回环测试的输出快照，其中 `cymsg` 已更新为 v2，`cymsgr` 一项仍是旧对象。生成方式：在 cyfs-ndn 运行 `cargo test -p ndn-lib --lib test_non_chunk_named_object_file_roundtrip -- --nocapture`，取打印的 `[{objid, objjson}]` 数组。
- Desktop `msgobj.ts` 文件头仍指向旧的 Mac 路径 `/Users/liuzhicong/project/cyfs-ndn/...`。
- Desktop 依赖 npm 上的 `buckyos@0.7.116`（pnpm 解析结果），本地 WebSDK 已是 0.7.125。WebSDK 改动需要先发版，Desktop 升级依赖后才能使用。

## 2. 待办

### 2.1 WebSDK

- [ ] 新增 MsgObject v2 类型并从包入口导出：`MsgObject`、`MsgObjKind`、`TopicThread`、`MsgRelation`、`MsgRelType`（保留未知值）、`MsgMentions`、`MsgContent`、`MsgContentFormat`、`MachineContent`、`CanonValue`、`RefItem`、`RefTarget`、`RefRole`。`meta` 是 flatten 字段，类型保留索引签名。
- [ ] 移植校验：`validateMsgObject`（对应 `MsgObject::validate`：`to_session` 取值与单目标、`mentions` 不能为空对象、`relates_to` 目标类型与 `key` 规则、`meta` 保留键含 `proof`）、`isValidMsgSessionId`、`MSG_OBJECT_RESERVED_KEYS`、`randomMsgNonce`（2^53 以内）。
- [ ] `msg_center_client.ts` 的 `MsgObject` 改为上述类型，不再是 `JsonObject`。
- [ ] 新增 `ReceiptObj`，把 `OBJ_TYPE_MSG_RECE` 改为 `cyrece`，并重新生成 `tests/ndn_object.json`。
- [ ] 新增 `ActionObject`、`PackageMeta`，与 Rust serde 形态一致。
- [ ] 支持 JWT 形式：`loadNamedObjectFromObjStr` 能解出 claims，按 claims 计算 ObjId；提供 MsgObject JWT 的解码（取 `kid`、claims、ObjId）。浏览器侧验签（WebCrypto Ed25519）按需求再定。
- [ ] 导出 dispatch 的两个 Content-Type 常量及编码判断。

### 2.2 Desktop

- [ ] WebSDK 发版后升级 `buckyos` 依赖。
- [ ] `messagehub/protocol/msgobj.ts` 只保留 UI 自己的部分：`MessageObject` 上的 `ui_*` 字段（`ui_message_id`、`ui_session_id`、`ui_delivery_status`、`ui_sender_name`、`ui_status_type`、`ui_item_kind` 等）及其读取函数，协议类型全部改从 `buckyos` 导入。
- [ ] 检查其它前端（filebrowser、control-panel 等）是否也有自写的标准对象类型，一并改为引用 WebSDK。

## 3. 约束

- Rust ndn-lib 是参考实现。修改《CYFS 标准对象》时，同一次改动同步更新 WebSDK 的类型、校验与 fixture。
- canonical JSON 必须与 Rust `serde_jcs` 一致：空的 `Option`、空 `mentions`、`false` 的 `all` 都省略，否则 ObjId 不一致。
- 反序列化保持宽松，校验在入口显式调用，与 Rust 的做法一致，保证旧数据能读出。

## 4. 验收

- Desktop 中不再有协议对象的类型定义。
- WebSDK 的 fixture 覆盖所有非 Chunk 标准对象（含 `cyrece`），jest 中按 fixture 重算 ObjId 全部通过。
- MsgObject 校验的测试用例与 `msgobj.rs` 中 `test_msg_v2_*` 一一对应，结果一致。
