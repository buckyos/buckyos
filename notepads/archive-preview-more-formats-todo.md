# 压缩包浏览扩格式（7z、tar 系）TODO

状态：2026-11-16 owner 同意按本文方案实施，Q1–Q4 全部按建议定（见 §6）；同日已实施，未提交。真实 7-Zip 样本、基准、真实 Zone 未做，见 §8。日期：2026-11-16。源码基线：buckyos `af3ed7df0`（zip 预览已提交）、sevenz-rust2 0.25.0、tar 0.4.46。

**来源**：zip 预览（[archive-preview-files-todo.md](archive-preview-files-todo.md)，下称“zip TODO”）实施后，owner 在测试中提出：除 .zip 外还应支持更多格式，比如 .7z、.tar.gz。zip TODO 的 D15 把 7z / RAR / tar / tar.gz 列为“本版不做”，§6 要求先评估 solid block 和顺序流的索引成本、密码、跨平台依赖，新依赖要先评审。本文就是这份评估和实施方案。

**给实施 agent 的阅读顺序**：先读 zip TODO 的 §1、§2，本文只写和它不同的地方。没写到的规则一律沿用 zip TODO，包括地址、引用、鉴权、只读、密码记忆、文件名编码和缓存。§6 的 Q1–Q4 已按建议定。

## 1. 结论

| 格式 | 建议 | 新依赖 | 主要难点 |
| --- | --- | --- | --- |
| `.tar` | 本版做 | 无 | 没有中央目录，要顺序扫描头部；成员数据就是外层文件里的连续区间 |
| `.tar.gz` / `.tgz`、`.tar.xz` / `.txz`、`.tar.bz2` / `.tbz2` / `.tbz`、`.tar.zst` / `.tzst` | 本版做 | 无（`tar`、`flate2` 已是 workspace 依赖；`xz2`、`bzip2`、`zstd` 已在锁文件里，改成直接依赖） | 整个包是一条压缩流，不能随机访问。列目录也要解完整条流 |
| `.7z` | 本版做 | `sevenz-rust2`（Q1） | solid 块要从块头解起；加了 `-mhe` 的包连列目录都要密码 |
| 单文件压缩：`.gz`、`.xz`、`.bz2`、`.zst` | 建议顺带做（Q3） | 无 | 和 tar 系走同一条路径 |
| `.rar` | 本版不做（Q4） | — | 许可证：unRAR 许可证与 AGPL 不兼容 |
| 分卷包、ISO、cpio、deb / rpm、嵌套 | 不做 | — | zip TODO D15 照旧 |

## 2. 实验（2026-11-16，本机，scratchpad 程序）

样本：224 个文件、78 MB。其中 220 个是词表拼成的文本（高度可压缩），4 个是各 5 MB 的随机数据。7z 用 sevenz-rust2 自己的写入器生成，LZMA2 level 1。tar 系用系统的 `tar` 打包，再分别用 `gzip -6`、`xz -6 -T0`、`zstd -3`、`bzip2 -9` 压缩。数据在页缓存里，所以下面是偏乐观的上限。

**7z**（sevenz-rust2 0.25）

| 场景 | 结果 |
| --- | --- |
| 列目录（solid，1 个块含全部 224 个文件；非 solid，224 个块） | 0.17 ms / 0.27 ms |
| solid 块：第 1 个成员 / 偏移 1 MB 的成员 / 偏移 76 MB 的成员，首字节时间 | 0.5 ms / 6.8 ms / 353 ms（前面的数据必须先解，约 215 MB/s） |
| 非 solid：任意成员的首字节时间 | < 1 ms |
| 加密头（`-mhe`）：不给密码 / 错误密码 / 正确密码 | 列目录分别得到 `PasswordRequired` / `MaybeBadPassword`（< 0.1 ms）/ 成功 |
| 只加密数据：列目录 | 不需要密码，224 个成员都标为加密 |
| 只加密数据：错误密码读成员（试了 3 个） | 都在输出第 1 个字节之前报 `MaybeBadPassword`（LZMA2 控制字节校验） |

**tar 系**：用 `tar` crate 单遍扫描，边解压边建索引，同时把解出的 tar 写到文件。81 MB 的 tar，写出的文件和原 tar 逐字节一致。

