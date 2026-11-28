# Personal Site Protocol

个人重排站（Agent 浏览器二期）的契约：用户重排桶里的数据、来源包、`site` 命令、模板页面、页面和 App 的消息、阅读记录的上报。设计依据见 [二期实施 TODO](<../../../notepads/agent-browser-phase2-todo.md>) §4。浏览器一侧的 kRPC（`browser.background_run`、`browser.page_hint`、`browser.visits`）在 [Agent Browser Protocol](<Agent Browser Protocol.md>) §2、§7；Goal 作业在 [Agent State Protocol](<Agent State Protocol.md>) §9.9。

消费方：`site` 命令（`src/tools/buckyos-agent/commands/site.ts`、`lib/site/`）、默认模板（`src/tools/buckyos-agent/site-template/`）、nfs-server 的 `reflow.<zone>` 主机、Desktop 的「Agent 浏览器」App、来源包的作者（Agent）。

共用的样例在 [`fixtures/site/`](fixtures/site/)：`site.json`、`feed.json`、`article.json`、`events.json`、`reading.json`、`feed_object_vectors.json`。TS、Rust、Desktop 的测试读同一份。

## 1. 名词

| 名词 | 含义 |
| --- | --- |
| 个人站 | 一个用户一个，数据在这个用户的重排桶里，页面在 `https://reflow.<zone>/` |
| 来源（source） | 主人长期阅读的一个网站或它的一个栏目。`<source>` 是它的 id：`[a-z0-9-]`，1–48 个字符，不以 `-` 开头或结尾；缺省取主主机名的 `<site>`（Agent Browser Protocol §5.1） |
| 来源包 | 取一个来源的程序和它的说明，一个目录（§4） |
| 文章 | 收录的一篇内容。`<article>` 是规范化网址（Agent Browser Protocol §5.1）SHA-256 的前 16 个十六进制字符，和一期的 `<page>` 同一算法、同一组向量 |
| 模板 | 画页面的一套静态文件，按版本存，`t<n>` |
| 维护者 | 现在维护这个站的 Agent 和它的 Goal，记在 `site.json` 的 `maintainer` |

三样东西分开：来源包和模板的源码在维护者的工作区；装进桶里和 Goal 作业里的是运行副本；内容归用户，在桶里。

## 2. 桶和 key

桶 `user/control-panel/<用户>/reflow`（control-panel 声明，配额 8192 MiB，不开版本）。

| key | 写的人 | 内容 |
| --- | --- | --- |
| `index.html` | `site init`、`site template put` | 入口（§6.1），所有模板版本共用 |
| `_app/<version>/**` | `site template put` | 一个模板版本，写入后不改，带 `Cache-Control: public, max-age=31536000, immutable` |
| `_data/site.json` | Work、主人的 UI 会话、App（主人）、Goal（只改参数） | 站点定义（§3.1）。写入带 `If-Match`（第一次带 `If-None-Match: *`） |
| `_data/feed/<source>.json` | 这个来源的作业 | 状态和最近的条目（§3.2）。一次同步里最后写它 |
| `_data/a/<article>.json` | 同上 | 文章（§3.3） |
| `_data/md/<article>.md` | 同上 | 译文的 Markdown，FeedObject 的 `wraps` 指向它；页面不读 |
| `_data/img/<sha256>.<ext>` | 同上 | 图片，以存下来的字节的 SHA-256（64 个十六进制字符）命名，带 `immutable` |
| `_data/events.json` | Goal | 事件（§3.5） |
| `_data/reading.json` | 汇总作业（`site reading --compact`） | 阅读汇总（§3.6） |
| `_read/<yyyymmdd>/<ulid>.json` | nfs-server（`POST /_read`） | 页面上报的一批阅读记录，90 天过期 |
| `_src/<source>/<version>/**` | `site source put` | 来源包的副本（换维护者时取回），`<version>` 是 `s<n>` |

- 新的 key 都以 `_` 开头或就是 `index.html`，不会和一期的 `<site>/…`（必含 `-`）冲突，也不会和网关在这个主机上按路径分走的前缀相同。
- 写入顺序保证读的人不会看到半成品：图片、Markdown、文章先写，`feed/<source>.json` 后写；模板文件先写，`site.json` 的 `template.version` 后改。
- `_data/a/`、`_data/md/`、`_data/img/` 下的对象写入时带过期时间（`fetched_at` 加 181 天；图片每次被再次引用时顺延）。作业按保留期删得更早，过期只是兜底：来源被移除、换了维护者之后留下的对象靠它清掉。
- `_data/` 下的 JSON 是 UTF-8、对象在最外层、带 `schema` 字段（`_read/` 下的文件是页面上报的数组原样，§7）。时间一律是毫秒时间戳（FeedObject 里的 `iat` 除外，它是秒）。

## 3. 数据

### 3.1 `_data/site.json`

