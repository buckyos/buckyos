# 压缩包浏览与 Preview 支持 TODO

状态：已实施（2026-11-16），T8 的真实 Zone / 真机 / 基准未做，见 §8。方案定稿日期：2026-11-16。源码基线：buckyos `f3d914288`、buckyos-websdk `39f2127`、cyfs-ndn `e36948c`，下文的行号以这些提交为准。

2026-11-16 的定稿过程：

- owner 明确 Preview 内部复用 Files 的文件列表显示（D1）。
- owner 同意按设计 review 的建议定稿（D2–D16）。其中 D6、D7 是 review 在两个候选里替 owner 选的，owner 可以推翻；推翻时只改对应任务，不影响其他决定。
- owner 补充：**这个需求的来源是在 MessageHub 里预览聊天收到的 zip**，Files 和 Zone 里的文件是顺带的扩展。所以对象来源（聊天附件）这个版本必须做，而且是首要场景（D8）。
- owner 要求文件名做传统的 CJK 编码自动探测（D11，算法见 §2.7），并确认引入新依赖 `chardetng`（见 §1 表后）。

**目标**：在 MessageHub 里点开聊天收到的 zip，在 Preview 里浏览目录、原地预览包里的文件；Zone 里的 zip 在 Files 和 Preview 里同样能浏览。每个包内文件都有一个可以复制、传递、重新打开的地址。密码验证成功后，按“用户 + 压缩包内容”记住密码。不默认解压整包。

**给实施 agent 的阅读顺序**：先读 §1 决定和 §2 协议，这两节是冻结的；然后按 §4 的任务顺序实施，§3 是接入点。遇到这里没写的边界情况，按 §1 的原则取最简单、能如实报错的做法，并在 §8 实施记录里写下来，不要自己扩大范围。

**术语**

- **外层文件**：压缩包本身。**成员**：包里的文件或目录。
- **对象来源**：只有对象 ID、没有路径的文件对象，主要是聊天附件。
- **路径来源**：Zone 里有路径的文件，包括 DFS 原生文件和 bucket projection 文件。

## 1. 已定决定

