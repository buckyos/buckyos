# WebSDK 标准对象实现收敛 TODO

> 状态：开发完成，三个仓库都未提交，见第 9 节。现状核实于 2026-11-06（buckyos `070d73afb`、buckyos-websdk `b25937d`、cyfs-ndn `eafd667`），下文行号以这些提交为准。
>
> 背景：2026-09-30 实现 MsgObject v2（cymsg breaking change）时发现，TypeScript 侧没有统一的标准对象实现。Desktop 手写了一份 `msgobj.ts` 镜像，WebSDK 的 `MsgObject` 只是 `Record<string, unknown>`。
>
> 目标：CYFS 标准对象的 TypeScript 实现统一放在 buckyos-websdk，前端只从 `buckyos` 包引用，不再各自维护协议镜像。同时修复 FileObject 常用 meta 缺失的 bug：构造 FileObject 时就写入常用 meta，不再放在对象之外。
>
> 协议权威：`cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md`（§5.2 JWT、§16 MsgObject、§17 ReceiptObj），参考实现是 `cyfs-ndn/src/ndn-lib`。Rust 与其它来源冲突时以 Rust 为准。

## 1. 现状

下表路径中，SDK 指 `buckyos-websdk/src/`，Desktop 指 `buckyos/src/frame/desktop/src/app/messagehub/`。

| 对象 / 能力 | Rust | SDK | 其它位置 |
| --- | --- | --- | --- |
| ObjId / ChunkId、canonical JSON、ObjId 计算 | 有 | 有 | — |
| `cyfile`、`cyinc`、`cyrel`、`cydir`、`cymap`、`clist` | 有 | 有 | — |
| `cypath` PathObject | `fileobj.rs:69`：`path, iat, target, exp, host?` | **已落后**：仍是 `uptime`，没有 `host`（`ndn_types.ts:1043`）。Rust 在 2026-04-19（`a56decf`）改的字段 | SDK 之外没有调用方 |
| `cymsg` MsgObject v2 | `msgobj.rs` | 只有常量；`msg_center_client.ts:31` 是 `JsonObject`，也没有从包入口导出 | Desktop `protocol/msgobj.ts`（610 行），被 src 48 个、tests 7 个文件引用 |
| `cyrece` ReceiptObj | `msgobj.rs:653` | 没有；`OBJ_TYPE_MSG_RECE = 'cymsgr'` 已过时（`ndn_types.ts:60`），无其它引用 | — |
| `cyact` ActionObject | `action_obj.rs` | 只有常量；`repo_client.ts:32` 的 `RepoActionProof` 是 `Record<string, unknown>`，而 Rust 里它是 `ActionObject` 的别名 | — |
| `pkg` PackageMeta | `package-lib/src/meta.rs` | 只有常量；`provision.ts:899,928` 有归一化和 ObjId 计算，但在 Node 专用模块里 | CLI `pikg_protocol.ts` 有手写字段校验 |
| JWT 形式的 NamedObject（§5.2） | `object.rs:400,433` | `loadNamedObjectFromObjStr` 抛 `Unsupported`（`ndn_types.ts:852`），`SimpleMapItem` 的 JWT 分支同样（`:1215`）。`namelib.ts` 已有 `decodeJwtClaimWithoutVerify`、`decodeJwtHeaderWithoutVerify`、`verifyJwtEdDSA`、`signJwtEdDSA`，且只依赖 `./types`，可直接复用 | Desktop `guest/model.ts:73` 手写了 `decodeNamedObject` |
| MsgObject 校验与 JWT（§16.5） | `MsgObject::validate`、`from_json_value_checked`、`decode_msg_object_jwt`、`verify_msg_object_jwt` | 没有 | Desktop 有 `isValidMsgSessionId`、`randomMsgNonce` |
| dispatch 编码 | `cyfs_http.rs:116-175` | 没有 | Desktop `decodeNamedObject` 靠 `contentType.includes('jwt')` 判断 |
| System Notify 类型 | `buckyos-api/src/msg_notify.rs` | 已有并已导出（`msg_center_client.ts:601` 起，`task_mgr_client.ts:537` 起） | Desktop `protocol/msgobj.ts:392-534` 重复定义 |
| FileObject 常用 meta | `fileobj.rs:23` 是 flatten 的 `meta`；《CYFS 标准对象》§10 的示例用 `mime` | **有 bug**：`ndm.importFiles` 构造时不写任何 meta（`ndm_client.ts:1015,1036,1088,1103,1117`） | HomeStation 把展示字段存在本地表 `import_metadata`，不随内容分发。`mime` 和 `mime_type` 混用，有的读取方只认其中一个，见 3.6。全仓没有 EXIF 解析；filebrowser 的 `FileExif`（`types.ts:55`）靠 enrich 扩展提供 |

其它事实：

- **fixture 不能当作实现完成的证据。** `tests/ndn_object.json` 的 `cypath`、`cymsgr` 都是旧对象。`cyact`、`cymsg`、`cymsgr` 没有 TS 解码器，只校验原始 JSON 的哈希，所以现在 jest 全过。
- **`MsgReceiptObj` 不是 `cyrece`，不要改。** `msg_center_client.ts:191` 是 msg-center 读回执 RPC 的 DTO，对应 `buckyos-api/src/msg_center_client.rs:1212`，没有实现 `NamedObject`。
- **Desktop 不依赖 npm 发版。** 依赖是 `git+https://github.com/buckyos/buckyos-websdk#main`。`.pnpmfile.cjs` 调用 `src/tools/pnpm_local_sdk.cjs`，存在同级 SDK 时改用 `link:`（可用 `BUCKYOS_SDK_TOOL_SOURCE` 指定位置）。
- **SDK 的 `dist/` 和 `cli/dist/` 纳入了 Git。** Desktop 的类型来自 `dist/index.d.ts`，所以改完 SDK 源码必须先 `pnpm run build`，Desktop 才能看到新类型。

## 2. 已确认的决定