```json
{
  "schema": "buckyos.site/1",
  "title": "我的阅读",
  "language": "zh-CN",
  "owner_did": "did:bns:alice",
  "topics": ["国际", "科技", "体育"],
  "template": { "version": "t1", "params": { "home_order": "time", "per_column": 20, "hide_topics": [], "collapsed": [] } },
  "columns": [ { "id": "jp", "title": "日本", "sources": ["news-yahoo-co-jp"] } ],
  "sources": [
    { "id": "news-yahoo-co-jp", "title": "雅虎日本", "home_url": "https://news.yahoo.co.jp/",
      "match": [ { "host": "news.yahoo.co.jp" } ], "lang": "ja", "kind": "news", "needs": "http",
      "job": "src-news-yahoo-co-jp", "version": "s1", "every_secs": 1800, "max_new": 20, "keep_days": 30, "comments": 10,
      "state": "active", "note": null, "added_at": 1764000000000, "added_by": "work:ws-…" }
  ],
  "upgrades": [],
  "reading": { "enabled": true },
  "limits": { "translate_tokens_per_day": 1000000 },
  "maintainer": { "agent": "did:bns:jarvis.alice", "goal_id": "g-…" },
  "updated_at": 1764000000000,
  "updated_by": "did:bns:jarvis.alice"
}
```

- `language` 是站点语言（BCP 47），译文和界面文字都用它。`owner_did` 是主人的 DID，FeedObject 的 `publisher` 取它。
- `topics` 是归类用的标签表，至多 16 个；文章的 `tags` 只会是其中的值。
- `template.params`：`home_order`（`time | source | events`）、`per_column`（1–100）、`hide_topics`（主人不想看的标签，**只由主人的话改**，Goal 不自己改）、`collapsed`（默认折叠的栏目 id）。模板不认识的参数忽略。
- `columns`：首页的栏目和顺序；不在任何栏目里的来源排在最后。
- `sources[]`：
  - `match[]` 同一期的匹配规则（`host`、可选的通配 `path`）：一个网址属于第一个匹配它的来源。
  - `kind`：`news | blog | forum`；`needs`：`http | browser | login`（§4.1）。
  - `job`：这个来源的 Goal 作业 id，`src-<source>`。`version`：桶里 `_src/<source>/` 的当前版本。
  - `every_secs`、`max_new`、`keep_days`、`comments`：同步间隔（≥ 300）、一次最多发布的新文章、文章保留天数、每篇收的评论数（0 关闭，至多 10）。缺省按 `kind`：新闻 1800、博客 21600、论坛 3600；`needs` 是 `login` 时缺省 21600；`max_new` 20、`keep_days` 30、`comments` 10。读过或归入事件的文章保留 180 天。
  - `state`：`preparing`（Work 正在接入）、`active`、`paused`（主人暂停，作业停用）、`failed`（接入没成功，`note` 是原因）。作业不写这个字段。
- `upgrades[]`：升级请求，`{ "id", "source"?, "title", "summary", "file", "goal_id", "opened_at", "state": "open" | "accepted" | "dismissed" | "closed", "decided_at"? }`。`id` 是短名；`file` 是工作区里那份说明的路径；带 `source` 的显示在那个来源上，不带的显示在总首页。Goal 用 `site upgrade open` 写，主人在 App 里点“升级 / 暂不”改 `state`，Work 做完用 `site upgrade close`。至多留 50 条，`closed`、`dismissed` 的超过后删旧的。
- `reading.enabled`：阅读记录的开关（缺省开）。关着时模板不上报。
- `limits.translate_tokens_per_day`：这个站的作业每天用于翻译的 token 上限，缺省 1000000；交接时写成它和 Goal 周期预算一半中较小的那个。
- `maintainer`：现在维护它的 Agent 和 Goal。没有时是 `null`。
- 来源的**显示状态**由三处合成，按顺序取第一个成立的：`site.json` 里是 `paused` → `paused`；有 `state` 为 `open` 的升级请求指着它 → `needs_upgrade`；`site.json` 里是 `preparing` 或 `failed` → 照原样；否则取 `feed/<source>.json` 的 `status.state`（没有这个文件是 `preparing`）。

### 3.2 `_data/feed/<source>.json`

```json
{
  "schema": "buckyos.site.feed/1",
  "source": "news-yahoo-co-jp",
  "status": { "state": "ok", "checked_at": 1764001800000, "published_at": 1764001800000, "error": null, "pending": null, "login_url": null },
  "items": [
    { "id": "0123456789abcdef", "object_id": "cyfeed:…", "url": "https://news.yahoo.co.jp/articles/…",
      "title": "<译文标题>", "summary": "<译文摘要>", "image": "_data/img/<sha256>.jpg",
      "tags": ["科技"], "published_at": 1764000000000, "fetched_at": 1764001700000, "rev": 1 }
  ],
  "updated_at": 1764001800000
}
```

- `status.state`：`preparing`（还没有通过核对的同步）、`ok`、`failed`、`needs_login`。
- `checked_at`：最近一次**通过核对**的检查时间，没有新文章也算；跑完但没通过核对不更新。`published_at`：最近一次发布新文章的时间。
- `error`：`{ "kind": "transient" | "auth" | "program" | "config", "message", "at" }`，最近一次失败；成功后清成 `null`。`login_url` 只在 `needs_login` 时有。
- `pending`：有文章取到了但没发布时的说明，`{ "count", "reason": "translate_budget" | "translate_config" | "translate_failed" | "time" | "images", "message" }`；页面按 `reason` 显示本地化的一句（如“今日翻译额度已用完”）。
- `items[]` 按 `published_at`（没有时 `fetched_at`）从新到旧，至多 100 条。标题和摘要是译文；`image` 是列表用的小图在桶里的 key（没有配图是 `null`）。列表只放索引，不放整个 FeedObject。
- 作业整份重写这个文件；别的角色不写它。

### 3.3 `_data/a/<article>.json`

