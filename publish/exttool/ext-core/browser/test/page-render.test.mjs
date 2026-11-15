// page-render with a real Chromium.
//   node --test "publish/exttool/ext-core/browser/test/*.test.mjs"
// playwright-core has to be resolvable (NODE_PATH in development) and its Chromium installed (PLAYWRIGHT_BROWSERS_PATH).
// The PDF checks use poppler's pdfinfo and pdftotext.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { after, before, test } from 'node:test';

import { loadPlaywright, readVersions } from '../runtime.mjs';
import { BROWSER_DIR, close, listen, node, PAGE_RENDER, png, pngSize, tempDir } from './helpers.mjs';

let dir;
let server;
let origin;
const hits = [];

function pageRender(args, env) {
  return node(PAGE_RENDER, args, { cwd: dir, env });
}

function write(name, text) {
  const file = path.join(dir, name);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
  return file;
}

function poppler(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  assert.equal(result.status, 0, `${command}: ${result.error?.message ?? result.stderr}`);
  return result.stdout;
}

function lastLine(text) {
  return text.trimEnd().split('\n').at(-1);
}

before(async () => {
  dir = tempDir('page-render-test');
  server = http.createServer((request, response) => {
    hits.push(`${request.headers.host}${request.url}`);
    if (request.url.endsWith('.png')) {
      response.setHeader('content-type', 'image/png');
      return response.end(png(40, 20, [0, 120, 200]));
    }
    if (request.url === '/missing') {
      response.statusCode = 404;
      return response.end('not here');
    }
    response.setHeader('content-type', 'text/html; charset=utf-8');
    return response.end(`<!doctype html><meta charset="utf-8"><title>在线页面</title><h1>在线页面</h1>
<img src="/own.png"><img src="http://localhost:${server.address().port}/other-host.png">`);
  });
  server.on('upgrade', (request, socket) => {
    hits.push(`${request.headers.host}${request.url} (websocket)`);
    socket.destroy();
  });
  origin = `http://127.0.0.1:${await listen(server)}`;

  write(
    'book/index.html',
    `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>系统渲染</title>
<style>section { break-after: page; } .cover { background: #cde; height: 100mm; }</style></head><body>
<section class="cover"><h1>第一章 中文标题</h1><p>中英混排 mixed text，第一页。</p><img src="dot.png"></section>
<section><h1>第二章</h1><p>第二页的内容。</p></section>
<section><h1>第三章</h1><p id="late">还没排好</p></section>
<script>setTimeout(() => { late.textContent = '排好之后的内容'; window.ready = true; }, 300);</script>
</body></html>`,
  );
  fs.writeFileSync(path.join(dir, 'book', 'dot.png'), png(8, 8, [200, 0, 0]));
  write('header.html', '<div style="font-size: 9px; width: 100%; text-align: center">页眉：<span class="title"></span></div>');
  write('footer.html', '<div style="font-size: 9px; width: 100%; text-align: center">第 <span class="pageNumber"></span> 页，共 <span class="totalPages"></span> 页</div>');
});