| # | 决定 | 做法 | 原因 |
| --- | --- | --- | --- |
| 1 | 新对象放哪 | 放 `ndn_types.ts`，经已有的 `ndn` 命名空间导出（`ndn.MsgObject`），不加顶层导出 | 三个入口都已 `export * as ndn`，Desktop 也已在用 `ndn.FileObject` |
| 2 | 新对象的写法 | `MsgObject`、`ReceiptObj`、`ActionObject`、`PackageMeta` 都用 interface 加函数，不用 `NamedObjectBase` class | 它们以普通 JSON 在 RPC 和 React state 里流动，Desktop 还会展开并附加 `ui_*` |
| 3 | `agent_task`、`agent_ask`、`voice` | 类型和读取函数留在 Desktop，SDK 的 `MsgObject` 只用索引签名承载 | 它们是 OpenDAN 与 MessageHub 之间的约定，不在《CYFS 标准对象》里 |
| 4 | Notify | 类型和常量改用 SDK 已有的；`messageNotify`、`parseHumanTaskPrompt` 和 `NOTIFY_CATEGORIES/LEVELS/STATUSES` 留在 Desktop | SDK 的 `notifyMessageData` 只做类型断言，Desktop 的版本会清洗不可信数据 |
| 5 | 验签 | 提供 `verifyMsgObjectJwt`，不提供 TS 侧签名 | `namelib.verifyJwtEdDSA` 已有，不加依赖，还能完整移植 JWT 用例。Desktop 暂无调用点 |
| 6 | fixture 范围 | 覆盖 `cyfile cypath cyinc cyrel cyact cymsg cyrece pkg` 共 8 种 | 这是 Rust 回环测试实际输出的范围；`cydir`、`cymap` 不在其中，保留现有 TS 用例 |
| 7 | HomeStation 的 `FileObject`（`src/frame/desktop/src/app/homestation/protocol/feed.ts:74`） | 删除这个投影类型，后端直接返回标准 `cyfile` JSON，Desktop 用 `ndn.FileObjectJson` | 常用 meta 写进对象之后，就不再需要投影 |
| 8 | 校验调用点 | 这次不在 Desktop 新增校验调用，只迁移实现 | 保持收发行为不变；接入校验可单独提 |
| 9 | 与 Rust 同名但取值不同的常量 | 这次一起改，见 3.1 | 只有 `ndn_types.ts` 内部在用，外部没有引用，改动很小 |
| 10 | 常用 meta 的范围 | `mime`、尺寸、时长、编码、音频标签、拍摄时间、地理位置、相机、曝光参数，键表见 3.5，写进《CYFS 标准对象》§10 | 媒体文件的价值就在这些信息里，尤其是照片的 EXIF |
| 11 | mime 的键名 | 统一为 `mime`，删除 `mime_type` 的读写，不做兼容 | §10 示例、fixture、`bucket_store`、nfs 快照、HomeStation 都用 `mime` |
| 12 | 谁写 meta | 谁构造 FileObject 谁写，都用通用 helper：TS 在 SDK，Rust 在 `ndn-lib`。例外见 3.6 | 放在各调用方就会再次漏写、键名再次分叉。HomeStation、MessageHub、filebrowser、aiworkspace 的上传都走 SDK 的 `importSelectedFiles` |
| 13 | 已有对象 | 不迁移，缺字段就按缺失处理 | 开发阶段不做兼容 |
| 14 | 隐私 | 这次不考虑。所有入口都写全部字段，不设开关，发布时也不清理 | BuckyOS 是私有存储，原样保存对用户更重要。这与 `product/control_panel/ShareContentMgr_PRD.md:275`（设备信息和非必要 EXIF 不自动公开）不符，以后单独处理 |
| 15 | 解析实现 | TS 和 Rust 都手写白名单解析，不加依赖 | 只需要十几个标签，一次性原型在 3 个真实样本上的结果和 `exifread` 完全一致。可选的库里，`exifr` 是 MIT 但 2022 年后没有更新，`exifreader` 在维护但是 MPL-2.0，而且 Rust 侧还要另选一个 |
| 16 | 尺寸和时长的来源 | 优先解析字节，解析不了才在浏览器里用媒体元素补 | 不用解码，批量导入没有额外开销；TS 和 Rust 结果一致，ObjId 不再因浏览器而异 |

决定 7、10–13 的背景：展示字段不进 FileObject 是 bug。FileObject 这一层就是用来放 meta 的，构造时应该把常用 meta 都写进去。现在浏览器上传的展示字段只存在本用户的 `import_metadata`（`publish.rs:909`），不随内容分发，所以关注者拿不到 mime、尺寸和时长。架构文档（`doc/homestation/BuckyOS HomeStation 架构设计.md:371`）的写法是对的，协议文档（`doc/homestation/HomeStation 协议与实现.md:59`）要改。

影响：相同字节但 meta 不同的文件，FileObject 的 ObjId 不同，和 `name` 不同时一样。只有靠浏览器补的字段（如 WebM 的时长）才可能因浏览器而异。chunk 层的去重不受影响。

## 3. 待办

建议按小节顺序做，只有一处例外：Rust 是参考实现，3.6 的 `ndn-lib` helper 和测试向量要先于 3.5 的解析用例。3.1–3.5 在 buckyos-websdk；3.6 先改 cyfs-ndn（helper 和其中的构造点），再改 buckyos；3.7、3.8 主要在 buckyos。

MsgObject 收敛（3.1–3.4 和 3.7 的 `msgobj.ts` 部分）和 FileObject meta（3.5、3.6 和 3.7 的 HomeStation、MessageHub 部分）互不依赖，可以分成两批提交。

### 3.1 SDK：修正已落后的内容，重新生成 fixture

- [x] `PathObject` 改为 `iat`，增加可选的 `host`（为空时省略），同步构造函数、`fromJSON`、`toJSON` 和 `tests/ndn_types_cases.ts:670,692,825-831`。
- [x] `OBJ_TYPE_MSG_RECE` 改名为 `OBJ_TYPE_RECEIPT`，值为 `cyrece`，与 Rust 同名。
- [x] 其它 `OBJ_TYPE_*` 常量按 `cyfs-ndn/src/ndn-lib/src/lib.rs` 对齐：

  | SDK 现在（`ndn_types.ts:67-78`） | 改为 |
  | --- | --- |
  | `OBJ_TYPE_OBJMAP = 'cymap-mtp'`、`OBJ_TYPE_OBJMAP_SIMPLE = 'cymap'` | `OBJ_TYPE_OBJMAP = 'cymap'` |
  | `OBJ_TYPE_LIST = 'cylist-mtree'`、`OBJ_TYPE_LIST_SIMPLE = 'cylist'` | `OBJ_TYPE_LIST = 'cylist'` |
  | `OBJ_TYPE_CHUNK_LIST = 'cl'`、`OBJ_TYPE_CHUNK_LIST_SIMPLE = 'clist'` | `OBJ_TYPE_CHUNK_LIST = 'clist'` |
  | `OBJ_TYPE_TRIE`、`OBJ_TYPE_TRIE_SIMPLE`、`OBJ_TYPE_CHUNK_LIST_FIX_SIZE`、`OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE` | 删除，Rust 已注释掉 |

  同步 `isChunkList`、`isContainer`、`isBigContainer`，判断规则同 `object.rs:93,116,127`，并加用例。现在 `isBigContainer` 对 `clist`、`cymap`、`cylist` 返回 `false`，而 Rust 返回 `true`；TS 侧目前没有调用方，所以没有实际影响。需要改的引用只有 `ndn_types.ts:563,582-604,1568,1572,1732` 和 `tests/ndn_types_cases.ts:46,180,1005`；SDK 的 `src`、`tests`、`cli/modules` 和 Desktop、`src/tools`、`test`、`src/apps` 都已搜过，没有其它引用。