```json
{
  "schema": "buckyos.site.article/1",
  "id": "0123456789abcdef", "source": "news-yahoo-co-jp",
  "url": "https://news.yahoo.co.jp/articles/…", "lang": "ja",
  "published_at": 1764000000000, "fetched_at": 1764001700000, "rev": 1,
  "object_id": "cyfeed:…", "object": { "…": "§3.4" },
  "title": { "o": "<原文>", "t": "<译文>" },
  "summary": { "o": "", "t": "<译文摘要>" },
  "byline": "<作者或媒体>",
  "tags": ["科技"],
  "blocks": [
    { "type": "p", "text": { "o": "…", "t": "…" } },
    { "type": "img", "src": "_data/img/<sha256>.jpg", "object": "cyfile:…", "alt": { "o": "…", "t": "…" }, "w": 1280, "h": 720 },
    { "type": "table", "rows": [ [ { "o": "…", "t": "…" } ] ] }
  ],
  "comments": [ { "author": "…", "at": 1764000500000, "text": { "o": "…", "t": "…" } } ],
  "links": [ { "url": "https://…", "text": { "o": "…", "t": "…" }, "article": "fedcba9876543210" } ],
  "images": { "cyfile:…": { "key": "_data/img/<sha256>.jpg", "w": 1280, "h": 720 } },
  "thumb": "_data/img/<sha256>.jpg",
  "supplement": null
}
```

- 双语文本一律是 `{ "o": 原文, "t": 译文 }`。原文语言和站点语言相同时 `t` 等于 `o`。页面按文本渲染，从不当 HTML 插入。
- `blocks[].type`：`p`、`h2`、`h3`、`quote`、`li`、`code`（都带 `text`）；`img`（`src` 是桶里的 key，`object` 是它的 `cyfile` ID，`alt`、`w`、`h`；图片没取到时 `src`、`object` 为 `null`、带 `"missing": true`）；`table`（`rows` 是双语文本的二维数组）。正文里的链接收在 `links`，不做行内链接；`links[].article` 是这个网址在站内的文章 id（收录了才有）。
- `comments` 至多 `sources[].comments` 条，按原站的默认排序。`author` 是原站显示的名字。
- `rev` 从 1 起；原文更正后覆盖同一个 key，`rev` 加一。
- `thumb`：列表用的小图（长边至多 480）的 key，没有配图是 `null`。
- `supplement`：补充文章才有，`{ "event": "<事件 id>", "adds": "<为之前的报道增加了什么>" }`（§3.5）。
- `object`、`object_id`：FeedObject 外壳（§3.4）。文章 JSON 是完整的一份，外壳是它的一个视图；两者冲突时不为外壳删减文章。
- **站内只用 `<article>` 作主键**：路由、阅读记录、事件都用它，更正不会断开。

### 3.4 FeedObject 外壳

每篇文章带一个合法的 HomeStation FeedObject（按它的私人抓取约定：主人名义、没有 `entry`、只在本地、不签名、不发表），`object_id` 是它的 ObjId（`cyfeed`，规范化 JSON 的 SHA-256，websdk 的 `buildNamedObjectByJson('cyfeed', object)`）。合不合法只看 HomeStation 的 `validate_feed_object`（`src/frame/homestation/src/protocol.rs`）。

```json
{ "kind": "post", "publisher": "<site.json 的 owner_did>", "iat": 1764001700,
  "content": { "type": "article", "title": "<译文标题>", "summary": "<译文摘要>",
               "cover": "cyfile:…", "media": [ { "object": "cyfile:…", "alt": "…" } ] },
  "wraps": "cyfile:…", "tags": ["科技"], "link": "<原文网址>",
  "source": { "kind": "web", "original_url": "<原文网址>", "original_author": "<byline>", "captured_at_ms": 1764001700000 },
  "base_on": "cyfeed:…" }
```

- `wraps` 是 `_data/md/<article>.md` 的 `cyfile` ID；`cover`（第一张配图）、`media`（至多 9 张，其余只在 `blocks` 里）是 `_data/img/` 里图片的 `cyfile` ID。都取 `PUT /bucket/v1` 返回的 `obj_id`，不在脚本里自己算。没有配图就没有 `cover` 和 `media`。
- `iat` 是抓取时刻（秒，`fetched_at / 1000` 取整），`captured_at_ms` 是 `fetched_at`。不带 `nonce`、`entry`。同一次抓取算出来的 ObjId 是确定的。
- 更正（`rev` 加一）是新的对象，`base_on` 指前一个 `object_id`；第一版没有 `base_on`。
- `tags` 就是文章的 `tags`（至多 3 个，取自 `site.json` 的 `topics`）。空数组、空字符串、`null` 的字段都不写进对象。
- 标题和摘要合计超过 8000 字符时摘要截短。对象超过 64 KiB、或校验不过时，这一篇不发布。
- Markdown：`# 标题`，一行 `原文：<网址>`，然后各块按顺序（`p` 是段落，`h2`/`h3` 是 `##`/`###`，`quote` 是 `>`，`li` 是 `-`，`code` 是围栏代码块，`img` 是 `![alt](<图片的 cyfile ID>)`，没取到的图不写，`table` 是 Markdown 表格），只用译文，页面文字里的 Markdown 和 HTML 记号都转义。评论不进 Markdown。
- 原站的评论不做成 FeedObject。原文、原文语言、发布时间在 FeedObject 里没有位置，留在文章里。
- 要登录才能看到的来源照样有外壳；以后做分享时要默认排除（本期没有消费方）。