| # | 决定 | 来源 / 理由 |
| --- | --- | --- |
| D1 | Preview 复用 Files 的文件列表显示与交互，落实到同一份组件（`MainContent` 及其展示层），不另写相似 UI。复用的是列表显示和交互；Files 的窗口、侧栏、tab、上传仍由 Files 自己管理 | owner |
| D2 | **成员地址**<br>- 对象来源：`ObjectRef { obj_id: <外层 FileObject>, inner_path: "/archive/<成员路径>" }`<br>- 路径来源：`<外层文件路径>/@/archive/<成员路径>`。判定规则是：只有紧跟在一个普通文件后面的 `@` 段才表示“进入压缩包”。普通文件不可能有子项，所以真实名叫 `@` 的目录不受影响，`%40` 被不被归一化成 `@` 也无所谓<br><br>两种来源用同一个 `archive` 分支，`content` 等其他分支名保留给 cyfs-ndn 的 FileObject 语义。不另设“友好路径” | review：手机用户看的是面包屑、不看原始路径，路径长一点没有体验代价 |
| D3 | **引用**<br>- 对象来源：成员引用是 `object-id` 引用加 `innerPath`。外层对象不会变，所以这就是固定引用，不需要 `snapshot`。要给 Preview 的 `ObjectIdRef` 和 SDK 的 `TransferableContentRef`（object-id 分支）加 `innerPath`<br>- 路径来源：成员引用是普通的 `cyfs-path` 引用加现有的 `version` 字段，是 live 引用，外层变了就 `STALE`。路径来源的固定引用要先把外层对象化，依赖 [文件对象 ID TODO](nfs-server-file-objid-todo.md) 的 `snapshot`，本版不做 | review、owner（对象来源本来就有不变的 ID） |
| D4 | 路径来源的外层版本 = `(ino, dev, birth, size, mtime)`，编码进成员的 node_id；解析时与当前 `lstat` 比对，不一致返回 `STALE`。mtime 只到秒，原地改写同大小文件、且不换 inode 的情况识别不到，记为已知限制（§7）。对象来源不会变，没有 `STALE` | review：“写临时文件再 rename 覆盖”会换 inode，最常见的覆盖能识别 |
| D5 | 压缩方法 = 当前 `zip` crate（锁文件 2.6.1，默认特性）能解的全部方法：Stored、Deflate、Deflate64、BZip2、LZMA、Zstd、XZ；加密支持 ZipCrypto 和 AES。不加新依赖。以样本实测结果为准，写进 `hello` | 锁文件已带 `aes-crypto`、`deflate64`、`bzip2`、`lzma`、`zstd`、`xz` |
| D6 | 记住密码：验证成功后自动记住，存在 nfs_server 自己的 SQLite 里（服务私有目录，靠文件权限保护），**不新建密钥设施**；明文落盘是已知风险（§7）。另提供单包“忘记密码”和“清除所有记住的密码” | review 替 owner 选（先能力再安全）；分享密码只存了 hash（`share/db.rs:67`），不能拿来解密，系统里也没有可逆的秘密存储 |
| D7 | 系统文件选择器（`sysdlg/select`）本版把压缩包当普通文件：可以选整个包，不能进包选成员 | review 替 owner 选；路径来源的成员只有 live 引用，不适合交给调用方长期保存 |
| D8 | **本版做两类来源**<br>- 对象来源，首要场景：本 Zone NDM 里调用方可读的文件对象，主要是聊天附件<br>- 路径来源：DFS 原生文件（home、srv 等，不含 host realm），以及 bucket projection 文件<br><br>对象内容不在本地时，沿用现有读取的行为；读取失败如实报错 | owner：需求来自 MessageHub |
| D9 | **成员读取策略**<br>- Stored 且未加密：直接映射到外层文件的字节区间，支持任意 Range<br>- 其他成员，没有 Range 或 Range 从 0 开始：流式解压<br>- 其他成员，Range 起点不为 0：先把这个成员完整解压到缓存文件，再走现有的文件数据面响应 | review：PDF 读尾部、视频拖动都能用；206 / 416 语义和内容安全策略直接复用现成代码 |
| D10 | 标准 ZIP 的中央目录是明文（ZipCrypto 和 AES 都只加密成员数据），所以列目录永远不需要密码，只有读加密成员时才要 | ZIP 规范；PKWARE 的中央目录加密不支持 |
| D11 | 文件名编码自动探测：没有 UTF-8 标志的文件名，以整个压缩包为单位，在 GBK、Big5、CP932、EUC-KR、EUC-JP 里探测并转成 UTF-8，探测不出来时用 CP437。名字很短、判断不可靠时，参考请求方的语言偏好。算法、阈值和实验数据见 §2.7 | owner；`zip` crate 没有标志时只会按 CP437 解码，中日韩 Windows 打的包会乱码 |
| D12 | **不可寻址的成员**不出现在列表里，也读不到；数量放在压缩包根 `stat` 的 `archive.hidden_entries` 里，界面显示一行提示。包括：<br>- 解码并处理完分隔符后（§2.7）是绝对路径、含 `.` 或 `..` 段、有段过不了 `validate_name` 的<br>- 同时含 `/` 和 `\` 的<br>- 符号链接条目<br>- 规范化后同名的：两项都隐藏，按路径读返回 `AMBIGUOUS_ENTRY`<br><br>成员路径区分大小写；只在成员路径里出现的父目录自动补成目录 | 不能默默选第一项或最后一项 |
| D13 | Files 设置项“将压缩包作为目录浏览”默认开启，和图标大小一样存 localStorage（`data/iconSize.ts` 的做法），入口放在图标大小所在的视图菜单。开启时打开 `.zip`（扩展名不区分大小写，且 `hello` 声明支持 zip）会进入包内；关闭时按现有内容打开规则交给 Preview。DOCX / XLSX / EPUB 等扩展名不是 `.zip`，不受影响 | review |
| D14 | 包内只读：不能写入、删除、改名、上传、改权限、分享。主要导出方式是把成员复制到 Zone 里可写的目录，两种来源都支持，复用现有的 copy 任务（`copy.rs` 的 `copy_submit`）；“下载到本机”是次要入口 | 产品原则：目标用户主要用手机，Zone 是主文件系统 |
| D15 | 本版不做：嵌套包、分卷包、7z / RAR / tar / tar.gz、批量“解压到”、host realm 归档、把包内成员转发到别的会话（要先把成员做成独立对象）、cyfs-ndn 的 `/ndn/<obj_id>/@/archive/...` 直接读、路径来源的固定引用。遇到时如实返回 `UNSUPPORTED`，现有能力不变 | 本版范围 |
| D16 | 对象来源的外层先整份落到本地缓存文件（复用 preview 的 `materialize`），然后与路径来源共用同一个 provider；不先写基于分块读取的 `Read + Seek`。落盘时顺带算整文件 sha256，供密码记忆使用，这样同一个 zip 在聊天里和存进 Zone 后对应同一个密码键 | review：组合优于发明；大附件的落盘成本见 §7 |

**新增依赖（owner 2026-11-16 已确认）**：D11 给 nfs_server 新增直接依赖 `chardetng = "1.0.0"`。

- 它是 Firefox 用的编码探测器，作者与 encoding_rs 相同，许可证 Apache-2.0 OR MIT。它的依赖 encoding_rs、memchr、cfg-if 都已在锁文件里。
- `encoding_rs`（锁文件 0.8.44，目前经 reqwest 间接引入）同时改成直接依赖。
- 两项都加到 `src/Cargo.toml` 的 workspace 依赖里，nfs_server 用 `{ workspace = true }` 引用，和现有的 `zip` 写法一致。

## 2. 协议（冻结）

### 2.1 地址

**对象来源**（聊天附件等）：

```
NFSP          { "type": "object", "obj_id": "<外层 FileObject>", "inner_path": "/archive" }              压缩包根
NFSP          { "type": "object", "obj_id": "<外层 FileObject>", "inner_path": "/archive/docs" }         包内目录
NFSP          { "type": "object", "obj_id": "<外层 FileObject>", "inner_path": "/archive/docs/a.txt" }   包内文件
Desktop / SDK { kind: "object-id", objectId: "<外层 FileObject>", innerPath: "/archive/docs/a.txt" }
```

**路径来源**：

```
cyfs:///home/alice/a.zip                         外层文件，普通文件语义不变
cyfs:///home/alice/a.zip/@/archive               压缩包根目录（规范形式不带尾斜线）
cyfs:///home/alice/a.zip/@/archive/docs          包内目录
cyfs:///home/alice/a.zip/@/archive/docs/a.txt    包内文件
dfs:///home/alice/a.zip/@/archive/docs           Files 内部 URL，同一路径
```

规则：

- **`inner_path`**：必须以 `/archive` 开头，后面是成员路径，规范形式不带尾斜线。其他首段（`/content` 等）保留，对归档无效。`obj_id` 必须是 FileObject：裸 chunk / chunklist 没有文件名，返回 `UNSUPPORTED`。
- **路径来源的保留写法**：`a.zip/@` 不带分支、`a.zip/@/content`、`a.zip/@/<其他名字>` 都返回 `NOT_FOUND`。
- **路径约束**：成员路径的每一段都必须过 `validate_name`；路径来源的总深度（外层 + 成员）受 `MAX_DEPTH` 限制。
- **嵌套**：包内文件本身是 zip 时，再进入返回 `UNSUPPORTED`，带 `reason: "nested_archive"`。路径来源仍按 D2 的规则识别（`@` 紧跟一个包内文件）；对象来源的嵌套写法以后再定。
- 包内名叫 `@` 的目录按普通目录处理。
- **cyfs-ndn 的对应形式**：`/ndn/<obj_id>/@/archive/docs/a.txt`。在 `ndn_read.rs` 里，`archive` 将来会和 `content` 一样是 File 节点的一个 step，本版不改（D15），成员只通过 NFS 读。

### 2.2 服务端解析

**对象来源**

- `resolve_ref` 遇到 `WireRef::Object { obj_id, inner_path: Some(p) }`，且 `p` 是 `/archive` 或以 `/archive/` 开头时，进入归档。其他 object ref 维持现在的 `Unsupported`（`namespace.rs:260`）。
- **鉴权**：用 `ndm::principal(state, headers)` + `ndm::may_read(principal, obj_id)`，与 repr 的 `args.source.obj_id` 分支相同（`preview/rpc.rs:160` 一带）。也就是调用方 (user, app) 持有这个对象（msg_center 通过 `bucket.grant_holdings` 授给收件人），或者有通用读权。
  - `resolve_ref` 目前拿不到请求头，要把 principal 传进去。
  - stat、list、read、repr、copy、unlock，以及缓存命中，每次都要检查。
- **node_id**：`no_<b64url(payload)>.<b64url(mac16)>`，复用 `handle.rs` 的 HMAC 编码。payload 是 `{o, e, k}`：外层 `obj_id`、成员的原始名字节（base64，隐式目录取前缀字节）、`d` / `f`。

**路径来源**

- **触发**：`resolve_native` 对整条路径 `lstat` 得到 `NotFound`，或者 `NotADirectory`（已映射为 `NOT_A_CONTAINER`，`error.rs:154`）时，从左往右找第一个满足条件的 `@` 段：它前面的前缀是普通文件（`lstat` 为 `kind == "file"`，不跟随符号链接），或者是 bucket projection 文件（`projection_at` 得到 `ProjAt::File`）。找到后，下一段必须是 `archive`，剩下的是成员路径。外层是符号链接时返回 `UNSUPPORTED`。
- **外层内容**：原生文件直接读；projection 文件取 binding 的对象，按对象来源落盘（D16）。
- **鉴权**：原生文件要求调用者对外层有 `Level::Read`，不能套用“目录只要 traverse 就能列举”的规则；projection 文件沿用 projection 现有的读检查。
- **node_id**：`na_<b64url(payload)>.<b64url(mac16)>`。payload 是 `{r, p, i, d, b, s, m, e, k}`：外层的 root、rel、ino、dev、birth、size、mtime，成员的原始名字节，`d` / `f`。`resolve_ref` 遇到 `na_` 时，先验 MAC，再 `lstat` 外层比对 `i/d/b/s/m`，不一致返回 `STALE`。projection 外层用 binding 的 etag 代替 `i/d/b/s/m`。
- **canonical_path**：`canonical_path(root, rel_of_outer) + "/@/archive" + 成员路径`。

**两种来源共同的规则**

- 新增 `Node::Archive`，里面带外层来源（原生文件、projection 或对象）、成员原始名字节（根为空）和成员信息：目录或文件、大小、压缩方法、是否加密、CRC、mtime。
- 成员身份（node_id、缓存键）一律用原始名字节，与文件名怎么解码无关（§2.7）。

### 2.3 列表、stat 与能力

- **列表项**：结构和原生列表项一致，包括 `entry_ref`、`name`、`target { kind: "dir" | "file", ref }`、`attrs`。
  - `target.ref`：对象来源是 `{ type: "object", obj_id, inner_path }`，Preview 拿到的直接就是固定引用，没有 `canonical_path`；路径来源是 live 的 `na_`，带 `canonical_path`。
  - `attrs`：`size` 是解压后大小，`mtime` 取 ZIP 的时间字段；另加 `compressed_size`、`encrypted`、`method`。
  - 分页用现有的无状态 cursor 编码；排序支持与原生列表相同的 `order`。
- **能力**：新增 `Capabilities::archive_dir()` 和 `archive_file()`，`accepts_content`、`accepts_references` 都是 false，`remove_semantics = "none"`。**不要**用 `for_kind("dir", false)`，它的 `remove_semantics` 是 `destroy`（`types.rs:123`）。
- **压缩包根的 `stat`**：`attrs.archive = { format: "zip", entries, hidden_entries, encrypted_entries, name_encoding: { encoding, by } }`。
  - `encoding` 是 encoding_rs 的名字或 `"IBM437"`；所有名字都有 UTF-8 标志或都是 ASCII 时为 `"UTF-8"`。
  - `by` 是 `"flag"`、`"utf8"`、`"detected"`、`"preference"`、`"fallback"` 之一，说明编码是怎么定下来的（§2.7）。
- **成员 `stat`**：`access_urls` 里的 `read` 是 `/nfs/v1/read/<na_ 或 no_ id>`。`etag`：路径来源是 `"<外层 size>-<外层 mtime>-<crc32 hex>"`；对象来源是 `"<obj_id>-<crc32 hex>"`，并且是强校验值。
- **只读**：任何把归档节点当作写入对象的操作，包括 `WRITE_METHODS`、`share.*` 的写操作、以它为目的地的 copy，都返回 `PERMISSION_DENIED`，带 `reason: "archive_read_only"`。以包内文件为来源的 copy 是允许的。在解析结果上集中检查一次，不要逐个方法打补丁。

### 2.4 `hello`

在现有的 `repr` 旁边加一项：

```json
"archive": {
  "available": true,
  "formats": [{ "format": "zip", "extensions": ["zip"], "methods": ["stored", "deflate", ...], "encryption": ["zipcrypto", "aes"] }],
  "limits": { "max_central_directory_bytes": ..., "max_entries": ..., "max_materialize_bytes": ... }
}
```

`methods` 只列样本实测能解的（T8）。客户端只靠这一项判断能不能进包。

### 2.5 错误

在现有 `error.rs` 的扩展码里新增三个（`as_str`、`http_status` 都要加），并同步写进 nfs_server README 的错误码表和 websdk：

| 码 | HTTP | 含义 |
| --- | --- | --- |
| `PASSWORD_REQUIRED` | 403 | 成员已加密，当前用户还没有对这个包验证过的密码 |
| `PASSWORD_INVALID` | 403 | 提交的密码没通过校验（快速校验失败，或完整读取后 CRC / HMAC 不符） |
| `ARCHIVE_CORRUPT` | 422 | 中央目录解析失败、成员数据损坏，或解压结果与声明不符。分不清是密码错还是数据坏时，用这个码并带 `reason: "decrypt_or_integrity"` |

其余情况用已有的码：

- 路径来源的外层已变化：`STALE`
- 不支持的格式、方法或嵌套：`UNSUPPORTED`，带 `reason`
- 超出索引、解压或磁盘上限：`QUOTA_EXCEEDED`，带 `limit`
- 同名成员：`AMBIGUOUS_ENTRY`
- 对象来源无权读取：`PERMISSION_DENIED`

### 2.6 密码 RPC

| 方法 | 参数 | 行为 |
| --- | --- | --- |
| `archive.unlock` | `at`（压缩包根或某个加密成员，两种来源都可以）、`args.password` | 用目标成员做快速校验（ZipCrypto 的校验字节，或 AES 的口令校验值；`at` 是根时取第一个加密成员）。通过后，把密码放进“待验证”会话状态并返回 `ok`；不通过返回 `PASSWORD_INVALID` |
| `archive.forget` | `at`（压缩包根），或 `args.all = true` | 删除当前用户对这个包（或所有包）记住的密码，以及会话状态 |

- 第一次完整读到成员末尾、CRC / HMAC 都对上时，才把这个密码升级为“已验证”；这时外层的整文件 sha256 如果已经可用，就持久化（D6）。
- 完整读取校验失败时，丢弃这个待验证密码，不影响其他已验证的密码。
- 这两个方法都是只读会话操作，不进 `WRITE_METHODS`，不要求 `seq`。
- 密码不能出现在 URL、日志、普通元数据或浏览器存储里。

### 2.7 文件名编码（D11）

**原则**

- 以整个压缩包为单位决定一种编码：同一个包通常由同一台机器、用同一个代码页打成，名字合在一起探测，远比逐个名字探测准。
- 编码只影响显示和路径字符串；成员身份始终用原始字节。

**步骤**

1. 有 UTF-8 标志或 Info-ZIP Unicode Path 扩展（0x7075）的条目，用 crate 解析好的名字。其余条目取 `file_name_raw()`。
2. 其余条目全是 ASCII，就不需要处理；全部是合法 UTF-8，就按 UTF-8（`by: "utf8"`）。
3. 否则开始探测：
   - 把这些原始名字按 `/`（0x2F）切成段并去重，用 `\n` 连接后喂给 chardetng：不传 TLD，`Iso2022JpDetection::Deny`，`Utf8Detection::Deny`。按 `/` 切原始字节是安全的，这几种编码的第二字节都不会是 0x2F。
   - **候选集 C** = GBK（encoding_rs 按 GB18030 解码）、Big5、Shift_JIS（即 CP932）、EUC-KR（即 CP949）、EUC-JP 中，能用 `decode_without_bom_handling_and_without_replacement` 无错解码全部名字的编码。
   - **N** = 去重后的非 ASCII 字节数。
4. 按顺序取第一条成立的：
   1. `N ≥ 16`：chardetng 的结果在 C 里就用它（`by: "detected"`），否则用 CP437（`"fallback"`）。
   2. 请求方的语言偏好对应的编码在 C 里，就用它（`"preference"`）。
      - 偏好取 `Accept-Language` 里第一个 CJK 语言：`zh-CN` / `zh-SG` / `zh-Hans` / `zh` → GBK；`zh-TW` / `zh-HK` / `zh-MO` / `zh-Hant` → Big5；`ja` → Shift_JIS；`ko` → EUC-KR。
      - 浏览器发出的请求会自动带这个头，Agent、CLI 通常不带。
   3. `N ≥ 8`，并且 chardetng 的结果在 C 里：用它（`"detected"`）。
   4. 其余：CP437，也就是 crate 的 `name()`（`"fallback"`）。
5. **分隔符要在解码之后处理。** CP932、Big5、GBK 的第二字节可能是 0x5C（例如 CP932 的“表”是 `95 5C`）。解码后的名字：
   - 只含 `\`、不含 `/`：当作 Windows 工具写的分隔符，替换成 `/`。早期 .NET 和 PowerShell 的 Compress-Archive 会这样写。
   - 同时含两者：不可寻址。
   
   然后按 D12 做安全检查。**不要用 crate 的 `enclosed_name()`**：它在 CP437 解码后的字符串上按操作系统处理反斜杠，会把 CP932 的“表”拆成两段。
6. **只有走到第 2 条（`"preference"`）时，结果才会因请求方而不同。** 这时路径解析先用当前请求方的结果查，查不到再依次用 C 里的其他编码和 CP437 查，所以别的语言的用户、Agent、CLI 拿到的地址也能打开。其他情况下结果和请求方无关。下载时的默认文件名按同样的规则解码。

**阈值依据**

阈值 16 和 8 是 2026-11-16 用 chardetng 1.0.0 实测得出的初值。样本是常见的文件名词 + 扩展名，构造了约 1,600 个单文件和多文件的包，统计不同 N 下的探测正确率：

| N（去重后非 ASCII 字节） | GBK | Big5 | CP932 | EUC-KR |
| --- | --- | --- | --- | --- |
| 0–7 | 41% | 90% | 75% | 59% |
| 8–15 | 87% | 98% | 98% | 99% |
| 16–23 | 94% | 100% | 100% | 100% |
| ≥ 24 | 97–100% | 100% | 100% | 100% |

- 两个汉字（4 字节）的 GBK 名字常被判成 EUC-KR 或 EUC-JP，并且能无错解码成乱码：短名字时候选集 C 几乎总是包含多个编码，靠能否解码排除不了。
- CP437 西文名（如 `Müller.docx`）非 ASCII 字节少于 4 个时，有 14% 被判成 Shift_JIS；4 个及以上为 0%。
- 传 TLD 提示（`cn`、`jp` 等）会压过很强的证据：带 `cn` 时，整包 CP932 被解成 GBK 乱码；带 `jp` 时，整包 GBK 被解成 EUC-JP 乱码。所以不用 TLD，只在名字短的时候参考语言偏好。

T8 要用真实样本复核这两个阈值。

## 3. 现状与接入点

跨仓库路径相对本文件所在目录。

| 环节 | 现状 | 接入点 |
| --- | --- | --- |
| MessageHub 附件 | 会话里点附件后，调用 `openPreview({ source: attachment.source })`（`components/conversation/media/context.ts:39`、`MessageMediaHost.tsx`），可以是内嵌查看器，也可以是新窗口 | 不用改。Preview 自己识别 zip（T6） |
| Preview 读对象 | object-id 走 [ndmSource.ts](../src/frame/desktop/src/components/preview/ndmSource.ts)：先 `/ndm/v1/store/get_object` 读 FileObject 元数据（名字、大小、mime），再 `/ndm/proxy/v1/read/object/open` 把整份内容读成 Blob，上限 256 MiB | 读完元数据、读内容之前判断是不是 zip，是就改走 NFSP |
| 对象读权限 | `ndm::may_read` = 调用方 (user, app) 的 holding 或通用读权（`ndm/mod.rs:312`）；msg_center 用 `bucket.grant_holdings` 授给收件人（`msg_center/src/buckets.rs:478`）；repr 的对象分支已经这样检查（`preview/rpc.rs:160` 一带） | 对象来源的所有归档操作都按它鉴权 |
| 对象落盘 | `materialize`：`content_of` + `open_reader` + 复制到文件；`local_input` 包了一层（`preview/mod.rs:571`、`613`） | 外层落盘直接复用（D16） |
| NFS 路径解析 | URI 整串先解码再拆分（`namespace.rs:174`）；`resolve_native` 在 `lstat` 失败时只会退到 bucket projection（`namespace.rs:207-230`）；路径经过普通文件时报 `NOT_A_CONTAINER`（`error.rs:154`） | 在 `resolve_native` 的失败分支里加归档拆分（§2.2） |
| 句柄与引用 | `nh_` 是 HMAC 签名的 JSON payload（`handle.rs`）；`node_ref` 为原生节点生成句柄（`namespace.rs:419`）；`resolve_ref` 按前缀分派，object ref 一律拒绝（`namespace.rs:260`） | 复用 `Handles` 编码新增 `na_` / `no_`；在 `resolve_ref` 里加 object 分支 |
| 能力 | `for_kind("dir")` 是 `destroy`（[types.rs:116](../src/frame/nfs_server/src/types.rs)） | 新增 `archive_dir` / `archive_file` |
| 数据面 | `read_node` 只接受原生文件；有 `parse_range`、`file_stream`、`apply_content_policy` 和弱 ETag（[server.rs:580-680](../src/frame/nfs_server/src/server.rs)） | 加 `Node::Archive` 分支；物化后的文件直接走现有的原生文件响应代码 |
| repr | `resolve_source` 按 `Node` 产出 `Input::Path` / `Input::Object`（`preview/rpc.rs:123`）；`native_obj_id` 按 (path, size, mtime) 缓存整文件 sha256（`preview/mod.rs:810`） | 加归档成员分支：物化后得到 `Input::Path(缓存文件)`，id = 缓存文件的 sha256 |
| copy 任务 | `copy_submit` 的来源是原生路径，目的地必须是本地目录（[copy.rs:421](../src/frame/nfs_server/src/copy.rs)） | 允许来源是包内文件（两种来源都要），执行时从 provider 流式读取；拒绝包内目的地 |
| 密码先例 | 分享链接只存 `password_hash`（[share/db.rs:67](../src/frame/nfs_server/src/share/db.rs)） | 新建凭据表（D6），不复用这张表 |
| zip crate | 2.6.1，默认特性；`ZipArchive<R>` 以 `Arc<Shared>` 派生 `Clone`；有 `by_index_decrypt`、`by_index_raw`、`data_start`、`file_name_raw`、`is_symlink`、`encrypted`；没有“用已解析的元数据加新 reader 构造归档”的公开接口 | provider 直接用这些 API；不用 `enclosed_name()`（§2.7） |
| 编码库 | `encoding_rs` 0.8.44 已在锁文件（经 reqwest）；`chardetng` 不在 | 两项都加为 workspace 直接依赖（§1 表后，owner 已确认） |
| Preview 引用 | `ObjectIdRef` 没有 `innerPath`（[types.ts:23](../src/frame/desktop/src/components/preview/types.ts)）；`refIdentity` 用的是 path / objectId 加 `version`（[session.ts:72](../src/frame/desktop/src/components/preview/session.ts)）；转 NFSP ref 时只给 `obj_id`，`enumerateContainer` 会丢掉对象条目的 `inner_path`（[nfspProvider.ts:78](../src/frame/desktop/src/components/preview/nfspProvider.ts)、`226-262`）；`ContentPreview` 拒绝目录和容器 | 加 `innerPath` 并贯穿；新增目录页 |
| SDK 引用 | `TransferableContentRef` 的 object-id 分支没有 `innerPath`；`contentRefString` 输出 `obj://<id>`；有 `isTransferableRef`、`parseSystemOpen`，app-frame 的 `PreviewEmbedSource`（[content.ts:5](../../buckyos-websdk/src/content.ts)、`87-110`、`223`，[app-frame.ts:274](../../buckyos-websdk/src/app-frame.ts)）；NFSP 的 `ObjectRef` 已有 `inner_path`（[nfsp.ts:33](../../buckyos-websdk/src/nfsp.ts)）；`encodeObjSegment` 保留 `@`（[object_url.ts:295](../../buckyos-websdk/src/object_url.ts)） | 加 `innerPath`，序列化、校验、跨 App 打开都要带上 |
| Preview 读取与容器 | 对 cyfs-path 做 NFSP `resolve`，读取地址取 `access_urls.read` 或 `readUrl(nodeId)`（`nfspProvider.ts:88`）；容器是 `parentCyfsPath`，`enumerateContainer` 只列文件、上限 5000 | 两种来源的成员都走这条读取路径；`parentCyfsPath` 识别归档边界 |
| Files 列表 | `MainContent` 接收 `FileItemList`、视图和选择状态、`onSelect` / `onOpenFolder(url)` / `onOpenFile` 等回调（[MainContent.tsx:92](../src/frame/desktop/src/app/filebrowser/MainContent.tsx)）；空态写死了上传、同步的文案（`MainContent.tsx:357-372`）；手机列表的点击走 `onSelect` | Preview 直接嵌这个组件；空态改由宿主或能力决定 |
| Files 数据层 | DFS reader 用 `dfs://` URL 调 NFS `list(path)`（[data/nfsp/readers.ts](../src/frame/desktop/src/app/filebrowser/data/nfsp/readers.ts)）；列表项映射在 `data/nfsp/mapping.ts`；`collectionDirectory.ts` 已有按 NFSP ref 列目录的先例；`parentUrl` / `crumbsForUrl` / `copyPathText` 在 [data/urls.ts](../src/frame/desktop/src/app/filebrowser/data/urls.ts)；`sysdlg/select` 也引用这套数据层（`SelectDialog.tsx:9-16`） | 路径来源对 reader 来说就是普通 DFS 路径；对象来源新增一个按 ref 列目录的 reader |
| Files 打开与命令 | `handleOpenFile` → `openContent`（[FileBrowserView.tsx:427](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx)）；`commandState` 按能力裁剪，`settings` 返回 unsupported（[menu/commands.ts](../src/frame/desktop/src/app/filebrowser/menu/commands.ts)）；图标大小在视图菜单里，存 localStorage | 加进包判断、只读命令裁剪、设置项 |
| cyfs-ndn | `/@/` 分隔 inner path，File 节点的 step 只有 `content`（[ndn_read.rs:226](../../cyfs-ndn/src/bucket_store/src/ndn_read.rs)） | 本版不改 |