- [x] 重新生成 `tests/ndn_object.json`，并删除 `tests/ndn_types_cases.ts` 中关于 `cymsgr` 的注释。在 `cyfs-ndn/src/` 运行：

  ```bash
  # 打印 7 项的数组（输出夹在测试日志里，需要提取）
  cargo test -p ndn-lib --lib test_non_chunk_named_object_file_roundtrip -- --nocapture
  # 打印单个对象，追加为第 8 项
  cargo test -p package-lib --lib test_package_meta_file_roundtrip -- --nocapture
  ```

  已实跑确认的预期结果：`cyfile`、`cyinc`、`cyrel`、`cyact`、`cymsg` 不变；`cypath` 变为 `cypath:b521d401…`；`cymsgr` 被 `cyrece:f7564f5b…` 取代；新增 `pkg:368aa598…`。

### 3.2 SDK：MsgObject v2

- [x] 在 `ndn_types.ts` 增加类型：`MsgObject`、`MsgObjKind`、`TopicThread`、`MsgRelation`、`MsgRelType`（保留未知值）、`MsgMentions`、`MsgContent`、`MsgContentFormat`、`MachineContent`、`CanonValue`、`RefItem`、`RefTarget`、`RefRole`。可以从 Desktop `protocol/msgobj.ts:23-149,211-239` 搬过来，已对照 Rust 确认字段一致。`meta` 是 flatten 字段，用索引签名表示。`created_at_ms` 沿用 Desktop 的必填写法，避免 Desktop 到处判空；校验入口仍接受缺失。
- [x] 增加校验：`validateMsgObject`、`msgObjectFromJsonChecked`（对应 `from_json_value_checked`）、`isValidMsgSessionId`、`MSG_OBJECT_RESERVED_KEYS`、`MSG_SESSION_ID_MAX_CHARS`、`MSG_REACTION_KEY_MAX_BYTES`、`randomMsgNonce`。规则见第 4 节。
- [x] `msg_center_client.ts` 的 `MsgObject` 改为 `ndn_types.ts` 的类型，并去掉 `msgMachineData` 等处多余的 `as JsonObject`。
- [x] 移植测试，对应关系见第 4 节末尾。

### 3.3 SDK：ReceiptObj、ActionObject、PackageMeta

- [x] 增加 `ReceiptObj`、`ReceiptStatus`（`accepted | rejected | quarantined`）。
- [x] 增加 `ActionObject`、`ACTION_TYPE_*` 常量。`repo_client.ts` 的 `RepoActionProof` 改为 `ActionObject`，并更新该处“保持 opaque”的注释。
- [x] 增加 `FileObjectJson` interface，表示 `cyfile` 的普通 JSON：`name?`、`size?`、`content?` 等基础字段，3.5 键表里的所有字段（都可选），再加索引签名。现有的 `FileObject` class 不动。
- [x] 增加 `PackageMeta`（`FileObjectJson` 加 `version`、`version_tag?`、`deps?`）。把 `normalizePackageMetaForObjId`、`calcPkgMetaObjId` 移到 `ndn_types.ts`，让浏览器也能用，`provision.ts` 改为引用它们。`tests/provision.test.ts:403` 必须继续通过。
- [x] 把 `cyact`、`cymsg`、`cyrece`、`pkg` 注册进 `tests/ndn_types_cases.ts:172` 的 `TEST_NAMED_OBJECT_DECODERS`，让它们走“解码再编码”的路径，而不是只校验哈希。

### 3.4 SDK：JWT 与 dispatch

- [x] `loadNamedObjectFromObjStr` 按 Rust 改写（`object.rs:433`）：去掉开头空白后以 `{` 或 `[` 开头按 JSON 解析，否则按 JWT 取 claims。现在的 `indexOf('{') >= 0` 判断不对。
- [x] 增加 `buildNamedObjectByJwt`、`verifyNamedObjectFromJwt`，并实现 `SimpleMapItem` 的 `objectJwt` 分支。ObjId 只由 claims 计算。
- [x] 增加 `decodeMsgObjectJwt`、`verifyMsgObjectJwt`、`msgJwtKidDid`。header 必须 `alg = EdDSA` 且 `kid` 非空，claims 走 `msgObjectFromJsonChecked`。
- [x] 增加 `CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON`、`CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT`、`namedObjectEncodingFromContentType`、`isCyfsNamedObjectContentType`。判断规则同 `cyfs_http.rs:167-175`：去掉 `;` 后的参数，大小写不敏感。

### 3.5 SDK：通用 helper，上传时写入常用 meta

常用 meta 的键如下。取不到就省略，空对象也不写。TS 和 Rust 的 helper 都按这张表实现：

| 键 | 类型 | 适用 | 来源 |
| --- | --- | --- | --- |
| `mime` | string，不带参数 | 全部 | 媒体文件以 magic bytes 为准，其它文件依次用调用方声明的类型、扩展名。判断不出就省略，不写 `application/octet-stream` |
| `width`、`height` | 整数 | 图片、视频 | 显示尺寸，已应用旋转，所以不另存 orientation |
| `duration_ms` | 整数 | 音频、视频 | MP4、MOV、m4a 的 `mvhd`，FLAC 的 `STREAMINFO`，WAV 的 `fmt`、`data` |
| `video_codec`、`audio_codec` | string | MP4、MOV、m4a | `stsd`：`avc1`/`avc3` → `h264`，`hvc1`/`hev1` → `hevc`，`av01` → `av1`，`vp09` → `vp9`，`mp4a` → `aac`，其它省略 |
| `title`、`artist`、`album` | string | 音频 | ID3v2 的 `TIT2`/`TPE1`/`TALB`，m4a 的 `©nam`/`©ART`/`©alb` |
| `captured_at` | ISO 8601 string | 照片、视频 | `DateTimeOriginal`，缺失时用 `DateTimeDigitized`，再加 `SubSecTimeOriginal` 和 `OffsetTimeOriginal`。没有时区就不带偏移，读取方不能当 UTC。不用 `DateTime`，它是修改时间 |
| `geo` | `{ lat, lon, alt?, heading?, accuracy_m? }`，WGS84 | 照片、视频 | EXIF GPS（含 `GPSImgDirection`、`GPSHPositioningError`）、MOV 的位置元数据。`lat`、`lon` 保留 6 位小数，其余保留 1 位 |
| `camera` | `{ make?, model?, lens? }` | 照片、视频 | EXIF `Make`、`Model`、`LensModel`，MOV 的 make、model |
| `exposure` | `{ iso?, f_number?, exposure_time?, focal_length_mm?, focal_length_35mm? }` | 照片 | EXIF。`exposure_time` 是约分后的分数字符串如 `"1/120"`，整数秒写 `"2"`。`f_number`、`focal_length_mm` 保留 2 位小数 |