`lib/site/feed.ts` 的校验只收这个外壳的形状，比 HomeStation 严：它收的 HomeStation 都收，反过来不一定（HomeStation 还收 `entry`、`references`、`content.text`、base32 的 ObjId 等）。

校验规则和 ObjId 的共用向量在 `fixtures/site/feed_object_vectors.json`：`valid[]` 是 `{ name, object, object_id }`，`invalid[]` 是 `{ name, object, problem }`。`lib/site/feed.ts` 的测试和 `cargo test -p homestation` 都读它。

### 3.5 `_data/events.json`

```json
{ "schema": "buckyos.site.events/1",
  "events": [ { "id": "e-20261124-chip-export", "title": "…", "summary": "…",
                "articles": [ { "id": "<article>", "adds": "…", "title": "<译文标题>", "source": "<source>", "at": 1764000000000 } ],
                "supplements": [ { "article": "<article>", "source_url": "https://…", "adds": "…", "title": "<译文标题>", "at": 1764000000000 } ],
                "updated_at": 1764000000000 } ],
  "updated_at": 1764000000000 }
```

`id` 是 `e-` 开头的短名（`[a-z0-9-]`，至多 48 个字符）。`articles` 是归到这件事的已收录文章，`adds` 是这一篇相对之前的报道增加了什么。`supplements` 是 Goal 为这件事另找的后续或原始材料：每条是一篇补充文章（存成普通文章，`source` 是 `_supplement`，带 `supplement`），写清出处和增加了什么。至多 200 个事件。标题、摘要、`adds` 用站点语言。`articles[]`、`supplements[]` 里的 `title`、`source`、`at`（发布时间，没有时是抓取时间）由 `site events put` 从文章里取来填上，写的人不用给；事件页靠它们排序和显示，不用逐篇去取文章。

### 3.6 阅读记录

页面上报的一条：`{ "article": "<article>", "source": "<source>", "opened_at": <ms>, "dwell_ms": <整数>, "max_scroll": <0–100 的整数> }`。`dwell_ms` 只算页面可见的时间；`max_scroll` 是读到的最远位置占全文的百分比。

`_data/reading.json`（汇总，保留 90 天内的记录）：

```json
{ "schema": "buckyos.site.reading/1", "updated_at": 1764000000000, "since": 1756000000000,
  "articles": { "<article>": { "source": "…", "opens": 2, "dwell_ms": 84000, "max_scroll": 100, "last_opened_at": 1764000000000 } },
  "sources": { "<source>": { "opens": 12, "dwell_ms": 600000, "last_opened_at": 1764000000000 } },
  "tags": { "科技": { "opens": 7, "dwell_ms": 420000 } } }
```

阅读记录只给维护这个站的 Agent 用；主人在 App 里可以看、清除（删掉 `_read/` 和 `reading.json`）和关闭。

## 4. 来源包

一个目录：

| 文件 | 内容 |
| --- | --- |
| `source.json` | 声明（下面） |
| `extract.ts` | 取数的程序，默认导出一个函数（§4.2） |
| `fixtures/` | 几份真实页面，给 `site try --fixtures` 和以后修它的人用；`fixtures/index.json` 是 `{ "<网址>": "<文件名>" }`，`site try --record` 把取到的页面和它一起写下 |
| `NOTES.md` | 怎么取、怎么核对、有什么坑；给以后修它的 Goal 和 Work 看 |

至多 200 个文件、20 MiB（它整个装进一个 Goal 作业）。

### 4.1 `source.json`

```json
{ "schema": "buckyos.site.source/1", "id": "news-yahoo-co-jp", "title": "雅虎日本",
  "home_url": "https://news.yahoo.co.jp/", "match": [ { "host": "news.yahoo.co.jp" } ],
  "lang": "ja", "kind": "news", "needs": "http",
  "every_secs": 1800, "max_new": 20, "keep_days": 30, "comments": 10,
  "check": { "min_chars": 200, "median_ratio": 0.3, "max_replacement": 0.01 } }
```

- `needs`：`http`（普通请求，不开浏览器）、`browser`（要渲染，用后台标签页）、`login`（要主人的登录态，用后台标签页）。取数的顺序是订阅源 / 接口 → 普通请求 → 浏览器，能用前面的不用后面的。
- `check` 是核对的阈值，都可以省略（§5.3）。
- `site source put` 把 `id`、`title`、`home_url`、`match`、`lang`、`kind`、`needs` 和四个参数写进 `site.json` 的这一项；主人后来在 `site.json` 里改的 `every_secs`、`max_new`、`keep_days`、`comments`、`title` 以 `site.json` 为准，重新 `put` 不覆盖。

### 4.2 `extract.ts`