## 4. 实施任务

顺序：T1 → T2 → T3 → T4 → T6（先让 MessageHub 场景跑通）→ T7。T5（密码）在 T2 之后可以并行；T6、T7 先不接密码界面，T5 完成后再接。T8 贯穿全程。

### T1 — provider、索引与路径来源解析（P0）

- [ ] 新建 `src/frame/nfs_server/src/archive/`：`mod.rs`（解析、stat、list）、`zip.rs`（provider）、`names.rs`（§2.7）、`cache.rs`（索引、外层落盘、物化缓存）。
- [ ] 读取器：写一个可 `Clone` 的 `Read + Seek` 读取器（内部是 `Arc<File>` 加各自的游标），按位置读。nfs_server 也在 Windows 上构建（`fsutil.rs` 有 `cfg(windows)` 分支），所以 Unix 用 `FileExt::read_at`，Windows 用 `FileExt::seek_read`。
- [ ] 索引缓存：按外层版本键缓存一个解析好的 `ZipArchive<该读取器>`，每个请求 `clone` 一份，共享 `Arc<Shared>` 和 fd。版本键：路径来源是外层版本，对象来源是 `obj_id`。
  - 固定 fd 有一个好处：外层被 rename 覆盖后，旧索引读到的仍是旧 inode 的字节，不会错配；新请求会因为版本比对失败而得到 `STALE`。
  - Windows 上持有句柄会影响用户删除或覆盖外层文件，所以空闲的缓存项要关闭 fd（初值 60 秒）。`zip` crate 不能用已解析的元数据加新 reader 构造归档，所以由读取器自己重新打开：fd 关闭后，下次读取时按路径重新打开，先比对 `ino / dev / size / mtime` 与版本键，不一致就返回 `STALE`，绝不能用旧索引读新字节。
