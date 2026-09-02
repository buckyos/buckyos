# AICC 模块所有权与评审清单

GitHub CODEOWNERS 账号：`@streetycat`。

| 路径 | Owner 角色 | 必需评审角色 |
| --- | --- | --- |
| `Cargo.toml`、`src/lib.rs`、`src/main.rs`、`README.md`、`OWNERS.md` | 集成人 | 集成人 |
| `src/api`、`src/error` | API 小组 | API owner、集成人 |
| `src/matching` | Catalog/Matching 小组 | Catalog/Matching owner |
| `src/catalog` | Metadata 小组 | Metadata owner、Catalog/Matching owner |
| `src/model`、`src/routing` | Model/Router 小组 | Model/Router owner |
| `src/protocol` | Protocol Infra/Codec 小组 | Protocol owner |
| `src/provider` | Provider Runtime 小组 | Provider Runtime owner、Protocol owner |
| `src/call` | Protocol/Router 联合小组 | Protocol owner、Model/Router owner |
| `src/execution` | Execution 小组 | Execution owner、Protocol owner |
| `src/resource` | Resource/Security 小组 | Resource/Security owner |
| `src/storage`、`src/observability` | Storage/Observability 小组 | Storage/Observability owner |
| `src/runtime`、`src/settings` | Runtime/Consistency 小组 | Runtime/Consistency owner |
| `src/service` | Service Integration 小组 | Service Integration owner、集成人 |

评审时必须确认：

- 修改仅发生在 owner 路径或已获得对应 owner 联合评审；
- 新增跨模块 contract 前已经由对应工作包确认必要性和 owner；
- Provider、模型名、URL 和模型前缀没有进入协议或路由特殊分支；
- contract 变更同步增加或更新对应模块的 fixture、fake 和单元测试；
- 没有恢复旧 AICC 文件、模块名或 settings/metadata 结构；
- `Cargo.toml`、`lib.rs` 和 `main.rs` 只由集成人合并。