```ts
export default async function extract(ctx: ExtractCtx): Promise<Extracted>;

interface ExtractCtx {
  source: SourceDef;                 // source.json；every_secs、max_new、keep_days、comments 是 site.json 里这一项的现值
  state: unknown;                    // 上次返回的 state（第一次是 null）
  limit: number;                     // 这一次最多取多少篇新文章
  known(url: string): boolean;       // 这个网址已经收录、而且不需要重取
  log(message: string): void;
  fetch(url: string, init?: { conditional?: boolean; headers?: Record<string, string> }):
    Promise<{ status: number; url: string; text: string; notModified: boolean; headers: Headers }>;
  html(text: string): Document;      // linkedom
  xml(text: string): Document;       // linkedom
  feed(text: string): FeedEntry[];   // RSS 1.0 / 2.0、Atom：{ url, title, published_at?, summary?, image?, content_html?, comments_url? }
  blocks(root: Element, base: string): { blocks: RawBlock[]; links: RawLink[] };   // 把正文的 DOM 变成块
  browser<T>(url: string | null, script: string, args?: unknown): Promise<T>;       // 仅 needs 为 browser / login
}

interface Extracted {
  list: { url: string; title: string; published_at?: number; summary?: string; image?: string }[];
  items: RawArticle[];
  state?: unknown;
}

interface RawArticle {
  url: string; title: string; lang?: string; published_at?: number; byline?: string; summary?: string;
  blocks: RawBlock[];                // { type: "p" | "h2" | "h3" | "quote" | "li" | "code", text } |
                                     // { type: "img", url, alt? } | { type: "table", rows: string[][] }
  comments?: { author?: string; at?: number; text: string }[];
  links?: RawLink[];                 // { url, text }
}

// 失败：throw new SiteError("auth" | "transient" | "program", message)
// SiteError 是运行环境给的全局名字，不用 import；抛出的错误带 kind 字段（"auth" | "transient"）也认，别的都算 program
```

- `ctx.fetch`：统一的 UA，同一主机每秒至多 1 次，跟随重定向，30 秒超时。`conditional: true` 时带上次的 `ETag` / `Last-Modified`，回 304 时 `notModified` 是真、`text` 为空。`401`、`403` 不抛错，由 `extract` 决定是不是 `auth`。网络错误和 5xx 抛 `transient`。
- `ctx.browser(url, script, args)`：经 `browser.background_run`（Agent Browser Protocol §7.1）在后台标签页里跑包里的 `script`（相对来源包的路径，`.mjs`）；`url` 给了就先打开它。`needs` 是 `http` 的来源调它抛 `program`。失败怎么算：脚本抛的错带 `code: "auth"`（任务的错误以 `auth: ` 开头）是 `auth`；浏览器不可用、Agent Service 连不上、站点连不上（`net::ERR_…`、`page.goto` 超时）、整次任务超时是 `transient`；其余（脚本抛错、等页面上的某样东西没等到）是 `program`。
- `list` 是这一次看到的列表（已收录的也在内），用来判断“列表原来有、现在空”；`items` 是这一次取下来的新文章，至多 `limit` 篇。一个来源没有变化时两者都可以是空的，前提是这一次用条件请求确认过没变（返回 `state` 里记着）。
- `extract.ts` 只能 import 包里的相对路径；解析一律用 `ctx.html`、`ctx.xml`、`ctx.feed`、`ctx.blocks`，不自己 import 解析库。
- `state` 至多 64 KiB，存在作业的 `data/` 里。

## 5. `site` 命令

`src/tools/buckyos-agent/commands/site.ts`。输出和退出码同 `reflow`：stdout 一行 AgentToolResult；0 完成，1 参数或包不合法，2 服务拒绝或失败，4 超时，5 本地文件错误。**作为 Goal 作业运行时（环境里有 `OPENDAN_JOB_RUN_ID`）`site sync`、`site reading --compact` 的 stdout 最后一行改为作业的结果行** `{ "summary", "kind"?, "metrics" }`（Agent State Protocol §9.9）。

| 子命令 | 作用 |
| --- | --- |
| `site init [--title ..] [--language ..] [--topics a,b,c] [--owner-did <did>]` | 没有 `site.json` 就建，装默认模板。已有时什么都不改，打印现状。`owner_did` 取 control-panel 的 `owner.context`，取不到时要 `--owner-did` |
| `site status [--json]` | 站点、每个来源的显示状态和最近同步、开着的升级请求 |
| `site match <网址>` | 已收录的文章（给出站内路由）、属于哪个来源、还是都不是 |
| `site config get [<路径>]`、`site config set <路径> <JSON 值>` | 读写 `site.json` 的一个字段（`title`、`language`、`topics`、`columns`、`template.params.*`、`sources.<id>.<字段>`、`reading.enabled`、`limits.*`、`maintainer`），带 `If-Match`，冲突时重读再改，三次不成报错 |
| `site source put <dir> [--state preparing\|active\|failed] [--note ..]` | 校验来源包，写 `_src/<source>/<新版本>/`，更新 `site.json` 里这一项 |
| `site source list`、`site source get <id> <dir>`、`site source rm <id>` | 列出；把桶里当前版本取到空目录；从 `site.json` 去掉这一项并删 `_src/<id>/`、`feed/<id>.json`（文章按保留期清理） |
| `site try <dir> [--limit 3] [--no-translate] [--fixtures] [--record] [--out <dir>]` | 跑取数、核对、（翻译），结果按桶的布局写到本地目录，打印核对结论；不写桶。`--fixtures` 只从包里的 `fixtures/` 取页面，`--record` 把这一次取到的页面存成 `fixtures/`。取不到的图片当场按 `missing` 处理 |
| `site sync <source> [--dir <来源包>] [--time-limit <秒>] [--no-translate]` | 完整跑一遍（§5.1）。不给 `--dir` 时从桶里取当前版本 |
| `site template put <dir>`、`site template list`、`site template use <version>`、`site template get <version> <dir>` | 装一个模板版本并启用（目录里的 `test/` 不装）；列出；切换（回退）；把一个装好的版本连同 `index.html` 取到空目录（换维护者时接手改过的模板，取到的目录 `site template put` 照收） |
| `site preview <dir> [--data <dir>] [--port <n>]` | 在本机起一个静态服务看模板：数据取自 `--data`（`site try` 的输出或 fixture），不给时读桶 |
| `site upgrade open <id> --title .. --summary .. --file <路径> [--source <id>] [--goal <goal_id>]`、`site upgrade close <id>`、`site upgrade dismiss <id>`、`site upgrade list` | 升级请求（§3.1）。`goal_id` 取 `--goal`，没给时取环境里的 `OPENDAN_GOAL_ID`，再没有取 `maintainer.goal_id`。`close` 是升级做完了（`closed`），`dismiss` 是主人说了不做（`dismissed`，和 App 里点“暂不”写的是同一个状态），都记 `decided_at` |
| `site events get [--out <file>]`、`site events put <file>` | 读、整份写 `events.json`（校验后）。给了 `--out` 就把它写成一个文件，改完用 `site events put` 写回；还没有 `events.json` 时写出的是空的一份 |
| `site supplement put <file> --event <id> --adds .. [--no-translate]` | 把一篇补充文章（`RawArticle` 的 JSON）翻译、封装、写进桶，并登记到事件里 |
| `site reading [--days 7] [--json]`、`site reading --compact` | 阅读汇总；`--compact` 把 `_read/` 汇成 `reading.json`（汇总作业跑它）。同一次打开（`article` 和 `opened_at` 相同）的几条记录只算一次打开，停留相加。作业结果行的 `metrics`：`{ records, opens_7d, dwell_ms_7d, articles_7d, profile }`，`profile` 是最近 7 天按停留排的前 3 个标签和前 3 个来源（`tags:科技,经济;sources:news-yahoo-co-jp`），检查脚本拿它判断阅读有没有明显变化 |

