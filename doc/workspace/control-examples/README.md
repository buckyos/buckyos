# control-examples：按“拼装”原则做的四个控件

> [控件机制完善 TODO](<../../../notepads/aiworkspace-control-mechanism-todo.md>) §0.4 的配套例子，2026-11-12 建立。前三个只用 `composition.json` 拼装（label、media / asset、shape 加投影），第四个走 HTML 路线。`aiws-manifest.json` 是 `manifest_version: 4`，`digest` 是按 websdk `aiwsContentDigest` 算的真值；websdk `tests/aiws_ext.test.ts` 用 `verifyBundle` 整包核对，服务端 `server/tests/ext.rs` 的 `control_examples_and_manifest_4` 用真实 `ext.build` 逐个构建。改了任何文件都要重算对应控件的 `digest`。权威契约仍是 aiworkspace README 与 `src/frame/aiworkspace/schemas/`。

| 控件 | 路线 | 用到的机制 | 验收（TODO §5） |
| --- | --- | --- | --- |
| `tv-screen` 电视屏 | composite | `media`：slot 里的视频按 `config.broadcast` 锚点播放（G17），服务端时钟（G16），空 slot 时从 Zone 选视频 | §5.3 |
| `time-limited` 限时可见 | composite | slot 的 `visible: { from: "first_open", for_ms }`（G19），观众投影显示剩余时间（`viewer` 数据源，`countdown` 格式） | §5.4 |
| `photo-vote` 照片投票 | composite | 目录成员投影 + 本人选择标记（G2、G20），计数投影做封面（G21），事件默认处理 `events.pick.on`，声明动作“准备选票” | §5.5 |
| `listen-together` 一起听 | HTML（host API 4） | 声明动作（H2）、`onConfig`（H1）、`serverNow`（H5）、`userState`（H4）、`mediaUrl` 流式地址（H6） | §5.6 |

## 怎么用

```bash
buckyos aiws build doc/workspace/control-examples/controls/tv-screen --workspace $WS
```

或在 Desktop 里“插入 → 扩展控件 → 工作区控件… → 构建”这个目录。四个控件互不依赖。

### 电视屏

1. 插入后屏幕是空的：有修改权的人点“选择视频或音频”，系统选择器先给 Zone 里的文件（本机是其中一个来源）。视频和音频上限 2 GiB，其余资产 32 MiB。
2. 屏幕下方的播出栏：“开始播出 / 暂停 / 设为播出进度 / 从头播 / 换视频”（只对有修改权的人显示）。每一下都是一个可撤销的 Commit，写 `config.broadcast = { pos_ms, at_ms, playing }`；换视频时资产和锚点在同一个 Commit 里。
3. 观众按服务端时间跟播：偏差超过 400 ms 跳转，80–400 ms 之间微调播放速度。本机暂停、静音只影响自己，“回到直播”回到当前播出位置。

### 限时可见

把要限时的 Block 拖进控件（进入 `content` slot），在属性里改 `limit_ms`。每个观众第一次打开时，宿主用服务端时钟把时间记进本人的 User State（`block:<id>/first_open`），到时间后这些 Block 只对这个观众隐藏。这是呈现，不是权限：编辑模式全部显示并标“限时内容”，打印、导出、静态视图不隐藏。

### 照片投票

1. 先建一张投票表：一个 `text` 字段设 `submitter: true`（服务端填写提交者），一个 `multi_select` 字段 `picks`，`max_items: 3`。
2. 插入控件，绑定 `photos`（照片目录）和 `votes`（这张表），然后在工具栏点“准备选票”：给每张照片加一个选项（选项名就是文件名）。以后加了照片再点一次。
3. 运行模式下点照片就是选 / 取消；每人一条记录，只能改自己的，最多 `picks` 张。封面区是得票前 3 张，同票按目录顺序，由读取方计算，不写回。

### 一起听

绑定一个音频目录（同名 `.lrc` 是歌词）。owner 不用先激活控件，直接在工具栏点“开始一起听 / 暂停 / 继续 / 下一首 / 顺序播放 / 随机播放”：开始和播放模式由宿主直接写配置；暂停、继续、下一首转给 HTML 计算后写入（控件没在运行时宿主先启动它）。正在听的人立刻跟上。每个人可以切到“自己听”、拖进度、切歌；音量和自己听的位置记在本人的 User State 里，下次打开接着听。

## 已知限制

- 一起听的波形是按曲名生成的示意波形，随播放进度走；没有接 WebAudio 分析器（跨源音频接入分析器会被静音）。
- 照片投票的投票人对能读这张表的人可见；匿名投票要“只读聚合”权限，v1 不做（TODO Q13）。
- 流式地址在服务端按顺序读到目标位置（大文件深处的跳转较慢），见实施记录。
