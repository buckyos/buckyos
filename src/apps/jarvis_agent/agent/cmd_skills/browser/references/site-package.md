# Package for a recurring site

Store these files in one directory.

#### manifest.json

```json
{
  "schema": "buckyos.reflow/1",
  "site": "www-example-com",
  "match": [{ "host": "www.example.com", "path": "/news/*" }],
  "preference": "Title, date and text only, one column for a phone",
  "refresh": { "script": "refresh.mjs", "timeout_s": 60 }
}
```

The site ID is the lowercase host with dots replaced by dashes. Use the exact host in `match[].host`; apex and www are separate sites. Match only supported pages. The path pattern is literal apart from `*`, which spans slashes. `/news/*` excludes `/news`. Omit path to cover the host. Put the user's preference in their own words. Before a package exists the app saves it in `preference.json`. If omitted from the manifest, `put` inherits it. `timeout_s` is 1–600. `put` sets `updated_at` and a missing `author`.

#### refresh.mjs

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

The user's original page is open in a signed-in background tab. Read only. Wait for a selector common to all matching pages, then extract optional fields in one evaluation. Action timeout is one third of `timeout_s`, capped at 15 seconds. The runner loads only this file and exposes no session token or `OPENDAN_*` / `BUCKYOS_*` environment. Return small JSON. `save.image(url)` fetches with the page's cookies and returns downscaled image data for the few pictures needed.

#### index.html

The app opens `<site>/?url=<original address>&page=<page id>`. Read `results/<page id>.json` (`schema`, `source_url`, `made_at`, `made_by`, `data`). Use a single mobile column and render site text with `textContent`. On 401, POST to `/sso_refresh` and retry once.

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

Send `reflow.ready` after drawing, `reflow.open` for HTTP(S) links, `reflow.refresh` to remake the result and `reflow.source` to open the original. On `reflow.updated` from the parent, reload the data.

For one-click actions (`reflow.action`), save a CSS selector in the refresh result and show a button in the rendered page. The app reopens the original, clicks the first matching element, waits for it to settle and refreshes. The selector must work on a freshly opened page. Extract the post-click state as well, e.g. the actual loaded comments. Send multi-step actions and forms to the original page.

#### Verify and install

Before `put`, read `$OPENDAN_AGENT_ROOT/cmd_skills/browser/references/preview.md` to test extraction and presentation on several matching URLs. Then run:

```sh
reflow put www-example-com www-example-com
reflow refresh https://www.example.com/news/1
reflow match https://www.example.com/news/1
```

`put` validates and replaces the site's package files, keeping results and fallbacks. It removes the old `preference.json` as the manifest now carries the preference. Do not include that file in the package; `get` omits it. `refresh` waits up to 120 seconds by default (`--timeout <seconds>` or `--no-wait`). Look for `result: made …` and the selected `view` in `match`. For an HTML-only package, supply the data with `reflow result put <url> data.json`.