| `site visits [--days 7] [--json]` | 主人在这个 Agent 的浏览器里看过什么（`browser.visits`，Agent Browser Protocol §7.3）：按主机、按页面（去掉查询串的网址、标题）列出次数、停留和最近一次；主人把记录关掉了会写明。它问的是这个 Agent 自己的 Agent Service（`OPENDAN_BROWSER_RPC`），这个运行环境连不上时回 `no_browser`；不读写桶 |

来源的作业：`agent-session goal job put src-<source> --goal <goal_id> --dir <来源包> --command "site sync <source> --dir {dir}" --every <every_secs> --timeout 600`。汇总作业：`… job put site-reading --dir <目录> --command "site reading --compact" --every 3600`，`<目录>` 里要有一个文件（比如一份写明这个作业做什么的 README）：空目录装不成作业。

只读权限的 Agent 写不进桶，不能维护个人站：写操作回 `permission_denied`，说明同 `reflow`。

### 5.1 `site sync` 的步骤

| 步骤 | 做法 |
| --- | --- |
| 1 取数 | 跑 `extract.ts`（§4.2） |
| 2 核对 | §5.3。单篇不过就不发这一篇；一次里超过一半不过，整批失败（`program`） |
| 3 图片 | 下载（带 `Referer`，单张至多 8 MiB），有 `magick` 时缩到长边 1280、去元数据、另出长边 480 的小图；没有 `magick` 时存原图（至多 2 MiB，小图用同一张）。取不到的（网络错误、超时、4xx / 5xx）留到下次，同一张连续两次取不到按 `missing` 发布。取到了但不是站里收的图片（SVG、ICO、BMP、回 200 的错误页等，认不出是 JPEG / PNG / GIF / WebP / AVIF 的）当场去掉这一块，不重试、不留占位，文章照常在这一次发布；地址以 `.svg` 结尾的不去取（`ctx.blocks` 也不把它和 `data:` 图片变成块）。取到了、是图片但存不下的（超过大小上限、转换失败）当场按 `missing` 发布 |
| 4 翻译与归类 | §5.2 |
| 5 封装与写入 | 图片、Markdown 先写，拿到 `cyfile`；组 FeedObject、校验、算 ObjId；写文章；每发布 10 篇和结束时重写 `feed` |
| 6 清理 | 每天一次：过保留期的文章（连同它的 Markdown）和没有文章引用的图片，一次至多删 500 个 |
| 7 记录 | 结果行：`{ summary, kind?, metrics: { listed, new, published, rejected, images, tokens, ms, pending, pending_reason } }`。`pending` 是取到了还没发布的篇数，`pending_reason` 是 `null` 或 `feed` 里 `pending.reason` 的值（§3.2）。宿主不保留退出码为 0 的运行的 `kind`，检查脚本靠这两项知道翻译被拒、图片在等 |

