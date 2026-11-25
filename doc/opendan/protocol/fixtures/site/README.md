# 个人站的共用样例

契约见 [Personal Site Protocol](<../../Personal Site Protocol.md>)。TS（`src/tools/buckyos-agent/lib/site/`）、默认模板、Desktop 的 mock、Rust 的测试读同一份，改契约时一起改。

| 文件 | 是什么 |
| --- | --- |
| `site.json` | `_data/site.json`：三个来源（一个接入失败）、一条开着的升级请求 |
| `feed.json` | `_data/feed/news-yahoo-co-jp.json`：三条，其中一条的标题和摘要带 HTML 标记（页面只当文字显示） |
| `article.json` | `_data/a/7eec8758d56a244a.json`：每种块各一个，含一张没取到的图 |
| `events.json`、`reading.json` | `_data/events.json`、`_data/reading.json` |
| `read_batch.json` | `POST /_read` 的正文 |
| `feed_object_vectors.json` | FeedObject 外壳的校验与 ObjId 向量（§3.4） |

文章 id 是规范化网址 SHA-256 的前 16 位，和 `../reflow_vectors.json` 同一算法。