after(async () => {
  await close(server);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('--version names the script, the browser and playwright-core', async () => {
  const result = await pageRender(['--version']);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /^page-render \d+\.\d+\.\d+ chromium \d+\.\d+\.\d+\.\d+ playwright-core \d+\.\d+\.\d+\n$/);
});

test('a multi-page Chinese document becomes an A4 PDF with header, footer and page numbers', async () => {
  const out = path.join(dir, 'out', 'book.pdf');
  const result = await pageRender(['--html', 'book', '--pdf', out, '--paper', 'A4', '--margin', '15', '--header-html', 'header.html', '--footer-html', 'footer.html', '--background', '--wait-ready', 'window.ready === true']);
  assert.equal(result.code, 0, result.stderr);
  const summary = JSON.parse(result.stdout);
  assert.deepEqual(summary, { pages: 3, bytes: fs.statSync(out).size, sandbox: summary.sandbox, warnings: [] });
  assert.equal(typeof summary.sandbox, 'boolean');
  const info = poppler('pdfinfo', [out]);
  assert.match(info, /^Pages:\s+3$/m);
  assert.match(info, /^Page size:.*\(A4\)$/m);
  const text = poppler('pdftotext', [out, '-']).replace(/[ \t]+/g, '');
  assert.ok(text.includes('中英混排mixedtext，第一页。'), text);
  assert.ok(text.includes('排好之后的内容'), text);
  assert.ok(!text.includes('还没排好'), text);
  assert.ok(text.includes('页眉：系统渲染'), text);
  for (const number of [1, 2, 3]) assert.ok(text.includes(`第${number}页，共3页`), text);
});

test('paper size and orientation', async () => {
  const custom = path.join(dir, 'out', 'custom.pdf');
  assert.equal((await pageRender(['--html', 'book/index.html', '--pdf', custom, '--paper', '100x150mm'])).code, 0);
  // 100 x 150 mm is 283.5 x 425.2 pt, A5 across is 595.3 x 419.5 pt; the browser rounds to whole pixels.
  assert.match(poppler('pdfinfo', [custom]), /^Page size:\s+28[34](\.\d+)? x 42[56](\.\d+)? pts/m);
  const wide = path.join(dir, 'out', 'wide.pdf');
  assert.equal((await pageRender(['--html', 'book/index.html', '--pdf', wide, '--paper', 'a5', '--landscape'])).code, 0);
  assert.match(poppler('pdfinfo', [wide]), /^Page size:\s+59[56](\.\d+)? x 4[12]\d(\.\d+)? pts/m);
});

test('an HTML input reaches neither the network nor files outside its directory', async () => {
  hits.length = 0;
  write('secret.txt', 'outside the input directory');
  write(
    'offline/index.html',
    `<!doctype html><meta charset="utf-8"><h1 id="state">断网渲染</h1>
<img src="${origin}/pixel.png"><img src="../secret.txt"><iframe src="../secret.txt"></iframe>
<link rel="stylesheet" href="${origin}/style.css">
<script>
  fetch('${origin}/api').catch(() => {});
  new WebSocket('${origin.replace('http', 'ws')}/socket');
  fetch('data:text/plain,inline').then((r) => r.text()).then((t) => { state.textContent += ' ' + t; window.ready = true; });
</script>`,
  );
  const out = path.join(dir, 'out', 'offline.pdf');
  const result = await pageRender(['--html', 'offline', '--pdf', out, '--wait-ready', 'window.ready', '--wait-ms', '300']);
  assert.equal(result.code, 0, result.stderr);
  const { warnings } = JSON.parse(result.stdout);
  for (const address of [`${origin}/pixel.png`, `${origin}/style.css`, `${origin}/api`, `${origin.replace('http', 'ws')}/socket`]) {
    assert.ok(warnings.includes(`blocked: ${address}`), `${address} not in ${warnings}`);
  }
  assert.ok(warnings.some((warning) => warning.startsWith('blocked: file://') && warning.endsWith('/secret.txt')), warnings.join('\n'));
  assert.deepEqual(hits, []);
  const text = poppler('pdftotext', [out, '-']);
  assert.match(text, /断网渲染 inline/);
  assert.doesNotMatch(text, /outside the input directory/);
});

test('a PNG of the whole page at the asked viewport and scale', async () => {
  const out = path.join(dir, 'out', 'book.png');
  const result = await pageRender(['--html', 'book', '--png', out, '--viewport', '400x300', '--scale', '2', '--wait-ready', 'window.ready']);
  assert.equal(result.code, 0, result.stderr);
  const summary = JSON.parse(result.stdout);
  assert.equal(summary.pages, 1);
  assert.equal(summary.bytes, fs.statSync(out).size);
  const size = pngSize(fs.readFileSync(out));
  assert.equal(size.width, 800);
  assert.ok(size.height > 600, `not the whole page: ${size.height}`);
});

test('a URL input loads the allowed hosts only', async () => {
  hits.length = 0;
  const out = path.join(dir, 'out', 'online.pdf');
  const port = new URL(origin).port;
  const result = await pageRender(['--url', `${origin}/page`, '--allow-host', '127.0.0.1', '--pdf', out]);
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout).warnings, [`blocked: http://localhost:${port}/other-host.png`]);
  assert.deepEqual(hits.toSorted(), [`127.0.0.1:${port}/own.png`, `127.0.0.1:${port}/page`]);
  assert.match(poppler('pdftotext', [out, '-']), /在线页面/);

  const refused = await pageRender(['--url', `${origin}/page`, '--allow-host', 'example.com', '--pdf', out]);
  assert.equal(refused.code, 2);
  assert.match(lastLine(refused.stderr), /^page-render: --url: 127\.0\.0\.1 is not among the --allow-host hosts/);
});

