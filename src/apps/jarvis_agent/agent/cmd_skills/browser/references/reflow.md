# Reflow: fallback pages and site packages

`reflow --help` has the commands, the package format and the exit codes. Every `reflow` command prints one result: `summary` is the answer in words, `detail` its structure.

## What the user gets

- The reflow library belongs to the user, not to you: it stays when an Agent is removed, and any Agent of the user can add to it. Per site it holds a package, the results made with it, fallback pages and the user's preference for the site: in the package's `manifest.json`, or in `preference.json` while the site has no package (the app writes it). `<site>` is the host name with dots as dashes; `reflow match <url>` prints it together with the page id and the preference.
- For an address the app shows the package's page when the library has a matching package and a result for the page, otherwise the fallback page when there is one; `reflow match <url>` says which under `view`. With a package and no result yet the app starts a refresh itself; with nothing to show, or when that refresh fails, it asks you in the browser conversation.
- The app shows it in a frame from `https://reflow.<zone>/`, to the signed-in user only, with the time it was made. You cannot open that address yourself: it takes the user's session, not an Agent's. Check your work with `reflow match` and the preview below.
- Nothing is live: a page is made ahead and read later, also when your browser is not running.

## One rule for every page you write

The page runs on an address that carries the user's session. Whatever comes from the site is data:

- Put it in as text: `textContent` in a package's page, `&amp;` `&lt;` `&gt;` in a fallback page.
- Never copy the site's markup, scripts, styles, `on…` attributes or `javascript:` links into your page, and never set `innerHTML` from site data. A link is an `http(s)` address or it is left out.
- The only script in a page is the one you wrote. Load nothing from other sites (scripts, styles, fonts); a package brings its own files.

## A fallback page

For one address, when the site has no package or its refresh failed.

```
reflow match <url>
agent-browser open <url>
agent-browser snapshot
```

