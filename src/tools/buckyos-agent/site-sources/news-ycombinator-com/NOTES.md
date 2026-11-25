# news-ycombinator-com — Hacker News, front page

Read on 2026-11-25 without a browser and without signing in.

## What an article is here

Hacker News is a list of links with a discussion each. An article of this source is one story:

- its address (`url`) is the discussion page, `https://news.ycombinator.com/item?id=<n>`: that is the page that belongs to this source, and where "read the original" goes;
- its body is the text of the page the story links to, read with `ctx.blocks`; for a story without a link (Ask HN, Show HN text posts) it is the post's own text;
- its comments are the first top-level comments of the discussion, in the site's order (`comments` of the source, 10 by default), without the replies;
- its first link is the linked page; the byline is that page's host name (the submitter for a text post);
- its language is the linked page's `<html lang>`; `en` when it has none.

A story whose link is not a page of text — a video, a PDF, a page that needs scripts, a page that refuses the request, fewer than 400 characters of text — is skipped and remembered in the state (`skipped`, the last 300), so it is not fetched again on every sync. It stays in `list`.

## How it is read

1. `https://news.ycombinator.com/rss` (RSS 2.0, 30 entries, no `ETag`: every sync fetches it). `<link>` is the linked page, `<comments>` the discussion (`ctx.feed` gives it as `comments_url`).
2. The discussion page: `.fatitem .titleline > a` (title and link), `.fatitem a[href^="user?id="]` (submitter), `.fatitem .age[title]` (time, UTC), `.fatitem .toptext` (a text post's body); comments are `tr.athing.comtr` rows whose `td.ind` has `indent="0"`, with `.commtext`, the first `a[href^="user?id="]` and `.age[title]`.
3. The linked page, on its own host: the longest `<article>` when it has 400 characters of text, otherwise `<main>` / `[role=main]`, otherwise `<body>`.

Requests for one new story: 2 pages (discussion, linked page) and the linked page's images. `max_new` is 10 in `source.json`: a first sync of 30 stories would make some 60 page requests.

## What to check

- `site try . --fixtures --no-translate`: 3 articles (a magazine article, a status page, a repository's README), each with comments and authors.
- The linked pages are anybody's markup. Expect some of a site's own furniture in the body (section labels, "most popular" lists inside `<article>`); when one site that shows up often reads badly, give it its own rule in `linked()` rather than tightening the rule for all.
- The length check counts the linked page's text: `check.min_chars` is 400, the same number `extract.ts` skips at.

## What goes wrong

- The discussion page asks to slow down ("Sorry, we're not able to serve your requests this quickly"): HTTP 200 with no `.fatitem`. The run fails as `program` after one such page; lower `max_new` or sync less often.
- `indent` or the class names change: comments come out empty or include replies. Look at one discussion page.
- A linked site answers 403 to programs: the story is skipped. Nothing is done about that.

## Fixtures

`fixtures/` holds the feed cut to three entries, their discussion pages and the three linked pages, with scripts, styles, SVG, comments, `<link>` and `style` / `data-*` / `aria-*` / `srcset` attributes removed, and the longest discussion cut after 40 comment rows.