一次性原型跑过 `media_samples/` 的 3 个真实样本（Photoshop 存过的 Sony JPEG、iPhone JPEG、iPhone HEIC），结果和 `exifread` 全部一致。样本暴露的要点：

- **JPEG 有多个 APP1（Exif 和 XMP）**，要按 `Exif\0\0` 前缀找，而且不一定紧跟 SOI（iPhone 的第一段是 APP0）。
- **HEIC 的 Exif 不在文件头。** 它在 `mdat` 里（样本偏移约 131 KB），要经 `iinf`、`iloc` 定位。item 数据开头 4 字节是到 TIFF 头的偏移（样本是 6）。主图是 `grid` item，`ispe`、`irot` 要按 `pitm` 指向的 item 取，样本里有 100 多个 tile item。
- **HEIC 不能叠加 EXIF 方向。** 样本 `ispe` 是 5712×4284、`irot` 是 3、EXIF Orientation 是 6，显示尺寸是 4284×5712，两个都应用会转回去。只有 JPEG 才看 EXIF Orientation（5–8 交换宽高）。
- **`DateTime` 不能当拍摄时间。** Photoshop 存过的样本里它和 `DateTimeOriginal` 相差约 50 分钟，也没有时区。
- **字节序两种都有**（Sony 小端，iPhone 大端）。
- **对象不会明显变大。** meta 的 JSON 约 300–400 字节。

- [x] 把探测能力移进 SDK。`sniffMagic`、`stripMediaTypeParams`、`heifImageSize` 和 `isoBoxes` 从 Desktop `src/frame/desktop/src/components/preview/mediaTypes.ts` 移到 `src/content_mime.ts`，经 `buckyos/content` 导出（`extensionOf` 已是这个模式），Desktop 改为引用。
- [x] 增加通用 helper `src/file_meta.ts`，经 `ndn` 导出：`probeFileMeta(source, { name?, declaredMime? })` 返回上表字段，`buildFileObject(name, size, content, meta)` 返回带 meta 的 `FileObject`。`source` 是 `Blob` 或 `{ size, readAt(offset, length) }`。
- [x] 实现字节解析。只按需读取，不把整个文件读进内存，单个文件的总读取量要有上限（建议 8 MiB），超过就放弃这一组字段：
  - **EXIF：** JPEG 的 APP1，HEIC、HEIF、AVIF 的 `Exif` item，PNG 的 `eXIf`，WebP 的 `EXIF`，TIFF 及基于 TIFF 的 RAW（DNG、CR2、NEF、ARW）直接读 IFD。只读上表用到的白名单标签。
  - **尺寸：** JPEG 的 SOF，PNG 的 IHDR，GIF，WebP，HEIF 的 `ispe` 加 `irot`，MP4、MOV 的 `tkhd` 加旋转矩阵。RAW 取不到就省略。
  - **视频（MP4、MOV）：** `moov` 可能在文件末尾，要按顶层 box 跳着读，并跳过 `stbl` 里的大表。时间取 `com.apple.quicktime.creationdate`，缺失时取 `mvhd`（UTC）；位置取 `com.apple.quicktime.location.ISO6709` 或 `©xyz`；相机取 `com.apple.quicktime.make`、`model`。
  - **音频：** 上表的标签和时长。
- [x] 字节解析不出的尺寸或时长（如 WebM、MP3）才用媒体元素补，沿用 Desktop `app/homestation/api/store.ts:76` 的 `loadMedia` 并加上超时。没有 DOM 的环境（Node、Deno、Worker）直接省略。
- [x] 解析的是不可信的二进制输入，要处理这些情况：
  - 偏移越界和循环引用。任何解析错误只丢弃这一组字段，不能让导入失败。
  - 分母为 0 的 rational、无效日期（如 `0000:00:00 00:00:00`）直接省略。
  - `lat`、`lon` 超出范围，或恰好是 `(0, 0)`，都省略。
  - 字符串去掉 NUL 和控制字符，trim 后最长 128 个字符。
  - **`NaN`、`Infinity`、`-0` 和孤立代理对会让 `genObjId` 抛错**（`ndn_types.ts:243-248`），所以写入前要过滤。
  - **取整规则必须和 Rust 一致：** 对绝对值四舍五入再恢复符号，结果是 `-0` 时归一成 `0`。`Math.round` 和 Rust 的 `round` 处理负数 `.5` 的方向不同，直接用会算出不同的 ObjId。
- [x] `importFiles`、`importDirectory`、`pickupAndImport` 都走 `importSelectedFiles`，在这里统一处理：QCID 查找之前探测，并把结果传给 `materializeFile`、`materializeFileViaWorker`、`buildFileObjectFromContentId`，让所有 `new NdnFileObject(...)` 都带上 meta。
- [x] 增加选项 `fileMeta`，接收调用方已有的探测结果。HomeStation 校验附件类型时不用探测两次，MessageHub 也靠它保留转码前的 EXIF（见 3.7）。
- [x] **改掉 `ndm_client.ts:1513` 的重建逻辑。** 它在 QCID 命中时按 `name/size/content` 重建对象来取 `obj_data`，不带 meta 就会和 `objectId` 对不上。
- [x] `ImportedFileObject.mimeType` 改取探测结果，不再用 `file.type`。
- [x] 更新写死 ObjId 和 mimeType 的用例：`tests/ndm_client.test.ts:211`、`tests/ndm_client_cases.ts:147,178,577,578,841,854`。在 `tests/browser/real-browser/ndm_client.spec.js` 加真实浏览器用例，验证媒体元素的回退。
- [x] 增加解析用例，用 3.6 生成的向量（`tests/media_meta_vectors.json`），再覆盖样本里没有的情况：EXIF Orientation 为 6 的 JPEG、南纬西经、负海拔、畸形输入。视频可以用 ffmpeg 给 `src/frame/nfs_server/tests/fixtures/preview/h264_aac.mov` 加 `location` 和 `creation_time`。
- [x] 增加读真实样本的用例：目录取环境变量 `BUCKYOS_MEDIA_SAMPLES`，默认是同级的 `media_samples/`，不存在就跳过。**样本含真实位置和时间，样本本身和由它得出的预期值都不能提交。**
- [x] `pnpm run build`，让 `dist/` 和 `cli/dist/` 包含 3.1–3.5 的改动。

### 3.6 服务端：Rust helper，统一键名，HomeStation 改用 cyfile

本节的 cyfs-ndn 路径相对 `cyfs-ndn/src/`，其余相对 buckyos 的 `src/`。cyfs-ndn 要先进入 main，buckyos 才能用上 helper（`cargo update -p ndn-lib`）；本地联调可以临时用 `[patch]`，不要提交。

