# news-yahoo-co-jp — Yahoo! JAPAN News, main topics

Read on 2026-11-25 without a browser and without signing in.

## How it is read

1. `https://news.yahoo.co.jp/rss/topics/top-picks.xml` (RSS 2.0, 8 entries, `ETag`, `max-age=60`), fetched conditionally: a 304 ends the run with nothing to do.
2. An entry's `<link>` is a topic page (`/pickup/<n>`), not the article. The article is named by `<comments>` (`/articles/<40 hex>/comments`); when the feed has none, the topic page is fetched and the first `/articles/<40 hex>` link in it is taken. The article URL is what the site keeps (`list[].url`, `items[].url`).
3. The article page embeds its data as `window.__PRELOADED_STATE__ = {...}` in a `<script>`. `articleDetail` has `headline`, `media.mediaName` (the byline), `createDateTime`, `maxPage` and `paragraphs[]`; a paragraph has `objectItems[]` (images: `resizedViewUrl.jpeg`, `caption`, `credit`) and `textDetails[].paragraphItems[]` (`type`, `text`; the text separates paragraphs with a blank line). `relatedLink[]` become the article's links. A long article has more pages: `?page=2` … `maxPage` (at most 5 are read).
4. Comments are not in the article page. `<article>/comments` embeds the first 10 of the site's default order as `commentFull.userCommentList[]` (`name`, `text`; the time is relative, "1時間前", and is left out). It is fetched only when the source's `comments` is not 0 and `articleDetail.isCommentVisible` is true.

Requests for one new article: 2 (article, comments), more for a long article, plus its images. A run that finds nothing new makes one request.

## What to check

- `site try . --fixtures --no-translate`: 3 articles, each with a headline, paragraphs and 10 comments.
- The body text carries zero-width characters (U+200B–U+200D, U+2060) between words; `clean()` removes them. Translations that keep stray invisible characters mean the list of characters has to grow.
- `articleDetail.bodyLength` is the site's own count of the body: compare when the body looks short.

## What goes wrong

- No `__PRELOADED_STATE__`, or no `articleDetail.headline`: the page was rebuilt. Look for the body in the new markup before thinking of a browser.
- HTTP 403 for every page from an address in the EEA or the UK: the site does not serve those regions. That is not something a program gets around; report it.
- Video-only topics (`articleDetail.hasVideo`, almost no text) fail the length check and are left out; that is intended.
- The images are 640 px wide at most (`…-view.jpg`); the larger renditions are behind the photo page.

## Fixtures

`fixtures/` holds the feed cut to three entries, and for each of them the article and comments pages reduced to the `<script>` with the part of `__PRELOADED_STATE__` the extractor reads (`articleDetail` without `richBody`; `commentFull.userCommentList` with the text fields). The real pages are 150 KB and 400 KB each.