test('usage errors exit with 2', async () => {
  for (const args of [
    [],
    ['--html', 'book'],
    ['--html', 'book', '--pdf', 'a.pdf', '--png', 'a.png'],
    ['--html', 'book', '--url', origin, '--pdf', 'a.pdf'],
    ['--html', 'book', '--pdf', 'a.pdf', '--viewport', '400x300'],
    ['--html', 'book', '--png', 'a.png', '--paper', 'A4'],
    ['--html', 'book', '--pdf', 'a.pdf', '--allow-host', 'example.com'],
    ['--html', 'book', '--pdf', 'a.pdf', '--paper', 'B5'],
    ['--html', 'nowhere', '--pdf', 'a.pdf'],
    ['--url', 'file:///etc/passwd', '--allow-host', 'localhost', '--pdf', 'a.pdf'],
    ['--html', 'book', '--pdf', 'a.pdf', '--timeout', 'soon'],
    ['--html', 'book', '--pdf', 'a.pdf', '--frobnicate'],
  ]) {
    const result = await pageRender(args);
    assert.equal(result.code, 2, `${args.join(' ')}: ${result.stderr}`);
    assert.match(lastLine(result.stderr), /^page-render: .*\(see page-render --help\)$/);
  }
  assert.ok(!fs.existsSync(path.join(dir, 'a.pdf')));
  const help = await pageRender(['--help']);
  assert.equal(help.code, 0);
  assert.match(help.stdout, /^page-render: /);
});

test('a page that never gets ready times out with 3', async () => {
  const out = path.join(dir, 'out', 'never.pdf');
  const started = Date.now();
  const result = await pageRender(['--html', 'book', '--pdf', out, '--wait-ready', 'window.neverSet', '--timeout', '2']);
  assert.equal(result.code, 3);
  assert.equal(lastLine(result.stderr), 'page-render: timed out after 2 seconds');
  assert.equal(result.stdout, '');
  assert.ok(Date.now() - started < 15_000);
});

test('a failing page is a rendering failure with 4', async () => {
  const out = path.join(dir, 'out', 'broken.pdf');
  const result = await pageRender(['--html', 'book', '--pdf', out, '--wait-ready', 'notDefinedAnywhere()']);
  assert.equal(result.code, 4);
  assert.match(lastLine(result.stderr), /^page-render: rendering failed: .*notDefinedAnywhere/);
  const gone = await pageRender(['--url', 'http://127.0.0.1:9/', '--allow-host', '127.0.0.1', '--png', path.join(dir, 'out', 'gone.png')]);
  assert.equal(gone.code, 4);
  assert.match(lastLine(gone.stderr), /^page-render: rendering failed: /);
  const missing = await pageRender(['--url', `${origin}/missing`, '--allow-host', '127.0.0.1', '--png', path.join(dir, 'out', 'missing.png')]);
  assert.equal(missing.code, 4);
  assert.equal(lastLine(missing.stderr), `page-render: rendering failed: ${origin}/missing answered HTTP 404`);
});

// Chromium keeps a socket under the temporary directory; a path past the limit of a socket address stops it.
test('a browser that does not start exits with 5 and Chromium\'s reason', { skip: process.platform === 'win32' }, async () => {
  const long = path.join(dir, 'a-temporary-directory-with-a-very-long-name'.repeat(3));
  fs.mkdirSync(long);
  const result = await pageRender(['--html', 'book', '--pdf', path.join(dir, 'out', 'unstarted.pdf')], { TMPDIR: long });
  assert.equal(result.code, 5);
  assert.match(lastLine(result.stderr), /^page-render: the browser did not start: Socket path too long/);
});