- [ ] 解析中央目录放进 `spawn_blocking`，并设上限：中央目录字节数（初值 64 MiB）、条目数（初值 200,000）、索引缓存总数。超出返回 `QUOTA_EXCEEDED`。
- [ ] 建索引时按 §2.7 决定编码。会因请求方而不同的（`"preference"`）只缓存原始名字和候选集，每个请求现解码；其余的直接缓存解码结果。然后按 D12 计算可寻址集合、补隐式目录、统计 `hidden_entries`，生成按目录分组、排好序的子项表，供 list 分页。
- [ ] 按 §2.2 在 `resolve_native` 里接入路径来源的拆分（含 projection 文件作为外层），新增 `Node::Archive` 和 `na_` 句柄。在所有按 `Node` 分派的地方补上分支（`node_ref`、`resolve_ref`、`display_name`、`canonical_path` 等）；编译器报出的每一处 match 都要处理，不能用 `_ =>` 吞掉。
- [ ] 按 §2.3 实现 stat、list 和能力，按 §2.5 实现错误码，按 §2.4 实现 `hello`。
- [ ] 只读：按 §2.3 在解析结果上集中拒绝写操作。
- [ ] 单测：
  - 解析规则：真实名叫 `@` 的目录、`a.zip/@`、`a.zip/@/content`、嵌套、外层是符号链接、外层被替换。
  - list 分页、同名和不安全的条目、权限。
  - §2.7 的每一步：UTF-8 标志、无标志的 UTF-8、各 CJK 编码的长名和短名、有 / 无语言偏好、CP437 西文、反斜杠分隔、CP932 含 0x5C 的名字（表、ソ、十）；以及“按另一种偏好解码出的路径也能解析”。

