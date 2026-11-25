# dv-live-news — a source that needs the browser to render

The news section `/live` of the DV test site (`test/agent_browser_dv/site.mjs`). It stands for a site whose pages are drawn by their own script: fetched as plain HTML, `/live` and `/live/story/<n>` are an empty `<div id="app">` and a `<script>`; there is no feed and no interface to ask instead. So `source.json` says `"needs": "browser"`.

## How it is read

1. `ctx.browser(<home_url>, "render.mjs")` opens `/live` in a background tab and returns `{ url, html }`: the page after its script ran (`page.content()` once the network is idle). `render.mjs` looks for nothing in the page.
2. `extract.ts` parses that HTML with `ctx.html`: the list is `.stories .story`, each with `a.story-link` (address, title) and `time[datetime]`.
3. Each new story is rendered the same way; the article is `article.story-body` with `.story-title`, `.byline .author` and the paragraphs under `.story-text` (through `ctx.blocks`).

One background run per page: the list, and one per new story. Each takes about a second on top of the page.

All the selectors are in `extract.ts`, none in the browser script: when the markup changes, the failure is a `program` error that names the selector, and the fix is in one file. A wait for a selector in `render.mjs` would fail as a timeout of the browser instead, which reads like the site being slow.

## What goes wrong

- `the drawn page has no ".stories .story"`: the list's markup changed. With `/__markup?v=2` the test site keeps its pages and renames the classes (`.feed .feed-item`, `a.feed-item__title`, `article.post`, `.post__title`, `.post__meta .by`, `.post__content`): a repair of the selectors. With `/__markup?v=3` it becomes another site (cards without links, articles at `/live/read?id=<n>`): that is an upgrade, not a repair.
- `browser.background_run: …net::ERR_CONNECTION_REFUSED…`: the site is down; `transient`, tried again by itself.
- No browser on this runtime (`browser_unavailable`): `transient`; the status says why.

## Running it without an Agent

`test/agent_browser_dv/README.md`, "Sources that need the browser": the test site, `browser_stub.mjs` in place of the Agent Service, then `site try`.