| 外层 | 压缩后大小 | 单遍解压 + 建索引 + 写盘 |
| --- | --- | --- |
| `.tar` | 81 MB | 29 ms（只读头部，不复制） |
| `.tar.gz` | 28.8 MB | 48 ms |
| `.tar.zst` | 30.5 MB | 45 ms |
| `.tar.xz`（`-T0` 压出 4 个 block） | 26.7 MB | 221 ms |
| `.tar.bz2` | 25.8 MB | 1.49 s（54 MB/s） |

真实数据的解压通常更慢：gzip 约 200–500 MB/s，xz 约 50–150 MB/s，bzip2 约 20–50 MB/s。所以 1 GiB 的 `.tar.bz2` 第一次打开要等几十秒，T5 要实测。

**依赖面**（`sevenz-rust2 = { version = "0.25", default-features = false, features = ["aes256", "bzip2", "ppmd", "deflate"] }`，与锁文件对比）：

- 新增 crate：`sevenz-rust2`、`lzma-rust2`、`ppmd-rust`、`cbc`、`block-padding`、`cpubits`、`libbz2-rs-sys`。
- 已有 crate 的第二个版本：`aes` 0.9（现有 0.8）、`cipher` 0.5、`inout` 0.2、`bzip2` 0.6（现有 0.5）。
- 全部是纯 Rust，不需要新的 C 工具链。许可证是 Apache-2.0 / MIT。MSRV 1.93，本机 1.99。

**没有做到的**：用官方 7-Zip（`7zz`）生成真实样本的那一步被权限拦下，所以 BCJ2、PPMd、`-mx0`、Windows 上打的包都还没用真实样本验证，留给 T5。

## 3. 决定

本节是本文的“冻结”部分。凡是标注“我选的”，是我替 owner 选的；owner 推翻时只改对应任务。