### T2 — 数据面读取与缓存（P0，依赖 T1）

- [ ] 在 `read_node` 里加 `Node::Archive` 文件分支，按 D9 处理：
  - **Stored 且未加密**：区间 = `data_start` 加偏移，直接从外层文件（或对象来源的落盘文件）读，复用 `parse_range` 和 `file_stream`。
  - **没有 Range，或 Range 从 0 开始**：在 blocking worker 里解压，经有界 channel 推给 HTTP body（有背压，客户端断开时停止）。返回 200、`Accept-Ranges: bytes`、`Content-Length` = 声明的解压大小。
  - **起点不为 0 的 Range**：先物化，再按原生文件响应 206。
- [ ] 解压结果与声明不符的处理：最多输出声明的大小；CRC / HMAC 失败或实际长度不符时中断连接（响应头已经发出，只能中断）。物化路径要在校验完成后才把结果标记为可用。
- [ ] 对所有成员调用 `apply_content_policy`，与原生文件一致：包里的 HTML 不会在 Desktop 源下运行。
- [ ] 物化缓存：
  - 放在 nfs_server 缓存目录下的 `archive/` 子目录；文件名由服务端从缓存键生成，不拼成员名。
  - 先写临时文件再 rename，权限 0600；启动时清掉残留的临时文件。
  - 缓存键 = sha256(外层版本键 + 成员原始名字节)。
  - 同一个键的并发请求合并成一次；最后一个等待者离开时才取消。
- [ ] 资源上限：
  - 物化与外层落盘共用一个磁盘总预算，按 LRU 驱逐（初值 4 GiB）。
  - 单个成员或外层的上限，初值等于总预算；声明大小超过时直接返回 `QUOTA_EXCEEDED`。
  - 解压 worker 并发数，初值为 CPU 数。
  - 所有初值都写进配置，T8 基准后再调。
- [ ] 加密成员：读取、缓存命中、repr、copy 时，都先检查当前用户是否已经对这个包验证过密码（T5）。T5 完成前，加密成员一律返回 `PASSWORD_REQUIRED`。
- [ ] repr：在 `resolve_source` 里加归档成员分支。物化后得到 `Input::Path(缓存文件)`，id 用 `native_obj_id` 对缓存文件计算；格式识别和转换器都不变。
  - 加密成员只有先解锁才能物化，所以没有密码的用户算不出 work key，拿不到别人缓存的预览产物。
- [ ] copy 任务：`copy_submit` 允许来源是包内文件，两种来源都要支持；拒绝包内目录，本版不做批量解压。执行时从 provider 流式写入，复用现有的冲突、进度和取消机制；对象来源在提交和执行时都检查 `may_read`。
- [ ] 单测：Stored 的 Range、Deflate 的流式与物化、206 / 416、取消、并发合并、预算驱逐、CRC 失败、压缩炸弹（声明很小、实际很大，以及反过来）。

### T3 — 对象来源（服务端，P0，依赖 T1、T2）

- [ ] 按 §2.2 在 `resolve_ref` 里接入 `WireRef::Object` + `/archive` 的 `inner_path`，新增 `no_` 句柄；其他 object ref 保持 `Unsupported`。
- [ ] 鉴权：把 `ndm::principal(state, headers)` 传到解析层，stat、list、read、repr、copy、unlock 和缓存命中都调用 `ndm::may_read` 检查外层 `obj_id`。确认 Preview 走 NFSP 时用的 principal，与它现在读 `/ndm/...` 时的一致，T8 的真实 Zone 验证要覆盖这一点。
- [ ] 外层落盘（D16）：
  - 用 `materialize` 把外层 FileObject 的内容写到 `archive/objects/` 下，文件名由 `obj_id` 生成；写的时候算整文件 sha256，保存在旁边。
  - 计入 T2 的磁盘预算；FileObject 声明的大小超过上限时直接返回 `QUOTA_EXCEEDED`。
  - 同一个对象的并发请求合并成一次；最后一个等待者离开时才取消。
  - 落盘期间 resolve / list 等待完成，Preview 显示加载中，可以取消。
  - 内容不在本地时沿用 `open_reader` 的现有行为，失败如实报错。
- [ ] 落盘完成后，与路径来源共用 provider、索引、物化、repr、copy。对象不会变，没有 `STALE`；成员的 `etag` 是强校验值。
- [ ] bucket projection 作外层：T1 的路径拆分识别 projection 文件后，内容走这里的对象落盘，鉴权沿用 projection 的读检查。
- [ ] 单测：有 holding 的用户能列、能读，没有 holding 的用户被拒（包括缓存命中时）；同一对象的并发落盘合并；超出上限；非 FileObject；`inner_path` 不以 `/archive` 开头。

### T4 — websdk、引用与路径工具（P0，依赖 T1 的协议）

- [ ] websdk `nfsp.ts`：新增三个错误码和 `hello.archive` 类型，以及 `archive.unlock` / `archive.forget` 两个调用。
- [ ] websdk `content.ts`：`TransferableContentRef` 的 object-id 分支加 `innerPath?: string`。
  - `isTransferableRef` 校验 `innerPath`：以 `/archive` 开头，每段合法。
  - `contentRefString` 带上 `innerPath`，保持可以往返解析，格式与 `object_url.ts` 的 `cyfs_object`（`cyfs://<objid>/archive/...`）对齐；有解析它的地方一起改。
  - `parseSystemOpen`、app-frame 的 `PreviewEmbedSource` 和 conversation 附件透传 `innerPath`。
- [ ] Desktop Preview：
  - `ObjectIdRef` 加 `innerPath`；`refIdentity` 带上它。
  - `nfspProvider` 转 NFSP ref 时带 `inner_path`。
  - `resolvePreviewSource` 遇到带 `innerPath` 的 object-id 时，走 NFSP `resolve` 和 `readRefOf`，不走 `ndmSource`。
  - `enumerateContainer` 保留对象条目的 `inner_path`；容器是父 `innerPath`，根是 `/archive`。