- 续跑：没发布完的文章（取到了、还没翻译或还没取完图）记在作业的 `data/pending.json`，下一次先做它们。`--time-limit` 缺省 480 秒：到时不再开始新的一篇，已发布的已经可见。
- 作业的 `data/`（不作为作业运行时是当前目录的 `.site-job/<source>/`）：`state.json`（`extract` 的 `state`、条件请求的 `ETag` / `Last-Modified`、最近 30 篇的正文长度、一天内核对没过的网址）、`pending.json`、`index.json`（这个作业发布过的文章和它们的图片，第 6 步的清理按它做）、`unlisted.json`（写了文章、还没写进 `feed` 的条目）、`tr/<article>.json`（译文缓存）。
- `pending.reason` 的取法：翻译被预算或配置拒绝是 `translate_budget` / `translate_config`；图片等下一次重试是 `images`；译文长度对不上、模型临时出错是 `translate_failed`；时间到了是 `time`。`message` 写明原因。一篇文章的翻译连续三次同步都失败就放弃它。
- 状态怎么变：通过核对 → `ok`，更新 `checked_at`；`auth` → `needs_login`（带 `login_url`，缺省是 `home_url`）；`program` → `failed`；`transient` → 状态不变、只记 `error`（宿主会重试）。失败时旧的 `items` 原样保留。
- 作业不暂停自己：`needs_login`、`failed` 的来源照时间表再试，主人登录后或原站恢复后自动回到 `ok`。
- 退出码：通过核对是 0（翻译被拒时也是 0，结果行的 `kind` 是 `budget`）；其余非 0，`kind` 是 `auth`、`program` 或 `transient`。

### 5.2 翻译与归类

- 一篇一次请求：标题、摘要、各块、评论、链接文字的原文数组进去，等长的译文数组出来，用 `json_schema` 约束；同一次请求给出至多 3 个 `topics` 里的标签，原文没有摘要时补一条。数组长度对不上重试一次，再不行这一篇留到下次。不在 `topics` 里的标签丢掉。
- AICC `helper.llm_chat`，模型 `llm.translate`，`policy: { no_fallback: true }`。没有可用模型、额度、权限的拒绝是配置问题（`pending.reason = translate_config`），不重试，等下一次同步。
- 原文语言和站点语言相同时不译，只归类和补摘要。
- 按原文块的 SHA-256 缓存在作业的 `data/tr/`，更正文章时只译变了的块。
- 用量：每次调用往 `OPENDAN_USAGE_LOG` 追加一行 `{ at_ms, model, run_id, input_tokens, output_tokens, total_tokens }`（和 `xllm` 同一格式，`run_id` 是 `OPENDAN_JOB_RUN_ID`）。没有这个变量时（Work 里试跑）不记。
- 预算：本次运行已用的 token 达到 `OPENDAN_MODEL_BUDGET`，或这个 Goal 的作业当天（站点时区取 UTC）在 `OPENDAN_USAGE_LOG` 里的用量达到 `limits.translate_tokens_per_day` 时，不再翻译：没译的文章不发布（`pending.reason = translate_budget`），第二天继续。

### 5.3 核对

| 规则 | 缺省 | 不过时 |
| --- | --- | --- |
| 列表原来有、现在空（没有 304） | — | 整批失败 |
| 标题为空 | — | 这一篇不发 |
| 正文（文本块的原文合计）少于 `max(min_chars, 最近 30 篇的中位数 × median_ratio)` | 200、0.3 | 这一篇不发 |
| 替换字符（U+FFFD）多于正文的 `max_replacement` | 0.01 | 这一篇不发 |
| 3 篇以上正文完全相同 | — | 这几篇都不发 |
| 一次里超过一半不过（至少 2 篇） | — | 整批失败（`program`） |

最近 30 篇的正文长度记在作业的 `data/state.json`。

## 6. 页面

### 6.1 入口和模板

- `index.html` 很小：读 `/_data/site.json`（`no-cache`），加载 `/_app/<template.version>/app.css`（有就加载）和 `/_app/<template.version>/app.js`（ES module），调用它的默认导出 `start({ site, mount })`。`site.json` 不存在时显示“还没有建站”。取文件遇到 401 时先 `POST /sso_refresh` 再试一次，还不行就重载页面（由主机带去登录，§7）。
- 一个模板版本：`app.js`（必须有）、`app.css` 和它们引用的文件，都在 `_app/<version>/` 下，用相对路径互相引用。不引外部资源，没有构建步骤。`template.json`：`{ "schema": "buckyos.site.template/1", "version", "updated_at", "updated_by" }`，由 `site template put` 写。
- 路由用查询串：`/` 总首页，`/?s=<source>` 来源页，`/?a=<article>` 文章页，`/?e=<event>` 事件页。站内跳转用 History API，不重新加载入口。
- 文本一律 `textContent`；链接只放 `http(s)`；图片 `loading="lazy"`；界面文字跟 `site.json` 的 `language`。
- 阅读上报：文章页隐藏或离开时把这一次的记录（§3.6）发到 `/_read`，用 `sendBeacon` 带 `application/json` 的 Blob，或 `fetch` 带 `keepalive`（不能传字符串，§7）；`reading.enabled` 为假时不报；失败不影响阅读。
- 会话 15 分钟过期，离开时的上报来不及重试：页面可见期间每 10 分钟 `POST /sso_refresh` 一次，保持会话有效。
- 缓存：`_data/**` 用 `no-cache`（靠 ETag 回 304）；`_app/**`、`_data/img/**` 长期缓存。

### 6.2 页面和 App 的消息

页面在 App 的 iframe 里时（`window.parent !== window`）`window.parent.postMessage(消息, '*')`；App 只收自己那个 iframe 的窗口发来、并且来源是 `https://reflow.<zone>` 的消息，发给页面的消息以这个来源作 `targetOrigin`。

