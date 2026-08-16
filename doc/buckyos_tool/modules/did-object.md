# DID Object 模块需求

> 状态：Draft  
> 对应 modules：`did`、`did-object`

## 1. 目标与边界

提供可信 DID 解析、文档验证以及 DID Object Protocol 的读取和动作调用。该模块不迁移旧
`buckycli did` 中本地生成文件、读取 start config、打印私钥等历史实现。

NamedObject/ObjId 的内容持有和导出属于 [Object 模块](object.md)。密钥生成、托管、轮换和
签名是独立安全领域，不作为普通 DID 查询命令的附带能力。

## 2. 资源模型

- DID 与经过验证的 DID Document；
- resolver status、authority、document version/hash；
- DID Object Card/Profile；
- object URL、read capability、action schema 和 event capability。

BNS 提供可信解析起点，DID Object Protocol 定义对象可读取和可调用的能力；二者不能简化为
任意 URL 请求。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `did resolve <did>` | read | 返回验证状态、来源和文档摘要 |
| `did get <did>` | read | 输出权限允许的完整 DID Document |
| `did verify` | read | 验证显式文档、owner、version 和签名 |
| `did-object describe <url-or-did>` | read | 获取 Profile、属性和 action schema |
| `did-object read <object-url>` | read | 读取对象属性或内容引用 |
| `did-object action <object-url>` | write | 调用显式 action + JSON params |

## 4. 安全与实现基础

- resolve 输出必须区分 Verified、NeedProof、Missing、Revoked、Tombstoned 和 Unknown。
- action 先验证对象身份和 capability，再执行；不能把裸 HTTP 200 当可信对象结果。
- 签名材料不得通过 stdout、verbose 和错误详情输出。
- Agent Tool 已有 `read/x-call` 和 route config 抽象，可复用其 runtime/library；BuckyOS Tool
  必须使用生产 route、身份和审计，不复用 dev 默认 file/http fallback。

## 5. 待决策项

- DID Object 的生产 route/config 是否属于 `~/.buckyos_tool` profile，或由 BNS/Profile 动态发现。
- action 的异步 Task、支付和用户确认如何映射到统一 CommandResult。
- 独立 key/sign 模块是否属于 BuckyOS Tool 正式运维范围。