- [x] `ndn-lib` 增加 `media_meta` 模块，键和规则同 3.5：`probe_media_meta(source, name, declared_mime)` 返回 `MediaMeta`，`FileObject::apply_media_meta` 写入。`source` 提供异步的 `read_at`，内置内存和本地文件两种实现。不加新依赖；扩展名映射自带一张媒体类型小表，和 SDK 的 `content_mime.ts` 保持一致。
- [x] 输出测试向量，和 `tests/ndn_object.json` 一样以 Rust 为参考：`ndn-lib` 增加一个用例，打印 `[{ name, input, expected }]`（`input` 是 base64 编码的合成字节），存为 SDK 的 `tests/media_meta_vectors.json`，TS 必须全部通过。
- [x] 所有构造点改用 helper。调用方已知的信息作为声明传入，或在 helper 之后覆盖；键统一用 `mime`：

  | 位置 | 读字节的方式 | 备注 |
  | --- | --- | --- |
  | cyfs-ndn `bucket_store/src/content.rs:41` | 内存 | `mime` 参数作为声明，去掉 `charset` 等参数 |
  | cyfs-ndn `bucket_store/src/s3.rs:288` | 经 `NamedDataMgr` 读已存的 chunk（`chunk_list_reader.rs` 支持 seek） | `content_type` 作为声明，同样去掉参数 |
  | cyfs-ndn `ndn-toolkit/src/cyfs_ndn_dir_server.rs:994,1185` | 本地文件 | 先写 helper，模板用 `or_insert`，所以探测结果优先 |
  | cyfs-ndn `fs_buffer/src/local_filebuffer.rs:840` | `diff_file_path` | 名字是 `inode-N`，没有扩展名，`mime` 只能靠 magic bytes |
  | cyfs-ndn `fs_buffer/src/local_filebuffer.rs:561` | 合并后的 overlay | 读不到完整内容时写能确定的字段，并在提交说明里写明原因 |
  | `frame/nfs_server/src/share/snapshot.rs:285` | 本地文件 | `base_mime(name)` 作为声明 |
  | `frame/homestation/src/publish.rs:1100` | 内存 | `mime` 参数作为声明，`extra_meta` 在 helper 之后覆盖 |
  | `frame/aicc/src/resource/mod.rs:886` | 内存 | 同时把 `:38,2243` 的 `mime_type` 改成 `mime`，`spec.attributes` 在 helper 之后覆盖 |
  | `frame/msg_center/src/tg_tunnel.rs:675` | 内存（`attachment.data`） | `attachment.mime_type` 作为声明，写入键改成 `mime` |
  | `kernel/workflow/src/io.rs:48` | 调用方 `:165` 有内存字节，`:366` 按该处实际来源读 | `content_type` 作为声明，写入键改成 `mime`。结果只取决于字节，对象仍是确定性的 |
  | `frame/agent_tool/src/llm_understand_media/pipeline.rs:378` | 经 store 读已存的 chunk | `mime` 作为声明，写入键改成 `mime` |

- [x] 读取 FileObject 时只认 `mime`：`frame/agent_tool/src/read_tool.rs:118`、`frame/agent_tool/src/llm_understand_media/pipeline.rs:431`、`frame/msg_center/src/object_access.rs:411`、`frame/aicc/src/resource/mod.rs:1607`。
- [x] 以下的 `mime_type` 不是 FileObject 字段，**不要改**：`frame/aicc/src/protocol/` 下的第三方协议（`gemini.rs`、`fal_queue.rs`）、`frame/msg_center/src/tg_tunnel.rs:387,451` 的附件 DTO、SDK `ndm_client.ts:1755` 的 tus 元数据。
- [x] 这是决定 12 的例外，**不要加 meta**：`frame/control_panel/src/buckets.rs:151`、`frame/repo_service/src/service.rs:183`、SDK `cli/modules/app.ts:1111`、cyfs-ndn `package-lib/src/meta.rs:53`。这些是包内容，CLI 和服务端都按 `name/size/content` 算出同一个 ObjId，不是所有一端都有字节。
- [x] 给 HomeStation 加跨用户用例：在 `frame/homestation/tests/` 用 `Net::new(&["carol", "bob"], …)`，carol 发布带 `mime`、`width`、`height` 的 `cyfile` 图片，bob 看到的 `media[0].file` 要有这些字段，`/content` 的 Content-Type 要正确。bug 的根因是 SDK 不写 meta，由 3.5 的用例覆盖；这个用例防止 HomeStation 的读取路径再丢字段。
- [x] `frame/homestation/src/projection.rs`：删除 `file_view`，`local_file` 直接返回对象 body；`resources.rs:409` 同样处理。
- [x] 删除 `import_metadata`：表定义 `kernel/buckyos-api/src/homestation_client.rs:137`，以及 `frame/homestation/src/` 下的 `objects.rs:509`、`publish.rs:904-914,929-931,1142-1144`、`projection.rs:126`、`http.rs:360-361`。媒体类型校验和 Content-Type 改读对象的 `mime`。
- [x] `object.register` 去掉 `metadata` 参数（`frame/homestation/src/api.rs:154`、`publish.rs:876`）。
- [x] 加跨语言用例：带嵌套 meta 和浮点数（如 `geo.lat = 34.9949`）的 `cyfile`，Rust 反序列化成 `FileObject` 后算出的 ObjId，要等于 SDK 算出的值。
- [x] 更新 `frame/homestation/tests/reading.rs:11,66,130`、`sources.rs:109`、`object_import.rs:70,81,198`。

### 3.7 Desktop 及其它消费方

先确认 Desktop 能解析到新版 SDK：本地有同级仓库时走 `link:`，否则要等 SDK 合入 main。

本节的 `protocol/`、`guest/` 相对 Desktop 的 messagehub 目录，`tests/` 相对 `src/frame/desktop/`，其余相对 buckyos 仓库根目录。