- [ ] 归档边界工具放在 Desktop 的 `app/filebrowser/data/urls.ts`，Files 和 Preview 共用：
  - 路径来源：`archiveSplit(path) → { outer, member } | null`、`archiveRootOf`；`parentUrl` 在压缩包根时返回外层文件所在目录，并告诉调用方要选中外层文件；`crumbsForUrl` 把外层文件显示成一节，隐藏 `@` 和 `archive`；`copyPathText` 复制完整的规范路径。
  - 对象来源：求父 `innerPath`、生成面包屑（根显示附件名）。
  - Preview `session.ts` 的 `parentCyfsPath` 使用同一套边界规则。
- [ ] 确认 `object_url.ts` 和 Desktop 的 `normalizeCyfsPath` 对 `/@/archive/` 路径的往返不变（补测试即可，按 D2 不需要改编码规则）。
- [ ] 按仓库约定构建 websdk dist。

### T5 — 密码（P0，依赖 T2）

- [ ] 凭据表放在 nfs_server 的 SQLite：`(user_id, archive_sha256, password, verified_at)`，一个包可以有多条（不同成员用不同密码）。会话状态（待验证 / 已验证）放在内存里，按 `(user_id, 外层版本键)` 记录。
- [ ] 按 §2.6 实现 `archive.unlock` 和 `archive.forget`，两种来源都支持；同一个包的并发解锁请求合并。
- [ ] 读加密成员时：
  - 先试会话里已验证的密码；
  - 再拿外层整文件的 sha256，试持久化的密码。路径来源用 `native_obj_id`（有缓存；没缓存就当场计算，界面显示“正在检查”），对象来源用落盘时算好的值。
  - 都不行就返回 `PASSWORD_REQUIRED`。
  - 一个成员成功不等于整包解锁：每个加密成员都要有能通过它校验的密码。
- [ ] 持久化时机：密码第一次被完整读取验证通过，并且整文件 sha256 已可用。只做过快速校验的密码只留在会话里。
- [ ] 不同用户之间不共享会话状态和凭据。调用方 App 能否使用，沿用现有的会话授权。
- [ ] 单测：快速校验误判（ZipCrypto 只有 1 字节校验）后，在完整读取时被纠正；不同成员不同密码；忘记单包和忘记全部；其他用户看不到；外层换了内容（sha256 变了）后不能复用；同一个 zip 在聊天附件和 Zone 文件里共用记住的密码。

### T6 — Preview 目录浏览，含 MessageHub 入口（P0，依赖 T3、T4，T5 完成后接密码界面）

- [ ] **识别 zip**：Preview 打开 `.zip`（`hello` 声明支持）时显示压缩包根的目录页，不论来源。
  - object-id：在 `ndmSource` 读完 FileObject 元数据、读内容之前，按文件名判断，转成 `{ objectId, innerPath: "/archive" }`。
  - cyfs-path：转成 `<path>/@/archive`。
  - 格式不支持或包损坏时，显示错误，并保留原有的下载 / 保存操作。
- [ ] **MessageHub 场景**：会话里点 zip 附件，在内嵌查看器（`MessageMediaHost`）和新窗口里都显示目录页；导航栈留在查看器内部，关闭查看器回到会话。附件原有的“下载”“保存到 Zone”等操作继续可用。
- [ ] **目录页**：嵌 Files 的 `MainContent`。
  - 路径来源的数据用 Files 的 DFS reader 和 `FileItemList`，引用方式同 `sysdlg/select`。
  - 对象来源新增一个按 NFSP ref 列目录的 reader，列表项映射复用 `data/nfsp/mapping.ts`，做法参考 `collectionDirectory.ts`。
  - 有宿主耦合时只做最小拆分，并让 Files 自己也用拆出来的部分。视图模式、图标大小、排序沿用 Files 的偏好。
  - 手机上的点击按 Files 的规则（普通点击打开，选择模式下点击是选择），要同时接 `onSelect`，不能只接 `onOpenFile`。
- [ ] **导航**：在目录页点目录，在同一个 Preview 里进入；点文件，原地切到成员预览，复用现有的各个渲染器。
  - Preview 自己维护导航栈，每层记住目录、选中项和滚动位置。
  - 返回（桌面的返回按钮、手机的系统返回）：成员 → 所在目录页 → 上级目录 → 压缩包根 → 关闭 Preview（或回到打开它之前的那一项）。
  - Preview 不提供“到外层目录”，那是 Files 的事。
- [ ] **深链接**：直接打开一个成员引用时，用 T4 的工具恢复它所在的目录，返回能回到这个目录页。
- [ ] **上一项 / 下一项**：沿用现有的按容器枚举，容器就是成员所在的包内目录，只遍历这个目录里的文件，上限 5000 不变。
- [ ] **空态和提示**：
  - 空包显示“压缩包是空的”，不出现上传、拖入、同步文案（`MainContent` 的空态改由宿主传入，或按能力决定）。
  - `hidden_entries > 0` 时显示一行提示。
  - 路径来源出现 `STALE` 时，提示“压缩包已变化”并提供“重新打开”，不静默跳到新版本的同名成员。
- [ ] **密码**（T5 完成后）：遇到 `PASSWORD_REQUIRED` 时，在 Preview 里弹出密码输入，状态有输入中、验证中、失败、取消；验证通过后重试读取。目录页菜单里提供“忘记这个压缩包的密码”。
- [ ] **成员菜单**：
  - 下载。
  - 保存到 Zone：用系统选择器选目标文件夹，然后提交 T2 的 copy 任务。
  - 复制引用 / 路径、在新窗口预览。
  - “用其他应用打开”按现有内容打开规则走。
  - 转发、分享、导入等按能力禁用（D15）。
- [ ] 渲染器遇到相对资源（比如 HTML 引用同目录的图片）时，本版不解析，降级为不显示，不能把相对地址交给 Desktop 根路径。

### T7 — Files 压缩包目录模式（P0，依赖 T1、T4）

- [ ] 按 D13 加设置项。开启时，`handleOpenFile` 遇到 `.zip`（且 `hello` 声明支持）就跳到 `dfs://<path>/@/archive`；关闭时走 `openContent`。两种模式都保留右键菜单里的“预览压缩包”（交给 Preview）和“下载”。
- [ ] 包内列表由现有 DFS reader 直接显示，大小是解压后的大小，类型按成员扩展名。外层列表里压缩包仍显示为 archive 类型和压缩后的大小。
- [ ] 命令裁剪：
  - 包内允许：打开、预览、下载、复制（用来粘贴到可写目录，走 T2 的 copy 任务）、复制路径。
  - 其余（改名、移动、剪切、删除、上传、新建文件夹、粘贴、权限、分享、拖放）按能力禁用，提示“压缩包里的文件是只读的”。
  - 粘贴的目的地是包内时，同样拒绝。
- [ ] 地址栏、面包屑、上级、前进 / 后退、tab 恢复都用 T4 的边界工具：从压缩包根往上，回到外层目录并选中原包；`STALE` 时刷新到外层目录并提示。
- [ ] 空态与提示同 T6。
- [ ] 侧栏目录树、搜索不会进入压缩包（侧栏本来只列目录，搜索不建归档索引），确认现有行为不变即可。
- [ ] 系统文件选择器：按 D7 不进包。确认 `.zip` 在选择器里仍是可以选的文件，不受 Files 设置影响。

### T8 — 验证、基准与文档（P0 验收）

- [ ] 样本（可再分发，放进测试 fixtures）：
  - 空包、只有隐式目录、ZIP64（成员数超过 65535；单个成员超过 4 GiB 的可以用生成脚本代替）。
  - 各压缩方法各一个。
  - 文件名：有 UTF-8 标志、无标志的 UTF-8、GBK、Big5、CP932、EUC-KR，每种各要长名包和两字短名包；CP437 / CP850 西文；反斜杠分隔（PowerShell 5 Compress-Archive）；CP932 含 0x5C 的名字。
  - 同名成员；`..`、绝对路径、符号链接条目；名叫 `@` 和 `content` 的成员。
  - 损坏的包；ZipCrypto、AES-128 / 256；不同成员不同密码。
  - 压缩炸弹；大量小文件。
  
  把每一项的实测结果写进 §8，并据此确定 `hello.methods`，复核 §2.7 的阈值。