| # | 决定 |
| --- | --- |
| E1 | **服务端按魔数识别格式，不看扩展名。** 依次检查：zip 本地头（`PK\x03\x04`，空包是 `PK\x05\x06`）、7z（`37 7A BC AF 27 1C`）、gzip（`1F 8B`）、xz（`FD 37 7A 58 5A 00`）、bzip2（`BZh`）、zstd（`28 B5 2F FD`）、tar（257 字节处是 `ustar`，或者是校验和正确的 v7 头），最后按现有做法找 zip 的 EOCD（前面带了其他数据的 zip）。RAR（`Rar!\x1A\x07`）和其他格式返回 `UNSUPPORTED`，带 `reason: "format"` 和 `format`，界面可以直接说“不支持 RAR”。扩展名只用来让客户端判断“要不要进包”；扩展名和实际格式不一致时，按实际格式打开 |
| E2 | **普通 `.tar`**：用 `entries_with_seek` 只读头部建索引。普通文件成员按外层文件里的字节区间读取，相当于 zip 的 Stored 路径（D9），支持任意 Range，不需要物化。外层版本和 `STALE` 的规则同 zip |
| E3 | **压缩的 tar（`.tar.*`）**：第一次访问时单遍解压整条流，边解边建索引，同时写出一个解压后的 `.tar` 到缓存目录 `archive/unpacked/`。之后完全按 E2 处理。<br>- 解压结果计入磁盘预算，按 LRU 驱逐；声明大小未知，所以边写边检查，超过 `max_materialize_bytes` 就中止并返回 `QUOTA_EXCEEDED`<br>- 被驱逐后按缓存未命中处理，重新解压，不返回 `STALE`<br>- 用 multi 解码器（`MultiGzDecoder`、`new_multi_decoder`、`MultiBzDecoder`、zstd 多帧），pigz、pbzip2 和多段拼接的流都能读<br>- **这是“不默认解压整包”的例外**，需要 owner 确认（Q2）。不这样做的代价是：每读一个成员都要从流头解到这个成员，翻下一项会越翻越慢 |
| E4 | **tar 条目类型**：普通文件（`0`、`\0`、`7`）是文件，`5` 是目录，pax 和 GNU 长名由 crate 处理。符号链接、硬链接、设备文件、FIFO、GNU sparse 都按 D12 处理：隐藏，计入 `hidden_entries`。sparse 的数据不是连续区间，所以也隐藏（我选的） |
| E5 | **tar 的同名条目**沿用 D12：两项都隐藏，路径返回 `AMBIGUOUS_ENTRY`。虽然 `tar -r` 追加的包在解开后是“后者覆盖前者”，但 D12 的原则是不默默选一项（我选的） |
| E6 | **文件名编码**<br>- tar：有 pax `path` 的条目按 UTF-8 处理，相当于 zip 的 UTF-8 标志；其他条目取原始字节，按 zip TODO §2.7 探测。tar 来自 POSIX，`\` 是普通字符，所以不做 §2.7 第 5 步的反斜杠替换，含 `\` 的名字过不了 `validate_name`，按 D12 隐藏<br>- 7z：格式本身存的是 UTF-16，不探测；但仍做 §2.7 第 5 步（只含 `\` 不含 `/` 时把 `\` 当分隔符），因为 Windows 上打的包可能用 `\`，要用真实样本确认（T5）<br>- `name_encoding.by` 新增取值 `"format"`，表示编码由格式规定 |
| E7 | **7z 列目录**：用 `Archive::read_with_memory_limit` 读头部。加了 `-mhe` 的包：<br>- 列目录、`stat` 都返回 `PASSWORD_REQUIRED`。zip TODO 的 D10（列目录不需要密码）只对 zip 和 tar 成立<br>- `archive.unlock` 的 `at` 是压缩包根时，用这个密码解头部。解头部带 CRC 校验，是完整校验，通过后直接记为“已验证”，满足条件时持久化（D6）<br>- 索引在用户之间共享，但每个用户每次访问都要先通过自己的密码检查，不然看不到任何名字 |
| E8 | **7z 成员读取**<br>- 块里只有这一个文件（非 solid）：同 zip 的 D9。没有 Range 或 Range 从 0 开始时流式解码；起点不为 0 时先物化<br>- 块的编码只有 Copy、不加密（`-mx0`）：成员是外层文件里的连续区间，按 E2 直接读，支持任意 Range<br>- solid 块（一个块里有多个文件）：第一次读到这个块的任何成员时，单遍解完整个块，把每个成员按顺序写进现有的成员缓存（`archive/members/`）。每个成员 CRC 通过后立即可用，请求方等到自己的成员就绪即返回，不必等整个块解完。同一个块的解码合并成一次；加密块按用户分开（同 zip TODO §8.2）。块的解压大小超过 `max_materialize_bytes` 时，这个块的成员返回 `QUOTA_EXCEEDED`。和 E3 一样，是“不默认解压整包”的例外（Q2）<br>- 反项（anti-item）隐藏；没有数据流的空文件是大小为 0 的文件 |
| E9 | **7z 密码**：只加密数据、头部不加密的包，快速校验是“开始解目标成员所在的块”：错误密码会立即被 LZMA / LZMA2 的头部检查拒绝。和 ZipCrypto 一样，快速校验通过只记为“待验证”，第一次完整读到成员末尾、CRC 对上，才升级为“已验证” |
| E10 | **7z 方法范围**：Copy、LZMA、LZMA2、PPMd、BZip2、Deflate，BCJ / BCJ2 / ARM / ARM64 / ARMT / PPC / SPARC / IA64 / RISC-V 过滤器，Delta，加密是 AES-256。不支持 Deflate64，也不支持 7-Zip-zstd 分支的 Zstd / Brotli / LZ4（`sevenz-rust2` 能开，但会再多一个 `zstd` 版本，而官方 7-Zip 不写这几种）。遇到不支持的方法返回 `UNSUPPORTED`，`reason: "method"` |
| E11 | **解码内存上限**：新增配置 `max_decoder_memory`，初值 256 MiB。作用于 7z 的 `memory_limit_kb` 和 xz 解码器的 memlimit；超出返回 `QUOTA_EXCEEDED`，带 `limit`。7-Zip 的 Ultra 和 `xz -9` 的字典都是 64 MiB，留了余量 |
| E12 | **D13（Files 设置）、D7（选择器）、D14（只读）、密码记忆、对象来源落盘（D16）都覆盖新格式**，不另设开关 |

## 4. 协议增量

对 zip TODO §2 的修改，其他部分不变。

**`hello.archive.formats`**：

```json
[
  { "format": "zip", "extensions": ["zip"], "methods": [...], "encryption": ["zipcrypto", "aes"] },
  { "format": "tar", "extensions": ["tar", "tar.gz", "tgz", "tar.xz", "txz", "tar.bz2", "tbz2", "tbz", "tar.zst", "tzst"],
    "compression": ["none", "gzip", "xz", "bzip2", "zstd"] },
  { "format": "7z", "extensions": ["7z"], "methods": [...], "encryption": ["aes256"] }
]
```

- `extensions` 可以带点。客户端按**最长后缀**匹配，不区分大小写，所以 `a.tar.gz` 匹配 `tar.gz`，不匹配 `gz`。
- 7z 的 `methods` 只列真实样本实测过的（T5）。
- Q3 答“做”时，再加一项 `{ "format": "compressed", "extensions": ["gz", "xz", "bz2", "zst"] }`：成员只有一个，名字取 gzip 头里的 FNAME，没有时取外层名去掉扩展名；解压结果按 E3 落盘。解出来恰好是 tar 时，按 tar 处理。

**`attrs.archive`**（压缩包根的 `stat`）：

- `format` 取 `"zip"`、`"tar"` 或 `"7z"`。
- tar 加 `compression`。
- 7z 加 `solid` 和 `header_encrypted`。

**成员 `attrs`**：

- tar：`method` 为 `"stored"`，`compressed_size` 省略，`encrypted` 为 false。
- 7z：`method` 是主编码器名（如 `"lzma2"`），solid 块里的成员 `compressed_size` 省略。
- 成员 `etag` 里 zip 用 crc32 的位置：tar 换成成员数据在 tar 里的偏移（十六进制），7z 用 CRC（没有 CRC 时用文件序号）。

**错误**：没有新增错误码。语义上有三处变化：

- `PASSWORD_REQUIRED` / `PASSWORD_INVALID` 也可能来自列目录和 `stat`（E7）。
- `QUOTA_EXCEEDED` 也可能来自解压整条流、整个块，或者解码内存（E3、E8、E11）。
- `UNSUPPORTED` 新增 `reason: "format"`（E1）。

## 5. 实施任务

顺序：T0 → T1 → T3（客户端）→ T2 → T4 → T5。T0、T3 和 T1 里的普通 tar 不依赖任何答复，可以先做。

### T0 — 格式抽象（不改行为）

- [ ] `Index` 里的 `zip: ZipArchive<…>` 改成按格式区分的枚举（`Zip`、`Tar`、`SevenZ`）。`Member` 里 zip 特有的字段（`method`、`header_offset`、`index`、`aes`、`strong`）收进按格式区分的位置信息。
- [ ] 读成员只留一个入口，返回三种情况之一：外层（或解压后文件）里的连续区间、可流式读取的解码器、要先解块。`read_member_inner`、`materialize`、`stream`、`copy_member_to`、`quick_check`、repr 都改走这个入口，不再直接调 `zip::open`。
- [ ] `describe` 改成汇总各格式。客户端的扩展名判断改成读 `hello`（见 T3）。
- [ ] 验收：现有 `archive` 单测和 `tests/archive.rs` 全部不改就能通过。

### T1 — tar 系（P0）

- [ ] 新增 `archive/tar.rs`：魔数识别（E1）、普通 tar 索引（E2）、压缩 tar 的单遍解压 + 索引（E3，`spawn_blocking`，可取消，合并同一外层的并发请求）、条目类型（E4）、同名（E5）、名字编码（E6）。
- [ ] `nfs_server/Cargo.toml` 引用 workspace 的 `tar`、`flate2`；`xz2`、`bzip2`、`zstd` 按锁文件里的版本加进 `src/Cargo.toml` 的 workspace 依赖，再由 nfs_server 引用。
- [ ] 解压结果的落盘复用 `cache.rs` 的预算、临时文件 + rename、0600 权限、启动时清理。路径来源和对象来源都要支持；对象来源先按 D16 落盘外层，再解压。
- [ ] 单测：GNU / ustar / pax 长名与 UTF-8 名、GBK 名（复用 `names.rs` 的探测）、四种压缩、多段 gzip、截断与损坏（`ARCHIVE_CORRUPT`）、超出预算（`QUOTA_EXCEEDED`）、符号链接 / 硬链接 / sparse 隐藏、同名、Range 读、外层被替换（`STALE`）、解压结果被驱逐后重建。

### T2 — 7z（P0，Q1、Q2 答复后）

- [ ] 加依赖（§2 的写法，`default-features = false`）。新增 `archive/sevenz.rs`：列目录（E7）、名字（E6）、成员读取（E8）、方法范围（E10）、内存上限（E11）。
- [ ] solid 块解码：合并键 = (索引键, 块号, 加密时加用户)，最后一个等待者离开时**不**取消，块解完为止（受预算约束），因为下一项、上一项很可能马上要用；服务停止时取消。每个成员在 CRC 通过后才 rename 进缓存。
- [ ] 加密头：`archive.unlock` 对根做完整校验（E7）；列目录、`stat`、解析成员路径前检查当前用户的密码。`prepare` 阶段遇到加密头的包，先按“需要密码”建一个空索引，解锁后再建完整索引。
- [ ] 快速校验（E9）复用 T5（zip TODO）的待验证 / 已验证流程。
- [ ] 单测：solid / 非 solid / Copy 块的读取与 Range、solid 块按顺序就绪、并发读同一块只解一次、加密数据（错误密码、待验证升级、按用户隔离缓存）、加密头（列目录要密码、他人看不到名字、忘记密码后再列）、不支持的方法、超出内存上限。

### T3 — 客户端（P0）

- [ ] websdk `nfsp.ts`：`ArchiveCapability.formats` 的类型改为三种格式（`compression`、带点的 `extensions`）；`opensArchive` 改成最长后缀匹配。
- [ ] Desktop `app/filebrowser/data/urls.ts`：去掉写死的 `ARCHIVE_EXTENSIONS = ['zip']`，`isArchiveName` 也按最长后缀匹配，扩展名列表从 `hello` 来。拿不到 `hello` 的同步调用点用一份与服务端同步的静态列表兜底，并加一条测试，保证两边列表一致。`archiveSplit` 随之支持 `a.tar.gz/@/archive/...`。
- [ ] **列目录时要密码**（E7）：Preview 的 `ArchiveBrowser` 和 Files 的包内列表遇到 `PASSWORD_REQUIRED` 时，弹出现有的 `PasswordDialog`，`at` 是压缩包根，解锁后重新列目录。现在这个对话框只在读成员时出现。
- [ ] **第一次打开的等待**（E3、E8）：Files 进包时显示加载中、可以取消（Preview 已有）。错误页区分“格式不支持”（`reason: "format"`，比如 RAR）和“压缩包损坏”。
- [ ] i18n：新增文案的中英文。
- [ ] 测试：`archive-urls.test.ts` 补 `tar.gz`、`tgz`、`7z` 的边界；websdk `archive_refs.test.ts` 补最长后缀匹配。

### T4 — 单文件压缩（Q3 答“做”时）

- [ ] 按 §4 的 `compressed` 项实现，复用 T1 的解压落盘。解出来是 tar 时按 tar 处理，不出现“包里套一个 tar”。

### T5 — 验证、基准与文档（P0 验收）

- [ ] 真实样本（可再分发，放进 fixtures；本次没能运行官方 7-Zip，见 §2）：
  - 7-Zip（Windows 和 Linux 各一份）：默认参数（solid）、`-ms=off`、`-mx0`、`-mx9`（BCJ2）、`-m0=PPMd`、`-m0=BZip2`、`-p` 加 / 不加 `-mhe`、带空目录和空文件、名字含中文，以及 Windows 上打的包的分隔符（E6）。
  - tar：GNU tar、bsdtar（macOS）、Windows 自带的 `tar.exe`、长名、pax UTF-8、GBK 名（老 Linux）、`pigz` / `pbzip2` / `xz -T0` / `zstd -T0` 压的包。
- [ ] 基准：1 GiB 的 `.tar.gz` / `.tar.xz` / `.tar.bz2` / `.tar.zst`，以及 1 GiB solid 7z，在本机 Zone 上测第一次列目录的时间（含对象来源落盘）、峰值内存和临时写入量。确认网关和客户端在最慢的 bz2 上不会超时。如果会超时，记下来请 owner 决定是否把建索引改成带进度的后台任务（§7）。
- [ ] 端到端：MessageHub 发 `.7z`（含加密头）和 `.tar.gz`，收件人浏览、预览、保存到 Zone、下载；Files 进包、设置关闭时的行为。
- [ ] 文档：nfs_server README 的归档一节和已知边界；zip TODO 的 D15 / §6 注明 7z、tar 已移到本文；cyfs-ndn 的 NFSP 文档同步 `hello` 和 `attrs.archive`。按 AGENTS.md 的 cmd_skills 表检查手册（预计不用改，依据同 zip TODO §8.5）。
- [ ] 验证入口同 zip TODO T8。

## 6. owner 答复

2026-11-16 owner：“按你的方案继续”，四项都按“建议”一列定。

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | 是否引入 `sevenz-rust2 0.25`（Apache-2.0，纯 Rust）？会新增 7 个 crate，并让 `aes` / `cipher` / `inout` / `bzip2` 各多一个版本（§2） | 引入。Rust 里只有这一个维护中的 7z 解码库；不用它，就要在 ext-core 里放外部程序 |
| Q2 | `.tar.gz` 这类压缩 tar 和 solid 7z 块，第一次访问时整条流 / 整个块解到缓存（磁盘 = 解压后大小，受 4 GiB 预算约束，超出返回“超出上限”）。这是 zip 方案“不默认解压整包”的例外，可以吗？ | 可以。不这样做，每读一个成员都要从头解到它，在大包里翻下一项会越来越慢；而且列目录本来就要解完整条流 |
| Q3 | 单独压缩的文件（`server.log.gz`、`dump.sql.xz`）也当作只含一个成员的压缩包浏览吗？ | 做。和 tar 系同一条路径，几乎没有额外代码 |
| Q4 | RAR 本版不做，可以吗？unRAR 许可证与 BuckyOS 的 AGPL-3.0 不兼容，`unrar` crate 不能链进 nfs_server；能走的路是在 ext-core 里放 libarchive（`bsdtar`），由 nfs_server 调用外部进程，但它不支持加密 RAR，RAR5 也只是部分支持 | 本版不做，返回“不支持的格式”。等早期用户真的提出再评估 libarchive 路线 |

## 7. 已知限制与风险

- **第一次打开大的压缩 tar 或 solid 7z 要等**：耗时等于整条流或整个块的解压时间，bz2 最慢。解压期间请求一直挂着，T5 要确认不会超时。
- **磁盘占用**：解压后的 tar 和 solid 块的成员都占预算，可能把别的缓存挤掉。
- **加密头的 7z 包索引在用户之间共享**：名字只在当前用户的密码通过后才返回；索引只放在内存和服务私有目录里，风险和 D6 相同。
- **7z 只用了 sevenz-rust2 自己写的样本**：真实 7-Zip 的样本、Windows 分隔符都留给 T5。
- **multi-block xz 和 seekable zstd 本可以按块随机访问**，本版不利用，统一按 E3 处理。

## 8. 实施记录

2026-11-16 实施，未提交（buckyos、buckyos-websdk 两个仓库）。

### 8.1 落地位置

| 任务 | 状态 | 位置 |
| --- | --- | --- |
| T0 | 完成 | 新增 `nfs_server/src/archive/source.rs`（`Source` / `PosReader` / `Stamp` / `pump` / 错误工具，从 `zip.rs` 移出）、`format.rs`（`Kind`、`Compression`、魔数识别 `sniff`、统一的 `Member` / `Loc` / `Body`、`decode`、`quick_check`、`data_start`）；`cache.rs` 的 `Index` 改为按格式区分，`Index::build` 先识别格式；`mod.rs` 的读、物化、流式、复制、快速校验都只走 `format::decode` |
| T1 | 完成 | `archive/tar.rs`（`is_header`、`parse_plain`、`unpack` 单遍解压 + 索引、`reopen`）；缓存目录 `<data_dir>/archive/unpacked/`；`src/Cargo.toml` 加 `xz2` / `bzip2` / `zstd`（锁文件已有的版本），nfs_server 引用 `tar` / `flate2` 等 |
| T2 | 完成 | `archive/sevenz.rs`（头部、成员、`for_block`、`decode`、`quick_check`、`unlock`）；`mod.rs` 的 `join_block` / `extract_block` / `block_member`（solid 块整块解）、`open_view`（加密头的用户视图）、`op_unlock` 的加密头分支；workspace 依赖 `sevenz-rust2 0.25`（`aes256`、`bzip2`、`ppmd`、`deflate`），测试另开 `compress` |
| T3 | 完成 | websdk `nfsp.ts`（`ArchiveFormat`、`ArchiveSummary` 新字段、`archiveExtensionOf` 最长后缀、`opensArchive`）并重建 dist；Desktop `data/urls.ts`（`ARCHIVE_EXTENSIONS` 全量、最长后缀）、`preview/archive.ts`（`archiveRootOfUrl`）、`ArchiveBrowser.tsx`（列目录要密码、错误文案）、新文件 `ArchivePasswordDialog.tsx`、`MainContent.tsx`（`onArchivePassword` 面板）、`FileBrowserView.tsx`（Files 的密码框）、`i18n/archive.ts` |
| T4 | 完成 | `tar.rs` 的 `Kind::Compressed` 分支：成员名取 gzip FNAME，否则外层名去掉压缩扩展名 |
| T5 | 部分 | 单测、集成测试、本机浏览器验证、README；真实 7-Zip 样本、基准、真实 Zone、MessageHub 端到端、真机未做（§8.4） |

### 8.2 §3 / §4 之外的取舍

- **加密头的 7z 按用户建视图（E7 的实现方式）**：用户解锁后，用密码解出的头部单独建一个索引，键是 `<外层键>|u:<用户>`，只有这个用户的请求会用到；E7 写的是“索引共享、每次检查密码”，这样做名字从不进入共享结构，检查也只有一处。会话里已验证或记住的密码在 `prepare` 阶段自动打开视图。`archive.forget {at}` 删这个包的视图，`{all:true}` 删这个用户的全部视图（浏览器验证时发现 `all` 原先漏了，已修并加了用例）。
- **Copy 块按字节区间读（E8）**：只在整个包没有 BCJ2 块时启用。sevenz-rust2 不公开每块的 pack stream 序号，没有 BCJ2 时每块正好一个 pack stream，按块号推算；有 BCJ2 时 Copy 块按普通块解码。
- **大小为 0 的成员**：7z 里即使带数据流，也按空区间直接返回，不算加密，不参与块解码。
- **成员缓存键**：所有格式改为按条目序号（不再按原始名字节），块解码时可以直接定位缓存文件。zip 旧版本留下的缓存文件不再命中，等 LRU 驱逐。
- **tar 名字**：开头的 `./` 去掉，`./` 本身不算成员也不计入隐藏数；有 pax `path` 的名字视同带 UTF-8 标志。
- **单个压缩文件**：外层名没有压缩扩展名时，成员名是 `<外层名>.out`；gzip 的成员 mtime 取头部 MTIME。
- **未知格式**：开头不像任何已知格式、也找不到 zip EOCD 的文件，返回 `UNSUPPORTED{reason:"format"}`（原来是 `ARCHIVE_CORRUPT`）；以 `PK` 开头但坏了的仍是 `ARCHIVE_CORRUPT`。
- **同步兜底**：没经过 `prepare` 的解析遇到压缩外层时，返回 `UNSUPPORTED{reason:"archive_not_prepared"}`，不在异步线程上解压；`prepare` 失败（超出上限、取消）记下来原样返回。
- **`hello`**：7z 分 `methods` 和 `filters`，只列 sevenz-rust2 写出、再由本服务解出的（BCJ2 能解，但写不出样本，不列）；`limits` 加 `max_decoder_memory`。
- **块解码的停止**：块解码不因等待者离开而停，也没有接服务停止信号，随进程退出结束。
- **扩展名两边一致**：Desktop `ARCHIVE_EXTENSIONS` 与服务端 `describe` 由 `archive::tests::desktop_knows_every_extension_hello_lists` 比对（测试里 `include_str!` 读 Desktop 源文件）。
- **Files 第一次打开**：等待期间显示列表骨架屏，没有单独的取消按钮，离开这个目录即可；请求被中止后服务端是否随之取消没有验证。

### 8.3 样本实测

- **tar 系**（集成测试 `tar_family_browse_and_read`、单测）：`.tar`、`.tar.gz`、`.tgz`、`.tar.xz`、`.tar.bz2`、`.tar.zst`，由 `tar` crate 打包、Rust 压缩库压缩；GNU 头、`./` 前缀、符号链接隐藏、反斜杠名字隐藏、条目上限、Range 读、解压结果复用、多段拼接的 gzip、取消后不留文件、超出上限返回 `QUOTA_EXCEEDED`。系统 `tar` + `gzip` / `xz -T0` 打的包在浏览器验证里可用。
- **单个压缩文件**：带 FNAME 的 gzip、`dump.sql.zst`、系统 `gzip` 管道压的 `server.log.gz`（无 FNAME）。
- **7z**（全部由 sevenz-rust2 0.25 写出）：solid（LZMA2）、非 solid、Copy、Copy + solid、只加密数据、加密头；COPY / LZMA / LZMA2 / PPMd / BZip2 / Deflate 与 BCJ / ARM / ARM64 / ARMT / PPC / IA64 / SPARC / RISC-V / Delta 过滤器逐一解码一致；篡改 CRC 得到 `ARCHIVE_CORRUPT{reason:"decrypt_or_integrity"}`；错误密码快速校验失败。
- **没有的样本**：官方 7-Zip（Windows / Linux）写的包、BCJ2、Windows 上打的 7z 的分隔符、PowerShell / Windows `tar.exe` / bsdtar 打的包、pbzip2、GBK 名字的 tar、GNU sparse 实物。下载运行官方 `7zz` 被本机权限拦下。

### 8.4 验证

- `cargo test -p nfs_server -- --test-threads=1`：全部通过（单元 118 个，其中归档相关的新增 10 个；`tests/archive.rs` 15 个，新增 8 个）。`cargo clippy -p nfs_server --no-deps --tests` 归档代码无警告。
- websdk：`tests/archive_refs.test.ts`（新增最长后缀用例）、`object_url` 通过；`pnpm build` 通过，dist 已更新。
- Desktop：`pnpm check`（`tsc -b`）通过；改动文件 `eslint` 无错误；`node --experimental-strip-types tests/datamodel/archive-urls.test.ts` 通过（新增 7z、tar.gz、tgz 用例）。
- 浏览器（独立 nfs_server + `VITE_NFS_PROXY` 的 Desktop，`?fbData=nfsp`，Playwright 脚本，不进仓库）：Files 双击进入 `.tar.gz` 和子目录、`.tar.xz` 子目录、solid `.7z`、`server.log.gz`；加密头 `.7z` 显示“压缩包已加密”面板，错误密码提示、正确密码后列出；同一用户新会话、服务重启后（靠持久化的密码）都直接列出；Preview 从右键“预览压缩包”打开 `.7z`，进图片（上一项 / 下一项）、进子目录打开 Markdown；Preview 打开加密头 `.7z` 时自动弹出密码框。
- **未验证**：真实 Zone 与 MessageHub 收发 `.7z` / `.tar.gz` 端到端、对象来源的浏览器界面（只有集成测试）、手机真机、Windows / macOS、1 GiB 样本的基准（第一次打开的耗时、峰值内存、网关是否超时）、Playwright 用例入库。cyfs-ndn 的 NFSP 文档本来就没有压缩包一节（zip 那次也没写），这次没补。

### 8.5 手册检查（AGENTS.md cmd_skills 表）

不需要修改：没有新增 CLI、设置项或运行路径；新格式的成员地址仍是 `cyfs:///<文件>/@/archive/...`，读取方式与 zip 相同（依据同 zip TODO §8.5）。