- [x] `protocol/msgobj.ts` 按下表收敛，并改写文件头（现在还指向旧的 Mac 路径）。其它文件的协议类型直接从 `buckyos` 引入，不留转发层。

  | 内容 | 处理 |
  | --- | --- |
  | 3.2 列出的协议类型、`MSG_SESSION_ID_MAX_CHARS`、`isValidMsgSessionId`、`randomMsgNonce` | 删除，改用 `ndn.*` |
  | `DID` | 改用 `buckyos` 顶层导出的 `DID`（SDK `types.ts:1`） |
  | `ObjId`、`Uri`、`JsonValue` | 确认无外部引用后删除，内部用 `string` |
  | Notify 类型和常量：`NOTIFY_INTENT`、`NOTIFY_SESSION_PREFIX`、`NOTIFY_CHOICE_CONTROL_ID`、`NOTIFY_TEXT_CONTROL_ID`、`Notify{Kind,Category,Level,Status,Conversation,Source,MessageData}`、`HumanPrompt{Option,Control}`、`HumanTaskPrompt` | 删除，改用 SDK 已有的 |
  | `notifySessionId` | 换成 `notifyCategorySessionId` |
  | `notifySessionCategory` | 换成 `notifyCategoryFromSessionId`。注意它不匹配时返回 `null` 而不是 `undefined`，调用方要逐个检查 |
  | `MessageObject` | 保留，定义为 `ndn.MsgObject & { agent_task?: AgentTaskRef; agent_ask?: AgentAsk; voice?: VoiceNote }` |
  | `AgentTaskRef`、`AgentAsk*`、`AGENT_ASK_ANSWER_INTENT`、`VoiceNote`、`VOICE_WAVEFORM_*`、`messageObjId`、`messageAgentTaskId`、`messageAgentAsk`、`messageVoice`、`messageAgentAskAnswer`、`agentAskTarget`、`buildAgentAskAnswer`、`agentAskQuestionText` | 保留 |
  | `messageNotify`、`parseHumanTaskPrompt`、`NOTIFY_CATEGORIES`、`NOTIFY_LEVELS`、`NOTIFY_STATUSES` | 保留 |
  | `MessageDeliveryStatus`、`ConversationStatusType`、`getMessage*`、`isStatusMessageObject` | 保留。`ui_*` 字段经索引签名访问，不需要声明 |

- [x] `guest/model.ts:73` 的 `decodeNamedObject` 删除，`guest/api.ts:84` 改用 `ndn.loadNamedObjectFromObjStr`。
- [x] 同步 `tests/datamodel/messagehub-*.test.ts` 的 import（如 `messagehub-session.test.ts:4`）和 `messagehub-guest.test.ts:53-54` 的断言。
- [x] 删除 `src/frame/desktop/pnpm-workspace.yaml` 中已无意义的 `minimumReleaseAgeExclude: buckyos@0.7.116`。
- [x] `test/jarvis_media_dv/jarvis_media_dv.ts:66-85` 的本地 `RefItem`、`MsgObject` 改用 SDK 类型。其中 `source?` 不是 v2 字段，改之前确认是否还在用。
- [x] 检查其它 SDK 消费方是否因类型收紧而需要调整：`src/tools/buckyos-agent/lib/ask.ts`、`test/test_msg_center/`、`test/test_sys_test/support_group_dv.ts`、`test/jarvis_media_dv/scenarios.ts`。
- [x] HomeStation 改用标准 `cyfile`。在 `src/frame/desktop/src/app/homestation/` 下：删除 `protocol/feed.ts:74` 的 `FileObject`，`datamodel/types.ts`、`media.ts`、`MediaThumb.tsx`、`mock/` 改用 `ndn.FileObjectJson`；读取处从 `file.meta.x` 改成 `file.x`（`media.ts:55,71`、`card/parts.tsx:198,228`、`detail/ItemDetail.tsx:81`、`mock/media.ts:15-16`、`mock/data.ts:252-254`、`mock/store.ts:1058`）。
- [x] 同一目录下，`api/store.ts:76,92` 的 `loadMedia`、`uploadMeta` 和 `api/fileKind.ts` 的 `sniffFile` 改用 SDK 的 `probeFileMeta`，保留附件类型校验；`api/transport.ts:17,98,110,114` 精简 `UploadMeta`，`object.register` 不再传 `metadata`。
- [x] `src/frame/desktop/src/components/preview/ndmSource.ts:95`、`src/frame/desktop/src/app/messagehub/api/objects.ts:79` 改读 `mime`。
- [x] MessageHub 保留转码前的 EXIF。`src/frame/desktop/src/app/messagehub/api/upload.ts:30` 的 `normalizeImageFile` 会把 HEIC 转成 JPEG，EXIF 随之丢失。先探测原文件，把 `captured_at`、`geo`、`camera`、`exposure` 经 `fileMeta` 传给 `importFiles`；`mime`、`width`、`height` 仍取转码后的文件。

已扫描过 `src/`、`test/` 下的其它前端：除上述位置外，没有发现自定义的标准对象类型（只按名称匹配）。filebrowser、OpenDAN WebUI、node_active、control_panel 都没有命中。

### 3.8 文档

- [x] `cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md` §10 增加 3.5 键表里的定义，并写明原则：谁构造 FileObject 谁写 meta，用 helper。
- [x] `doc/homestation/HomeStation 协议与实现.md:59` 改为常用 meta 写进 `cyfile`，并同步 `object.register` 的描述；`doc/homestation/HomeStation Durable Data Schema.md:74,124` 删除 `import_metadata`。
- [x] 更新 `doc/sdk/扩展self-host-group.md:399`，MsgObject 等类型的出处改为 SDK。
- [x] 检查 `product/message_hub/MessageHub_Current_UI_Model_Data.md:218` 的描述是否仍然成立。

## 4. MsgObject 校验的移植规则

`validateMsgObject` 对应 `msgobj.rs:483`：

- `to_session` 存在时取值合法，且 `to` 只有一个目标。
- `mentions` 存在时不能为空（`dids` 为空且 `all` 不为真）。
- `relates_to.target` 的 ObjType 必须是 `cymsg`。
- `reaction` 必须带 `key`，长度 1–64 字节；已知的其它 `rel` 不能带 `key`；未知 `rel` 不检查 `key`。
- 不能出现保留键。

容易写错的地方：

- **`key` 按 UTF-8 字节计数**（用 `TextEncoder`），`to_session` 按 code point 计数（200 个）。
- **`trim()` 有差异。** JS 的 `trim()` 会去掉 U+FEFF，Rust 不会。Desktop 现有写法对 U+FEFF 开头的值判断与 Rust 相反，迁移时要修正。
- **`meta` 是扁平的。** TS 对象上没有独立的 `meta`，声明字段不可能混进去，所以保留键检查实际只会命中 `proof`。

`msgObjectFromJsonChecked` 对应 `msgobj.rs:545`：用原始值算 ObjId，解析并校验，再把值按 serde 规则归一化后重算 ObjId，两者不同就拒绝。归一化和解析要覆盖：

