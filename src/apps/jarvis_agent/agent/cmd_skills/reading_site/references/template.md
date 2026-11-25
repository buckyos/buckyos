# The site's pages

A template is the set of static files that draws the site in the owner's browser from the data in the bucket. One version is in use (`template.version` in site.json); `site init` installed the default one. Changing the layout never refetches anything.

#### Look at a page

`site preview` serves a template with the data of a trial (`--data <the --out of site try>`) or, without `--data`, with the bucket's: what the owner has now. It serves until stopped, so the command goes on as a task: stop that task when you have looked (`cancel_task`, or `agent-session task cancel <task_id>`), or the work cannot end.

```sh
site preview /opt/buckyos/bin/opendan/buckyos-agent/site-template --data "$OPENDAN_SESSION_DIR/try-<source>" --port 8765
agent-browser run look.mjs --arg url=http://127.0.0.1:8765/ --arg shot=home.png
agent-browser run look.mjs --arg "url=http://127.0.0.1:8765/?a=<article>" --arg shot=article.png --arg height=2400
llm_understand_media article.png --goal "A phone page of a translated article: is it one readable column with title, source, date, text and pictures in order, and is anything cut off or overlapping?"
```

`look.mjs`, in the session directory, shows one page 390 pixels wide in a tab of its own, as the App does, and leaves the owner's tabs and viewport alone:

```js
export default async ({ context, args }) => {
  const page = await context.newPage();
  try {
    await page.setContent('<body style="margin:0"><iframe id="phone" style="display:block;border:0"></iframe>');
    const drawn = page.evaluate(() => new Promise((done) => {
      addEventListener('message', (event) => event.data?.type === 'site.ready' && done(true));
      setTimeout(() => done(false), 20000);
    }));
    await page.locator('#phone').evaluate((frame, { url, width, height }) => {
      frame.style.width = `${width}px`;
      frame.style.height = `${height}px`;
      frame.src = url;
    }, { url: args.url, width: Number(args.width ?? 390), height: Number(args.height ?? 844) });
    if (!await drawn) throw new Error(`${args.url} drew no page within 20 seconds`);
    if (args.theme) await page.locator('#phone').evaluate((frame, theme) => frame.contentWindow.postMessage({ type: 'site.theme', theme }, '*'), args.theme);
    const site = page.frameLocator('#phone');
    await page.waitForTimeout(500);
    const sideways = await site.locator('html').evaluate((html) => html.scrollWidth > html.clientWidth);
    const text = await site.locator('body').innerText();
    await page.bringToFront();
    await page.locator('#phone').screenshot({ path: args.shot, scale: 'css' });
    return { url: args.url, sideways, screenshot: args.shot, text };
  } finally {
    await page.close();
  }
};
```

The routes are the query string: `/` home, `?s=<source>`, `?a=<article>` (the ids `site try` printed), `?e=<event>`. Read the returned `text` first: the translated title, the paragraphs in order, no leftovers of the original site's furniture; `sideways` must be false. Then the screenshot for what text cannot tell. `--arg theme=dark` shows the dark side.

#### Parameters, not code

What the owner or the goal usually wants needs no new template:

```sh
site config get template.params
site config set template.params.home_order '"events"'
site config set template.params.per_column 12
site config set template.params.collapsed '["jp"]'
site config set columns '[{"id":"jp","title":"日本","sources":["news-yahoo-co-jp"]}]'
```

`home_order` is `time`, `source` or `events`; `per_column` 1–100; `collapsed` lists the column ids folded at first; `columns` is the home page's columns in order (a source in none comes last). `hide_topics` is the owner's alone. A value is JSON: text in double quotes.

#### A new layout

Template code changes on the owner's word only. Copy the default into the workspace once, change the copy, look at every kind of page, install:

```sh
cp -R /opt/buckyos/bin/opendan/buckyos-agent/site-template template
site preview template --port 8765
site template put template
site template list
site template use t1
```

`put` installs the directory as the next version `t<n>` and switches the site to it; `use` goes back to an installed version. The workspace copy is the source of what you install; `site template get <version> template` fetches an installed version when the workspace has none (a template another Agent changed).

What a template must keep (the default's `app.js` and `article.js` are the worked example; read them before changing):

- `app.js` default-exports `start({ site, mount })`: `site` is site.json, `mount` the element to draw into. `app.css` beside it is loaded when present; other files by relative address. `index.html` is the entry every version shares: leave it out of the directory unless the entry itself has to change. No build step; no script, style, font or picture from another host.
- Data comes from `/_data/site.json`, `/_data/feed/<source>.json`, `/_data/a/<article>.json`, `/_data/events.json`, `/_data/reading.json`, pictures from the `_data/img/…` keys in them, loaded lazily. Fetch with `cache: 'no-cache'`; on 401, `POST /sso_refresh` once and try again.
- Text that came from a website is set as text (`textContent`), never as HTML. Links are http(s) only; one that leaves the site is handed to the App (`site.open`) when the page is embedded.
- With the App (`window.parent !== window`): send `site.ready` once drawn, `site.location { route, title, url?, source? }` on every page, `site.open { url }`, `site.add { url? }`, `site.session` when the session cannot be renewed; take `site.navigate { route }`, `site.updated` and `site.theme { theme }` from the parent.
- The article page reports what was read, unless `reading.enabled` is false: `{ article, source, opened_at, dwell_ms, max_scroll }` in an array to `/_read` when it is hidden or left, `navigator.sendBeacon('/_read', new Blob([json], { type: 'application/json' }))`. The goal's reading analysis lives on it.
- Every state has a face: a source being set up, failed, needing sign-in, needing an upgrade, paused; articles waiting (`status.pending.reason`); a picture that is missing; a site without sources. Interface words follow `site.language`; `template.params` are honoured and unknown ones ignored.
- One column on a phone without sideways scrolling at 375 pixels, light and dark, a first screen under 100 KB without pictures.