| 方向 | 消息 | 含义 |
| --- | --- | --- |
| 页面 → App | `{ "type": "site.ready" }` | 页面画完了 |
| 页面 → App | `{ "type": "site.location", "route": "a=0123…", "title": "…", "url": "<原文网址>"?, "source": "<source>"? }` | 站内位置变了。`route` 是查询串（不带 `?`，首页是空串）；文章页带原文网址，来源页和文章页带来源 |
| 页面 → App | `{ "type": "site.open", "url": "<网址>" }` | 打开一个没收录的链接，或“看原文”：App 在原始页面模式打开它，不请 Agent |
| 页面 → App | `{ "type": "site.add", "url": "<网址>"? }` | 主人想加来源（空站首页、来源页的“加入阅读”） |
| App → 页面 | `{ "type": "site.navigate", "route": "s=news-yahoo-co-jp" }` | 去站内的一页 |
| App → 页面 | `{ "type": "site.updated" }` | 数据变了，页面重新读当前这一页的数据 |
| App → 页面 | `{ "type": "site.theme", "theme": "light" \| "dark" }` | App 的明暗主题（`site.ready` 之后和主题变化时发）；没收到时页面跟系统的 `prefers-color-scheme` |
| 页面 → App | `{ "type": "site.session" }` | 页面的会话续不上了（取文件 401，`/sso_refresh` 也不成）：App 重新走一遍登录（`prepare_app_launch`）再回到当前这一页。页面在 iframe 里时发它而不是自己重载 |

页面不在 iframe 里时（手机浏览器直接打开）：`site.open` 的链接在新标签页打开，`site.add` 的入口不显示。地址栏、代办入口、同步时间、需要登录、升级提示、“看原文”在 App 的外壳里，数据来自 `_data`，模板怎么改都不影响；页面自己也可以显示这些。

## 7. `reflow.<zone>` 主机上新增的两样

（原有的规则见 Agent Browser Protocol §5.4。）

- **未登录的导航请求**：`GET`、`Accept` 含 `text/html`、没有有效会话时，回 200 的一小段 HTML（`Cache-Control: no-store`；每个地址拿到的都是同一份，里面没有请求带来的内容）：它先 `POST /sso_refresh`，成功就重载；不成功就跳到 Zone 的登录页，登录后由 `sso_callback` 种下会话再回来。别的请求（`HEAD`、页面脚本取文件、`POST /_read`）没有会话仍是 401 纯文本。
  - 登录页的地址是 `<当前协议>//<zone>/login?redirect_url=<当前地址>`：`<zone>` 是当前主机去掉开头的 `reflow.` 或 `reflow-`（端口保留），当前地址整个做 `encodeURIComponent`（含查询串和片段）。
  - 登录页对这个主机和对 AI Workspace 的页面是同一套做法。浏览器在 Zone 上已经登录：不显示表单，调 `auth.prepare_app_launch { service_id: "nfs-server", redirect_url }`（Agent Browser Protocol §5.4 里 App 用的那个），直接去它给的 `callback_url`。没有登录：显示表单，`auth.login` 带着 `redirect_url`，登录的目标就是这个主机。用 OAuth、DID 登录的先回到登录页，再按已经登录处理。Zone 不肯把当前登录交给这个主机时（如外部 DID 的会话）显示表单。
  - 同一个标签页里同一个地址 10 秒内第二次拿到这段 HTML（登录了仍然不被接受）时，它不再自动跳转，只显示一个去登录页的链接。
- **`POST /_read`**：按下面的顺序检查，前一项不过就不看后一项；都过了回 204。
  1. 同一种会话（Agent Browser Protocol §5.4 的页面会话，cookie 或 Bearer），没有回 401。
  2. `Origin` 的主机名必须是这个请求的主机（协议和端口不比）；没有 `Origin`、不是一个网址（含 `null`）、对不上都回 403。这个主机的页面带 `Referrer-Policy: no-referrer`，这时 `no-cors` 的 POST 按 Fetch 规范带的是 `Origin: null`：`sendBeacon` 传字符串或 `text/plain` 的 Blob 在 Firefox 上会被拒（157 实测；Chromium 156 仍带真实来源）。页面上报要用 `sendBeacon('/_read', new Blob([json], { type: 'application/json' }))` 或 `fetch('/_read', { method: 'POST', body: json, keepalive: true })`，两种在两个浏览器上都带真实来源。
  3. 每个用户任意 60 秒内至多 30 次（进程内计数），超了回 429。过了前两项的请求都算一次，正文不合格的也算；回 429 的不算。
  4. 正文至多 16 KiB，超了回 413。
  5. 正文是 JSON 数组，1 到 50 条，每条恰好是 §3.6 的五个字段：`article` 是 16 个小写十六进制字符，`source` 是 `<source>`（§1）或 `_supplement`，`opened_at`、`dwell_ms` 是非负整数，`max_scroll` 是 0 到 100 的整数。不是这样的（缺字段、多字段、类型不对、不是数组、空数组）整批回 400。`Content-Type` 不看。
  - 收到的正文原样写成 `_read/<yyyymmdd>/<ulid>.json`（UTC 日期；`<ulid>` 是 26 个字符的大写 Crockford base32，按时间排序），`Content-Type: application/json`，带 90 天的过期时间。这个文件就是那个数组，没有 `schema` 一层。
  - 写入和 `/bucket/v1` 的 PUT 走同一个接口，桶的声明和配额照常生效；写不进去时回桶的错误（JSON，如 507 `quota_exceeded`）。401、403、429、413、400 的正文是纯文本，带 `Cache-Control: no-store`。
  - 这个主机上别的写入都不开：除了 `POST /_read`，`GET`、`HEAD` 以外的方法在任何路径上仍是 405。