- **省略默认值：** 值为 `null` 的可选字段、空 `thread`、`created_at_ms: 0`、`content.content: ""`、`refs: []`、`machine.data: {}`、`mentions.dids: []`、`mentions.all: false`。比如 `workspace: null` 必须被拒绝。
- **`format` 别名：** `image/jpg`、`image/svg`、`video/avi`、`audio/mp3`、`audio/x-wav` 在 Rust 反序列化时会转成标准值，所以这些输入因不是 canonical 形式而被拒绝。
- **封闭枚举：** `kind`、`RefRole`、`RefTarget.type` 遇到未知值就拒绝。`rel`、`format` 是开放的，保留原值。
- **必填字段：** `from`、`to`、`kind`、`content`。
- **DID：** 必须是 `did:method:id` 形式。Rust 虽然接受主机名，但转换后不再 canonical，结果同样是拒绝。
- **ObjId 字段：** `thread.reply_to`、`relates_to.target`、`RefTarget.obj_id` 都要能解析。
- **大整数：** Rust 的 `nonce`、`created_at_ms`、`expires_at_ms` 是 u64，而 `JSON.parse` 超过 2^53 会丢精度并算出错误的 ObjId。这些字段不是非负安全整数时要报错。当前 Rust 发送方都在范围内（`lib_opendan/src/protocol/input.rs:795` 是 u32，`msg_center/src/group_service.rs:1393` 是序号）。这与 Rust 有意不同，只影响超出范围的值。

测试对应 `msgobj.rs` 的 `test_msg_v2_*`（第 1400 行起），外加 `test_msg_checked_rejects_canonicalization_errors`。以下几处不能照搬：

| Rust 用例 | TS 处理 |
| --- | --- |
| `test_msg_v2_proof_is_reserved` | 只移植 `proof` 部分。`meta.insert("to_session", …)` 在扁平结构下无法表达 |
| `test_msg_v2_jwt_form` | 用 `namelib.signJwtEdDSA` 和 Rust 同一把测试密钥（`msgobj.rs:1545-1546`）现签。“无效对象不签名”一段没有对应，因为 TS 不提供签名 |
| `test_msg_checked_rejects_canonicalization_errors` | `JSON.parse('1e400')` 得到 `Infinity`，应由 `canonicalizeJson` 拒绝 |

## 5. 约束

- 修改《CYFS 标准对象》时，同一次改动同步更新 SDK 的类型、校验和 fixture。
- canonical JSON 必须与 Rust `serde_jcs` 一致，否则 ObjId 不同。
- 反序列化保持宽松，校验由入口显式调用，与 Rust 一致，保证旧数据能读出。
- 不做向下兼容，不加新依赖（JWT 和验签复用 `namelib`，解析见决定 15）。
- 探测过程不发起任何网络请求，不做逆地理编码，`geo` 只存坐标。
- 新增 FileObject 构造点时必须用 helper（决定 12 的例外除外）。
- 跨仓顺序：SDK 改动（含 `dist/`、`cli/dist/`）先进入 buckyos-websdk 的 main，再合 buckyos 的改动。没有同级 SDK 的环境从 GitHub main 取 SDK，顺序反了 Desktop 的 `tsc` 会失败。
- cyfs-ndn 要改 `ndn-lib`（helper）、`bucket_store`、`fs_buffer`、`ndn-toolkit` 和文档。它要先于 buckyos 的 Rust 改动进入 main。
- 协议字段和类型的改动都不加兼容分支：`mime_type` 回退读取要删掉，HomeStation 的 `file.meta` 结构也不保留。
- `src/rootfs/libexec/buckyos-tool/` 是构建产物，不要手改。

## 6. 验证

基线是 2026-11-06 改动前实跑的结果，用来区分已有失败和新引入的失败。

| 位置 | 命令 | 基线 |
| --- | --- | --- |
| `buckyos-websdk/` | `npx jest tests/ndn_types.test.ts tests/msg_center_client.test.ts tests/provision.test.ts tests/namelib.test.ts tests/types.test.ts` | 5 suites，227 通过 |
| `buckyos-websdk/` | `npx jest tests/ndm_client.test.ts tests/ndm_picker.test.ts` | 2 suites，59 通过 |
| `buckyos-websdk/` | `pnpm test` | 未跑 |
| `buckyos-websdk/` | `pnpm run build` | 未跑 |
| `buckyos-websdk/` | `pnpm run test:browser:real`（需要真实环境） | 未跑 |
| `src/` | `cargo test -p homestation --test reading --test object_import --test sources -- --test-threads=1` | 12 通过 |
| `cyfs-ndn/src/` | `cargo test -p ndn-lib -p bucket_store -p fs_buffer -p ndn-toolkit -- --test-threads=1` | 未跑 |
| `src/` | `cargo test -p workflow -p agent_tool -p aicc -p msg_center -- --test-threads=1` | 未跑 |
| `src/frame/desktop/` | `pnpm check` | 通过 |
| `src/frame/desktop/` | `deno test --no-check --config tests/datamodel/messagehub.deno.json --allow-all tests/datamodel/messagehub-*.test.ts` | 92 通过，7 失败 |
| `src/frame/desktop/` | `pnpm exec playwright test --config playwright.homestation.config.ts` | 未跑 |
| `src/frame/desktop/` | `pnpm lint`、`pnpm build` | 未跑 |

注意：

- **7 个基线失败都在 `messagehub-guest.test.ts`，单独运行也复现，原因未查。** 其余 messagehub 用例 90 个全过。
- **这些失败会挡住对 `decodeNamedObject` 的验证。** 第一个用例在第 46 行就失败，走不到第 53–54 行的 JWT 断言。替换后要用别的用例覆盖，比如在 SDK 测 `loadNamedObjectFromObjStr`。
- **`verifyMsgObjectJwt` 的浏览器分支 jest 覆盖不到。** `namelib` 在没有 Node 运行时才走 WebCrypto，这部分如未实测要如实说明。
- **提交说明要按 `AGENTS.md` 的 cmd_skills 联动表**写明受影响的手册，或写明无需修改的依据。手册和 `doc/sdk/` 里已搜过没有 `mime_type`，但 AICC 产物的 meta 变了，要核对 `aigc_*`。
- **改 `agent_tool` 后要重建 paios 镜像**（`./build_aios --local-test`），否则容器里跑的还是旧版。
- **真实样本只覆盖照片。** `media_samples/` 有 Sony JPEG、iPhone JPEG、iPhone HEIC，TS 和 Rust 都要跑通，再用 `uv run --no-project --with exifread` 交叉核对。视频、音频和 Android 照片还没有真实样本，只能靠合成字节，这部分要如实标为未用真实文件验证。
- **已有对象不迁移**（决定 13）。测试 HomeStation 附件时要新上传，旧附件没有这些字段，开发 Zone 需要干净数据时用 `start.py --all --wipe-data`。
- 未执行的检查必须如实说明。依赖 Zone 的 DV（`test/jarvis_media_dv` 等）跑不了时标为未验证。

## 7. 本次不做

- **隐私处理：** 见决定 14。字节和 FileObject 里的 EXIF、位置都原样保存，不清理、不设开关。
- **filebrowser 面板改读 FileObject：** `FileExif` 继续由 enrich 扩展提供，`location` 是展示字符串，和 `geo` 含义不同。
- `validate_cyfs_dispatch_body` 的 TS 移植：这是服务端入口逻辑，TS 侧没有消费方。
- CLI `pikg_protocol.ts` 的手写校验：属于发布校验逻辑，不改。
- `cyfs-ndn/doc/CYFS Protocol/Content Network.md:473,587` 仍写着 `cymsgr`：这是 cyfs-ndn 的文档问题，另行修正。

