# news-web-nhk — NHK News, headlines with their lead

Read on 2026-11-25 without a browser and without signing in.

## What this source is, and why it is not the full text

NHK moved its news from `www3.nhk.or.jp/news/` to `news.web.nhk` ("NHK ONE"). The old feed `https://www3.nhk.or.jp/rss/news/cat0.xml` still answers but has not changed since 2026-08-08, and the old article addresses redirect to the new site.

On the new site an article page carries only the lead: the first paragraph and the beginning of the second, cut with "…". The rest appears after the reader confirms how they use the service (a dialog: "このページを見るにはご利用意向の確認をお願いします", or signing in). A program does not answer that dialog for the user, so this source keeps the headline, the lead and the picture, and "read the original" takes the reader to NHK. The owner was to be told so when the source was added.

To get the full text the source has to become `"needs": "login"`: the owner confirms once in the Agent's browser, and `extract.ts` reads the article with `ctx.browser`. That is an upgrade (it starts the browser on every sync), not a repair.

## How it is read

1. `https://news.web.nhk/n-data/conf/na/rss/cat0.xml` (RSS 2.0, about 7 entries, `ETag`, `Last-Modified`), fetched conditionally. `<description>` is the lead, cut with "…".
2. For each new entry the article page is fetched for its JSON-LD (`<script type="application/ld+json">` with `"@type": "NewsArticle"`): `headline.@value`, `image[0].url` (1920×1080), `datePublished`. The paragraphs are the feed's lead, split at line breaks, without the cut last part.

Requests for one new article: 1 page (370 KB) and 1 image.

`check.min_chars` is 40 in `source.json`: a lead is about 100 characters, and the default of 200 would reject every article.

## What goes wrong

- No NewsArticle in the JSON-LD: the page changed; the feed alone still has title, lead and time.
- The lead getting longer or the dialog disappearing would mean the full text is back in the page: then read `<main>` with `ctx.blocks` and raise `min_chars` again.

## Fixtures

`fixtures/` holds the feed cut to three entries and the three pages reduced to their `<head>`, the JSON-LD scripts, the lead paragraphs and the dialog's sentence. The real pages are 370 KB each, almost all of it lists of other articles.