- [ ] 一致性与权限：
  - 路径来源：建索引后覆盖原包、读取过程中被 rename 替换、同路径换新包。
  - 对象来源：有 holding 和没有 holding 的用户；收件人与非收件人。
  - 两个用户读同一个包（含加密）；权限撤销后的新请求；忘记密码。
- [ ] 端到端：
  - **MessageHub 优先**：发一个 zip 到会话，收件人在内嵌查看器和新窗口里浏览、预览成员（图片 / 文本 / PDF 读尾部 / 音视频拖动 / 文档转换），把一个成员保存到 Zone，下载一个成员，走一遍密码交互；手机端用系统返回走完整个返回链。
  - 然后测 Files 进包、在 Preview 打开 Zone 里的 zip、从 embed 深链接打开成员（两种来源），以及设置关闭时的行为。
  - 真实 Zone 和真机手机分别记录结果，mock 不能代替数据链路验收。
- [ ] Files 回归：用同一组条目对照 Files 和 Preview 共享列表的行视图 / 图标视图 / 手机视图、缩略图、元数据格式、排序、选择、键盘 / 触摸打开、分页滚动；共享组件调整后，Files 原有的浏览行为不变。
- [ ] 基准：
  - 冷 / 热缓存下首次列目录、首次成员预览、重复打开、PDF 读尾部、视频拖动、多用户并发；对象来源的外层落盘单独计时。
  - 记录首字节时间、读取的压缩字节、解压字节、临时写入字节、峰值 RSS / CPU。
  - 先取基线，再调整 T2 的初值。
- [ ] 文档：
  - nfs_server README：错误码、`hello`、归档一节、已知边界。
  - cyfs-ndn 的 NFSP 文档（`NamedFileSystem_Protocol_v0.md`）：补 §2 的地址、object ref 的 `inner_path` 和错误码。
  - Desktop / Preview / MessageHub 的产品行为，websdk 类型与测试。
  - 按 AGENTS.md 的 cmd_skills 表检查受影响的手册：Agent 通过 `buckyos` CLI / NFSP 也能读成员，确认 `buckyos_admin` 等手册是否需要提一句，并写明依据。
- [ ] 验证入口：
  - 在 `src/` 下运行 `cargo test -p nfs_server -- --test-threads=1`。
  - Desktop 运行 `pnpm check`、`pnpm lint`，以及定向的 Playwright（MessageHub 附件、Preview、Files、选择器）。
  - websdk 按 package scripts 跑 nfsp / object_url / content 测试。

## 5. 完成标准

- **MessageHub**：收件人点开聊天里的 zip，能浏览目录、原地打开成员、返回原来的位置（包括手机返回），能把成员保存到 Zone 或下载；复制的成员引用在刷新后、在别的 App 里都能重新打开；不是收件人、没有读权的用户打不开。
- **Zone 文件**：在 Files 和 Preview 里能做到同样的事。外层文件被替换后，旧地址得到明确的“已变化”提示，不会读到错配的内容。
- **加密包**：同一用户验证一次之后，再次打开不用重新输入（聊天里的和 Zone 里的同一个包也算）；其他用户拿不到解锁结果，也拿不到明文缓存或预览产物；可以忘记密码。
- **只读**：包内写操作被服务端拒绝。
- **资源**：全程不默认解压整包；索引、落盘、物化、并发、磁盘都有上限，并有可复现的测试。
- **文件名**：中日韩 Windows 打的包，文件名能正确显示（短名按 §2.7 的规则）。
- §7 的已知限制如实写进 README。

## 6. 后续（不在本版）

- 路径来源的固定引用（依赖 `snapshot`），以及在选择器里选包内成员（D7）
- 把包内成员转发到别的会话（要先把成员做成独立对象）
- cyfs-ndn 的 `/ndn/<obj_id>/@/archive/...` 直接读
- 嵌套包、分卷包、7z / RAR / tar / tar.gz（先评估 solid block 和顺序流的索引成本、密码和跨平台依赖；新依赖要先评审）
- 批量“解压到”
- host realm 归档（保留 sudo 会话，不改写成 DFS 来源）
- 手动切换文件名编码（`by` 为 `"preference"` 或 `"fallback"` 时，让用户选）
- 渲染器的包内相对资源
- 对象来源改用分块读取，代替整份落盘

## 7. 已知限制与风险

- **版本识别有盲区（D4）**：路径来源在同一秒内原地改写成同样大小、且不换 inode 时识别不到。整文件 sha256 缓存（`native_obj_id`）有同样的盲区；在密码场景里最多导致记住的密码试一次失败，不会泄露内容。
- **记住的密码明文存在 nfs_server 的库里（D6）**：拿到服务数据目录的人能读到。以后系统有可逆的秘密存储时再迁移。
- **短文件名可能解错（§2.7）**：非 ASCII 字节少于 16 时，结果依赖请求方的语言偏好。比如日文用户收到一个只有两个汉字名字的 GBK 包，可能显示成乱码，要等后续的手动切换。
- **大附件第一次打开要等落盘（D16）**：对象来源第一次打开时要把整个附件写到本地，大附件会有可感知的等待，并占用磁盘预算。
- **数据面中途失败只能断开连接（T2）**：流式输出时响应头已经发出，中途发现 CRC 不符只能中断连接。
- **物化成本（D9）**：大的 Deflate 成员第一次被 Range 读时，要等完整物化，PDF 读尾部、视频拖动的首帧会慢。T8 基准后决定要不要做分段解压。

## 8. 实施记录

2026-11-16 实施，未提交。

### 8.1 落地位置

| 任务 | 状态 | 位置 |
| --- | --- | --- |
| T1 | 完成 | `nfs_server/src/archive/`：`names.rs`（§2.7、D12）、`zip.rs`（读取器、中央目录遍历、成员读取）、`cache.rs`（索引、落盘、物化、预算、单飞）、`mod.rs`（`Node::Archive`、解析、stat/list、能力、`hello`、只读）；`namespace.rs` / `access.rs` / `handle.rs`（`seal` / `open`）/ `error.rs` / `watch.rs` 接入 |
| T2 | 完成 | `archive/mod.rs` 数据面；`server.rs` 抽出 `serve_file` 供原生文件和成员共用；`preview/rpc.rs` repr 分支；`copy.rs` 包内来源 |
| T3 | 完成 | `archive::prepare`（请求级预备：对象鉴权、落盘、建索引）+ task-local 请求上下文 |
| T4 | 完成 | websdk `nfsp.ts`（错误码、`hello.archive`、`archiveUnlock` / `archiveForget`、地址工具）、`content.ts`（`innerPath`、`obj://<id>/archive/...` 往返、`parseSystemOpen`）；Desktop `urls.ts` 边界工具、Preview `types.ts` / `session.ts` / `nfspProvider.ts` |
| T5 | 完成 | `archive/password.rs` + `archive/mod.rs` 的 `password_for` / `settle` / `op_unlock` / `op_forget` |
| T6 | 完成 | `components/preview/archive.ts`（识别）、`ArchiveBrowser.tsx`（目录页、导航栈、密码框、成员操作）、`ContentPreview.tsx`（导出的组件先识别再分派，原组件改为内部 `ContentViewer`） |
| T7 | 完成 | `data/archiveBrowse.ts`（设置）、`TopBar.tsx`（视图菜单开关）、`FileBrowserView.tsx`（进包、“预览压缩包”、只读提示、空态、包内不取缩略图）、`useBrowserPane.ts`（上级选中原包）、`menu/registry.ts`、`MainContent.tsx`（`emptyState` / `readOnlyNote`）、`data/nfsp/readers.ts`（`archive://` reader；包内路径列表不走客户端缓存） |
| T8 | 部分 | 单测、集成测试、本机浏览器验证、文档；真实 Zone、真机、MessageHub 端到端、基准未做（§8.4） |