## 8. 验收

- `ndn` 命名空间提供第 3 节列出的所有类型和函数，浏览器入口和 Node 入口都可用。
- SDK 的 `PathObject`、`OBJ_TYPE_*` 常量和 `isChunkList`、`isContainer`、`isBigContainer` 与 Rust 一致。
- `tests/ndn_object.json` 含 8 种对象，全部走解码再编码的路径，jest 按 fixture 重算 ObjId 全部通过。
- 第 4 节列出的 Rust 用例都有对应的 TS 用例，结果一致。
- Desktop 中不再有协议对象的类型定义，只保留决定 3、4 和第 3.7 节表格列出的内容。
- SDK 的 `probeFileMeta` 和 Rust 的 `probe_media_meta` 对 `tests/media_meta_vectors.json` 和 `media_samples/` 的结果完全一致；对这 3 个样本，EXIF 字段和 `exifread` 一致，HEIC 的显示尺寸是竖向的 4284×5712。
- `importFiles` 产出的 `cyfile` 带 3.5 键表里能取到的字段，QCID 命中时的 ObjId 和 `obj_data` 也一致。畸形文件不会让导入失败。
- 3.6 表格里的构造点都改用 helper，除决定 12 的例外外，代码里没有裸的 `FileObject::new` 或 `new FileObject` 产出上线对象（测试除外）。
- MessageHub 把 HEIC 转成 JPEG 后，`cyfile` 仍带原文件的 `captured_at`、`geo`、`camera`、`exposure`。
- 关注者看到的媒体带这些字段，`/content` 的 Content-Type 正确，由 3.6 的跨用户用例证明。
- HomeStation 接口的 `file` 字段是标准 `cyfile` JSON，代码里不再有 `import_metadata` 和 `file_view`。
- `mime_type` 不再作为 FileObject 的键被读写（3.6 列出的非 FileObject 用法除外）。
- 第 6 节的命令没有新增失败：`pnpm check` 通过，messagehub 用例的失败不超出基线那 7 个。

## 9. 完成记录（2026-11-06）

代码都在工作树，未提交。提交和合入顺序见第 5 节：SDK（含 `dist/`、`cli/dist/`）→ cyfs-ndn → 删除 `src/.cargo/config.toml` 后 `cargo update -p ndn-lib -p named_store -p bucket_store -p s3_gateway -p ndn-toolkit -p package-lib` → buckyos。上线前重建 paios 镜像（改了 `agent_tool`）。

文档之外做的决定：

- **HomeStation 放开 `cyfile` 的浮点数。** `protocol::obj_id_of` 原来拒绝对象里的所有浮点数，带 GPS 的照片因此发布不了。改为只对 HomeStation 自己的对象禁止，`cyfile` 放行，协议文档同步。
- **表格外的三处也改用 helper：** `BucketClient::put_file`、`put_reader`（一个 chunk 以上时只保留 `mime`）、OpenDAN 的 `put_file`、aiworkspace 的 `store_file`。走 `cacl_file_object` 存包内容的位置（`pikg`、安装器、repo）属于决定 12 的例外，不动。aiworkspace 内部的 `file_object` 是物化产物，也不动。
- **`captured_at` 用 Digitized 时，优先用 `SubSecTimeOriginal`、`OffsetTimeOriginal`，再回退到对应的 Digitized 标签。** 比第 3.5 节多一层回退。
- **`fileMeta` 是函数** `(file) => meta`：带 `mime` 就不再探测，不带就合并到探测结果之上。
- **m4a 这类音频不写** `captured_at`、`geo`、`camera`、尺寸和 `video_codec`，按键表的“适用”列。
- **`canonicalizeJson` 修了 `__proto__` 键被丢弃的问题**，否则这种输入算出的 ObjId 和 Rust 不同。
- **workflow 的 `IMPLEMENTATION_REVISION` 升到 `impl-3`**，FileObject 的 meta 变了，旧结果不应再命中。
- 第 6 节的 `cargo test -p fs_buffer` 包名不对，实际是 `fs-buffer`。

验证结果：

| 位置 | 结果 |
| --- | --- |
| SDK `npx jest`（全量） | 872 通过，13 跳过（需要环境） |
| SDK 向量和样本 | 209 个向量与 Rust 输出完全一致；3 个样本 TS、Rust、`exifread` 全部一致，HEIC 为 4284×5712 |
| MsgObject 差分 | 225 个边界输入，TS 与 Rust 的结论和 ObjId 一致；`"did"` 这个值 Rust 会 panic，见下 |
| 真实 Chromium（本地静态服务，非 Zone） | `ndn_types` 105 个（含 WebCrypto 验签）、`ndm_client` 36 个（含媒体元素回退）全过 |
| `cyfs-ndn` `ndn-lib`、`bucket_store`、`fs-buffer`、`ndn-toolkit`、`named_store` | 全过。`concurrent_writes` 并行时失败一次，单独运行通过 |
| `src/` homestation 全量、workflow、aicc、msg_center、nfs_server、buckyos-api、opendan、aiworkspace-store | 全过 |
| `src/` agent_tool | 328 通过，3 失败：本机没有 `rg`，与本次无关 |
| Desktop `pnpm check`、`tsc -b` + `vite build` | 通过 |
| Desktop `pnpm lint` | 5 个 error，都在未改动的文件（`RequestFields.tsx`、`PublishComposer.tsx`、`AgentDevices.tsx`） |
| Desktop messagehub 用例 | 94 通过，7 失败，失败的还是基线那 7 个（新增 2 个用例） |
| Desktop Playwright homestation | 12 通过，1 失败。失败的 `mute choices` 查找 `person\|` 选项，HEAD 的 UI 已没有，用例早已过时。新增的上传用例跑通了浏览器到后端的全链路 |

未验证：

- Zone 上的 DV（`test/jarvis_media_dv`、`pnpm run test:browser:real`）。
- 视频、音频只验证了 ffmpeg 生成的文件和合成字节，没有手机或相机拍的真实文件；照片没有 Android 样本。
- 容器内的 agent_tool：`build_aios` 未跑。
- `pnpm run test:cli`、`src/` 全量 `cargo test`。

cmd_skills：无需修改。手册读的是 AICC 结果的 `detail.files[].mime`，字段没变；`cyfile` 上的键从 `mime_type` 改成 `mime` 由 `stored_file_from_object` 在服务内消化，手册里也没有 `mime_type`。

留给以后：`name-lib` 的 `DID::from_str("did")` 会 panic（`parts[2..]` 越界），`MsgObject::from_json_value_checked` 收到 `"from": "did"` 就会触发，而它是接收不可信输入的入口，应该修。