test('a browser that is not on the machine exits with 6 before anything is started', async () => {
  const nowhere = { PLAYWRIGHT_BROWSERS_PATH: path.join(dir, 'no-browsers') };
  const out = path.join(dir, 'out', 'absent.pdf');
  const result = await pageRender(['--html', 'book', '--pdf', out], nowhere);
  assert.equal(result.code, 6);
  assert.match(lastLine(result.stderr), /^page-render: Chrome for Testing is not downloaded on this node yet: .*buckyos ext-tools download ext-core chromium/);
  assert.equal(result.stdout, '');
  assert.ok(!fs.existsSync(out));
  assert.equal((await pageRender(['--version'], nowhere)).code, 0);
  assert.equal((await pageRender(['--html', 'book'], nowhere)).code, 2);
});

// As a node has it: the package unpacked as the tools directory, or in a staging or version directory of it and reached
// through store/<id>, and the browser in <tools>/online/chromium, which the package's own online/chromium is not. A tools
// directory may itself be in a directory called staging or versions.
test('the packaged command finds the node\'s browser from wherever the package is', async () => {
  const executable = loadPlaywright().chromium.executablePath();
  let revision = executable;
  while (!/^chromium-\d+$/.test(path.basename(revision))) {
    assert.notEqual(revision, path.dirname(revision), `${executable} is not in a Playwright browser directory`);
    revision = path.dirname(revision);
  }
  const browsers = path.dirname(revision);
  const versions = { ...readVersions(), binary: path.relative(browsers, executable) };
  const bundled = path.join(BROWSER_DIR, '..', 'playwright');
  const link = process.platform === 'win32' ? 'junction' : 'dir';
  const layouts = [
    ['tools-0'],
    ['tools-1', 'staging', 'ext-core-0123456789ab-1'],
    ['tools-2', 'versions', 'ext-core', '1.0.0-0123456789ab'],
    ['staging/tools-3'],
    ['versions/ext-core/tools-4'],
  ];
  for (const [index, [root, ...inside]] of layouts.entries()) {
    const tools = path.join(dir, root);
    const store = path.join(tools, ...inside, 'store', 'ext-core');
    fs.mkdirSync(store, { recursive: true });
    fs.cpSync(BROWSER_DIR, path.join(store, 'browser'), { recursive: true, filter: (source) => path.basename(source) !== 'test' });
    fs.writeFileSync(path.join(store, 'browser', 'versions.json'), JSON.stringify(versions));
    if (fs.existsSync(bundled)) fs.symlinkSync(bundled, path.join(store, 'playwright'), link);
    let packaged = path.join(store, 'browser', 'page-render.mjs');
    if (inside[0] === 'versions') {
      fs.mkdirSync(path.join(tools, 'store'));
      fs.symlinkSync(store, path.join(tools, 'store', 'ext-core'), link);
      packaged = path.join(tools, 'store', 'ext-core', 'browser', 'page-render.mjs');
    }
    const render = () => node(packaged, ['--html', 'book', '--pdf', path.join(dir, 'out', `layout-${index}.pdf`)], { cwd: dir, env: { PLAYWRIGHT_BROWSERS_PATH: '' } });

    fs.mkdirSync(path.join(tools, ...inside, 'online', 'chromium', path.basename(revision)), { recursive: true });
    const before = await render();
    assert.equal(before.code, 6, `${root} ${inside.join('/')}: ${before.stderr}`);

    fs.rmSync(path.join(tools, 'online'), { recursive: true, force: true });
    fs.mkdirSync(path.join(tools, 'online'));
    fs.symlinkSync(browsers, path.join(tools, 'online', 'chromium'), link);
    const after = await render();
    assert.equal(after.code, 0, `${root} ${inside.join('/')}: ${after.stderr}`);
    assert.equal(JSON.parse(after.stdout).pages, 3);
  }
});