新增文件 `archive/password.rs`（§4 只列了四个文件，密码逻辑单独成文件更清楚）。新增依赖 `chardetng 1.0.0`、`encoding_rs`（workspace 直接依赖，owner 已确认）。

### 8.2 §1 / §2 之外的取舍

- **重复条目**：`zip` crate 的索引按解码后的名字去重，同名记录只留最后一条。所以中央目录自己遍历一遍（原始名字节、标志、0x7075、0x9901、0x5455、ZIP64），按 crate 的键映射到 crate 的下标；同一个 crate 键对应多条记录的，全部视为不可读，路径标为 `AMBIGUOUS_ENTRY`。
- **目录重复**：显式目录项和隐式目录、或两条同名的显式目录项合并为一个目录，不算同名冲突；文件与目录同名、两个文件同名才是冲突。冲突目录下的条目也计入 `hidden_entries`。
- **成员身份**：`raw` = 按分隔符切出的原始段用 `/` 连接（反斜杠分隔的名字，分隔符位置按所选编码判定，CP932 的“表”`95 5C` 不会被拆）；带 0x7075 的条目用 crate 替换后的 UTF-8 字节。
- **`"preference"` 的缓存**：按编码缓存整棵树（每种候选编码最多建一次），不是每请求现解码；结果相同，开销更小。
- **请求上下文**：解析层是同步的，对象来源的鉴权（`may_read`）和落盘是异步的。实现为：控制面、数据面、copy 在进入同步解析前扫描 `at` / `args` 里的对象 ref、`na_` / `no_` 句柄和含 `/@/` 的路径，做鉴权、落盘、建索引（`spawn_blocking`），结果放进 task-local 上下文；同步解析只查缓存。原生路径没预备到时同步建索引兜底。
- **加密成员的缓存按用户分开**：ZipCrypto 的快速校验只有 1 字节，误判的密码不能拿到别人已解出的明文；各用户自己的物化在 CRC 通过后才可见。命中缓存前同样先过密码检查（集成测试覆盖“忘记密码后不能再读缓存”）。
- **ETag**：路径来源用弱校验值，对象来源强校验值并带 `immutable`；projection 外层用 binding etag。
- **DOS 时间**：按服务器本地时区解释（unix 用 `mktime`，与压缩工具的显示一致；Windows 退回按 UTC）。有 0x5455 扩展时间戳时用它。
- **copy 执行时的对象鉴权**：执行时没有请求 token，按任务创建者 (user, app) 的 holding 检查，zone owner 放行；提交时仍走完整的 `may_read`。
- **projection 外层**：projection 文件的内容按对象落盘（与对象来源共用），`na_` 句柄里带 obj_id 和 etag，etag 变了即 `STALE`。
- **Desktop 归档 URL**：对象来源的目录在 Files 数据层用 `archive://<objId>/archive/...`（只在 Preview 里用）；路径来源仍是 `dfs://`。客户端判断“进包边界”用“`@` + `archive` 紧跟一个 `.zip` 名字”，服务端仍以“紧跟普通文件”为准。
- **Files 包内列表不走客户端缓存**：包内目录的容器 ref 随外层版本变化，stale-while-revalidate 的失效通知匹配不到，外层被替换后会一直显示旧列表；服务端有索引缓存，直接重列开销很小。
- **Preview 会话内翻到 zip**：自动进入它的目录页，从包根返回时回到进包前的那一项（“回到打开它之前的那一项”）。
- **缩略图**：包内条目不取缩略图（图标视图会为整页成员触发解压 / 物化）。
- **路径来源 `STALE`**：Files 和 Preview 按路径浏览目录，外层被替换后看到的就是新包（live 语义），不会出现 `STALE`；`STALE` 只发生在 `na_` 句柄上（例如打开着的成员），Preview 显示“压缩包已变化 / 重新打开”。
- **MessageHub**：附件来源是 `messagehub-object`，Preview 用它的 resolver 的 `pipelineObjectId()`（`msg.prepare_save_attachment`，把对象 holding 授给当前用户经当前 App）拿到可读的 obj_id，再按对象来源打开；没有 Zone 账号或尚未发送的附件按原样显示。
- **mock 运行时**：没有 `hello.archive`，压缩包按原来的方式交给 Preview。

### 8.3 样本实测

- **压缩方法**（`zip.rs` 单测）：Stored、Deflate、BZip2、Zstd、XZ 由 zip crate 写出后解码一致；**LZMA 用 Python zipfile 写的成员解码失败**（`LzmaError: LZ distance … beyond output size`），所以 LZMA 不进 `hello.methods`，读取时返回 `UNSUPPORTED{reason:"method"}`；Deflate64 本机没有能写它的工具，未验证，不对外宣告，但读取时仍交给 crate 尝试。
- **加密**：ZipCrypto（macOS `zip -P`，浏览器验证；zip crate 写出，集成测试）、AES-256（集成测试）。错误密码 `PASSWORD_INVALID`，正确密码可读，记住后新会话可读，其他用户不可读，忘记后不可读。
- **文件名**：UTF-8 标志、无标志 UTF-8、GBK / Big5 / Shift_JIS / EUC-KR 长名、GBK 短名的偏好与回退、CP437 西文、反斜杠分隔、CP932 含 0x5C（表、ソ、十）、不安全 / 同名 / 符号链接 / 文件与目录同名，见 `names.rs` 单测；真实 GBK 包在浏览器里按 `detected` 正确显示（浏览器语言 en-US）。§2.7 的阈值未用真实样本复核。
- **其他**：空包、损坏包（`ARCHIVE_CORRUPT`）、条目上限（`QUOTA_EXCEEDED`）、声明大小大于 / 小于实际（`ARCHIVE_CORRUPT`）、外层 rename 替换后旧句柄 `STALE` 而旧描述符仍读旧字节、名叫 `@` 的成员和真实名叫 `@` 的目录。ZIP64、成员数超过 65535、4 GiB 成员、分卷、真实 Windows / PowerShell 打的包没有样本。

### 8.4 验证

- `cargo test -p nfs_server -- --test-threads=1`：除 `tests/preview.rs` 的 `stills_go_through_imagemagick`、`videos_remux_or_transcode_by_runtime_hint` 外全部通过；这两个失败是本机缺 `darktable-cli` 和 ffmpeg 的 zscale 滤镜，与本改动无关。新增：`archive` 单测 18 个、`tests/archive.rs` 7 个。
- websdk：`tests/archive_refs.test.ts` 与相关的 `content_open`、`conversation_embed`、`object_url` 测试通过；`pnpm build` 通过。
- Desktop：`pnpm check`（`tsc -b`）通过；改动文件 `eslint` 无错误（仓库里已有 12 个错误都在未改动的文件）；`node --experimental-strip-types tests/datamodel/archive-urls.test.ts` 通过。
- 浏览器（独立 nfs_server + `VITE_NFS_PROXY` 的 Desktop，`?fbData=nfsp`）：Files 进包、面包屑、只读提示、隐藏条目提示、上级回到原目录并选中原包、空包、GBK 名字；Preview 打开 zip 的目录页、进子目录、打开 Deflate 文本和 Stored / Deflate 图片、同目录上一项 / 下一项、返回链、成员菜单、加密包的密码框（错误 / 正确）、损坏包的错误页与“作为文件打开”、会话内翻到 zip 后进入与返回。
- **未验证**：真实 Zone（含 Preview 走 NFSP 时的 principal 与读 `/ndm/...` 时一致）、MessageHub 收发 zip 的端到端、对象来源的浏览器界面（只有集成测试）、保存到 Zone 的 copy 任务（需要 TaskMgr）、手机真机与系统返回、Windows（`seek_read`、空闲关闭描述符）、基准和 T2 初值调整、Playwright 用例。

### 8.5 手册检查（AGENTS.md cmd_skills 表）

不需要修改：没有新增 CLI、设置项或运行路径；成员地址就是普通的 `cyfs:///<文件>/@/archive/...` 路径，`ui_core` 的 `read` 与 `buckyos obj read`（DFS 对象走 nfs_server 的 `resolve_for` / `node_info` / `list_node`）原样可用；`buckyos_admin` 只记录 `hello.repr` 这类可调设置，归档上限目前只在 `ServerConfig` 默认值里，不是 system-config 设置。
