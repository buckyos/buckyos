# Test a reflow package before putting it into the library

Keep both scripts beside the package directory. Non-hidden files inside the package are uploaded.

`try.mjs` runs the real extraction on the site. Its image helper returns an absolute URL in place of downscaled image data.

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

`preview.mjs` serves the package and sample result in a temporary tab. It returns the page text and a screenshot.

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

```sh
agent-browser run try.mjs --arg dir=www-example-com --arg url=https://www.example.com/news/1 > data.json
agent-browser run preview.mjs --arg dir=www-example-com --arg data=data.json --arg url=https://www.example.com/news/1 --arg shot=preview.png
llm_understand_media preview.png --goal "is this one readable column with title, date and text?"
```

Test a second and third URL covered by the manifest. Check tags from the site display as literal text.

Previous verification (BuckyOS 0.7.0 / ext-core 1.3.0, 2026-11-15): both scripts and example pages ran against real Chromium with a stand-in for the app and reflow frame. Extraction also ran in the Agent Service runner, and the package passed `reflow put` checks. This example was not exercised against a Zone library.