Read the original page (`snapshot`, or a `run` script that returns the parts you need), keep what the user's preference asks for (in the request; `reflow match <url>` prints the one the library has for the site), and write one self-contained file. Text first; an image only when the preference wants it, by its original absolute address.

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Old site: a headline</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  img { max-width: 100%; }
</style>
</head>
<body>
<h1>Old site: a headline</h1>
<p>2026-11-03</p>
<p>The text of the page, with &amp;, &lt; and &gt; written as entities.</p>
<p><a href="https://www.example.com/news/2">Next page</a></p>
<script>
document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  event.preventDefault();
  parent.postMessage({ type: 'reflow.open', url: link.href }, '*');
});
parent.postMessage({ type: 'reflow.ready' }, '*');
</script>
</body>
</html>
```

The script hands links to the app, which shows them reflowed when the library has something for them. Then keep the page and check:

```
reflow result put <url> fallback.html
reflow match <url>
```

`match` now shows `fallback: made …` and a `view`. Tell the user the page is ready. When the same site keeps coming back, write a package.

## A site package

A directory named as you like, usually `<site>`:

| file | runs | what it does |
| --- | --- | --- |
| `manifest.json` | — | which addresses the package covers, the user's preference |
| `refresh.mjs` | in your browser, on the original page | takes the content out and returns it as data |
| `index.html` and the files it uses | on the user's device | draws the data of one page |

`manifest.json` (`reflow put` adds `updated_at`, and `author` when there is none):

```json
{
  "schema": "buckyos.reflow/1",
  "site": "www-example-com",
  "match": [{ "host": "www.example.com", "path": "/news/*" }],
  "preference": "Title, date and text only, one column for a phone",
  "refresh": { "script": "refresh.mjs", "timeout_s": 60 }
}
```

- `site` is the `<site>` of every `match[].host`. `host` is the exact host name in lower case; `www.example.com` and `example.com` are two sites with a package each.
- In `path`, `*` matches anything, `/` included; the rest is literal and has to match from start to end, so `/news/*` does not cover `/news`. Without `path` the rule covers every page of the host. Cover only the kind of page your code handles; other pages of the site get fallback pages.
- `preference` is the user's wish in their words. Write it into the manifest when you write a package: it is in the request, and `reflow match <url>` prints the one the library has for the site (its package's, or `preference.json` of a site without one). A manifest that states none gets that one from `put`. When the user changes it, change it here and the code with it.
- `timeout_s` is 1 to 600.

`refresh.mjs` gets the original page already open at `url`, with the user's sign-ins, in a background tab:

```js
export default async ({ page, url, save }) => {
  await page.locator('h1').first().waitFor();
  const data = await page.evaluate(() => {
    const text = (selector) => document.querySelector(selector)?.textContent.trim() ?? '';
    return {
      title: text('h1'),
      date: text('.date'),
      paragraphs: [...document.querySelectorAll('#content p')].map((p) => p.innerText.trim()).filter(Boolean),
      image: document.querySelector('#content img')?.src ?? null,
      links: [...document.querySelectorAll('a.next')].map((a) => ({ text: a.textContent.trim(), url: a.href })),
      comments: document.querySelector('#load-comments') ? { action: { selector: '#load-comments' } } : null,
    };
  });
  if (data.image) data.image = await save.image(data.image);
  return data;
};
```

- Wait for the one thing every covered page has, then read the rest in one `page.evaluate`: waiting for a part that some pages lack runs into the timeout and fails the refresh (an action waits a third of `timeout_s`, 15 seconds at most; the failure names the selector).
- It only reads. No signing in, no submitting, no clicking around.
- It is one file: a refresh runs `refresh.mjs` alone, without the package's other files, so it cannot `import` them. It runs outside your session too: `process.env` has no token and no `OPENDAN_*` or `BUCKYOS_*` variable.
- What it returns has to be JSON and stay small: the user's phone loads it whole. `save.image(address)` fetches a picture with the page's cookies, scales it down and returns it as a `data:` address to put into the data; use it for the few pictures the page needs.

`index.html` is opened as `<site>/?url=<original address>&page=<page id>` and reads `results/<page id>.json` next to itself: `{ "schema", "source_url", "made_at", "made_by", "data" }`, where `data` is what `refresh.mjs` returned.

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Reflow</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  img { max-width: 100%; }
  .date { color: #666; }
  button { font: inherit; }
</style>
</head>
<body>
<main id="main"></main>
<script>
const query = new URLSearchParams(location.search);
const main = document.getElementById('main');
const tell = (message) => parent.postMessage(message, '*');

function add(tag, text, into = main) {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  into.append(node);
  return node;
}

async function result() {
  const file = `results/${query.get('page')}.json`;
  let response = await fetch(file, { cache: 'no-store' });
  if (response.status === 401) {
    await fetch('/sso_refresh', { method: 'POST' });
    response = await fetch(file, { cache: 'no-store' });
  }
  if (!response.ok) throw new Error(`${file}: HTTP ${response.status}`);
  return response.json();
}

async function draw() {
  const { data } = await result();
  main.replaceChildren();
  add('h1', data.title);
  add('p', data.date).className = 'date';
  if (data.image) add('img').src = data.image;
  for (const paragraph of data.paragraphs) add('p', paragraph);
  if (data.comments) {
    add('button', 'Comments').onclick = () => tell({ type: 'reflow.action', selector: data.comments.action.selector });
  }
  for (const link of data.links) {
    if (!/^https?:/.test(link.url)) continue;
    const a = add('a', link.text, add('p'));
    a.href = link.url;
    a.onclick = (event) => {
      event.preventDefault();
      tell({ type: 'reflow.open', url: link.url });
    };
  }
  tell({ type: 'reflow.ready' });
}

addEventListener('message', (event) => {
  if (event.source === parent && event.data?.type === 'reflow.updated') draw();
});
draw().catch((error) => add('p', `Cannot show this page: ${error.message}`));
</script>
</body>
</html>
```

- A `401` means the session on this address ran out: `POST /sso_refresh` once, then read again.
- Design for a phone: one column, the viewport `meta`, large text.
- What the page tells the app, with `parent.postMessage(message, '*')`:

| message | when |
| --- | --- |
| `{ "type": "reflow.ready" }` | the page is drawn |
| `{ "type": "reflow.open", "url": "<address>" }` | the user follows a link; the app decides how to show it |
| `{ "type": "reflow.action", "selector": "<selector>" }` | the user wants something only the original page can do |
| `{ "type": "reflow.refresh" }` | the user wants this page made again |
| `{ "type": "reflow.source" }` | the user wants the original page |

  The app answers `{ "type": "reflow.updated" }` when the result changed: read it again.

### Actions that need the original page

"Load more", a tab that fetches, a vote: `refresh.mjs` records where the control is on the original page, as `{ "action": { "selector": "<CSS selector>" } }` at a place in the data your page knows (`comments` above). Your page shows its own button and sends `reflow.action` with that selector. The app then has your browser open the original address, click the first element the selector matches, wait until the page settles, run `refresh.mjs` again and keep the new result.

So the selector has to find the control on the freshly opened page, and `refresh.mjs` has to return what the page shows after the click (the loaded comments) when they are there. What one click cannot do — a form, several steps — stays with the original page: offer `reflow.source`.

### Try it before it goes into the library

Keep two helper scripts beside the package directory, not inside it (everything in the directory is uploaded, hidden files excepted).

`try.mjs` runs the package's `refresh.mjs` on a page of the site and prints its data. Its `save.image` is a stand-in that returns the picture's address; the real one returns the picture itself:

```js
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export default async ({ page, args }) => {
  const { default: refresh } = await import(pathToFileURL(path.resolve(args.dir, 'refresh.mjs')).href);
  await page.goto(args.url);
  const save = { image: async (source) => new URL(source, page.url()).href };
  return refresh({ page, url: args.url, save });
};
```

`preview.mjs` opens the package's `index.html` with that data in a tab of its own, the way the app does, and returns the text of the page and a screenshot:

```js
import fs from 'node:fs';
import path from 'node:path';

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png' };

export default async ({ context, args }) => {
  const dir = path.resolve(args.dir);
  const result = { schema: 'buckyos.reflow.result/1', source_url: args.url, made_at: Date.now(), made_by: 'preview', data: JSON.parse(fs.readFileSync(args.data, 'utf8')) };
  const page = await context.newPage();
  await page.route('https://reflow.invalid/**', (route) => {
    const name = new URL(route.request().url()).pathname.slice('/site/'.length) || 'index.html';
    if (name === 'results/preview.json') return route.fulfill({ json: result });
    const file = path.join(dir, name);
    if (!file.startsWith(dir + path.sep) || !fs.existsSync(file)) return route.fulfill({ status: 404, body: 'not found' });
    return route.fulfill({ body: fs.readFileSync(file), contentType: TYPES[path.extname(file)] ?? 'application/octet-stream' });
  });
  await page.goto(`https://reflow.invalid/site/?url=${encodeURIComponent(args.url)}&page=preview`);
  await page.locator('#main > *').first().waitFor();
  await page.waitForFunction(() => [...document.images].every((image) => image.complete));
  const text = await page.locator('body').innerText();
  await page.bringToFront();
  await page.screenshot({ path: args.shot });
  await page.close();
  return { text, screenshot: path.resolve(args.shot) };
};
```

```
agent-browser run try.mjs --arg dir=www-example-com --arg url=https://www.example.com/news/1 > data.json
agent-browser run preview.mjs --arg dir=www-example-com --arg data=data.json --arg url=https://www.example.com/news/1 --arg shot=preview.png
llm_understand_media preview.png --goal "is this one readable column with title, date and text?"
```

Try a second and a third page of the site: a package is worth it only when it holds for the pages it claims in `match`. In the preview's text, a tag from the site has to show as text.

### Into the library

```
reflow put www-example-com www-example-com
reflow refresh https://www.example.com/news/1
reflow match https://www.example.com/news/1
```

- `put <site> <dir>` checks the package and replaces the site's package with it: files the new one lacks are removed, results and fallback pages stay. The site's `preference.json` is removed as well: from now on the manifest carries the preference. A file of that name cannot be in the package directory, and `get` does not fetch it.
- `refresh` makes the result with the real `refresh.mjs` in a background tab of your browser and waits for it (`--timeout <seconds>`, default 120; `--no-wait` to start it and go on). `match` then shows `result: made …` and the `view`.
- A package with only `index.html` cannot refresh: make the data yourself and keep it with `reflow result put <url> data.json`.
- Pages the user reads regularly you may refresh ahead of time with `reflow refresh <url>`; doing that on a timetable is a goal's job.
- Later changes start from the library's copy: `reflow get <site> <dir>` into a new or empty directory, change, try, `put`, `refresh`. That is also the way when the user changes the preference, and when the site changed and refresh fails: make a fallback page for the address the user is waiting on first, then repair the package.
- `reflow rm <site>` removes the package, the preference, the results and the fallback pages of a site: only when the user asks.

### A read-only Agent

`put`, `rm` and `result put` fail with `permission_denied` when the owner made this Agent read-only: it may not write the user's library. Keep what you made under `$OPENDAN_AGENT_ROOT` (not in a temporary directory), give its Zone path (`/home/<owner>/agents/<agent_id>/…`) in your reply or report, and ask the user to import it in the Agent Browser app. `match`, `ls` and `get` still work. A refresh you start cannot keep its result either; one the user starts in the app can.

## When it fails

- `reflow => invalid`: the package breaks a rule; `detail.problems` lists each one.
- `reflow => no_library`: this Zone has no reflow library yet; that is for the owner or an administrator, not something a package can fix.
- `reflow refresh => no_browser`: the session is not on the Agent's private runtime. `reflow_no_package`: nothing in the library matches the address — `put` the package, or keep a fallback page.
- `reflow refresh => refresh_failed`: `refresh.mjs` threw or ran out of time on the real page; the reason is in the summary. Run `try.mjs` on that address to see it.
- Exit code 4: the refresh was still running after `--timeout`; `reflow match <url>` shows the result once it is made.

Rules of the library and of the pages: BuckyOS repo `doc/opendan/protocol/Agent Browser Protocol.md` §5.

Checked against: BuckyOS 0.7.0 working tree, ext-core 1.3.0 payload, 2026-11-15: the package, both helper scripts and both pages above ran against a real Chromium (the pages in a frame with a stand-in for the app and for `reflow.<zone>`), `refresh.mjs` also under the runner the Agent Service uses, and the package passed the checks of `reflow put`. Not yet run against a Zone's library.
