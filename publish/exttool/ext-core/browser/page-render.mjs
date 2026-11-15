#!/usr/bin/env node
// Renders one page to a PDF or PNG file with a throwaway headless Chromium (system rendering).
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { CliError, main, number, parse, usage } from './cli.mjs';
import { loadPlaywright, readVersions } from './runtime.mjs';

// Part of the fingerprint Workflow caches results under: raise it when the output of the same input changes.
const VERSION = '1.0.0';

const EXIT_TIMEOUT = 3;
const EXIT_RENDER = 4;
const EXIT_BROWSER = 5;

const PAPERS = ['A3', 'A4', 'A5', 'Letter'];
const MAX_WARNINGS = 50;

const HELP = `page-render: render one page to a PDF or PNG file with a throwaway headless browser

  page-render --html <file|dir> (--pdf <out> | --png <out>) [options]
  page-render --url <url> --allow-host <host>... (--pdf <out> | --png <out>) [options]

Input
  --html <file|dir>     a local HTML file, or a directory with index.html. Nothing is fetched from the network:
                        only files inside that directory (and data:, blob:) load
  --url <url>           an http(s) address. Only the hosts given with --allow-host load; the address's own host
                        has to be among them
  --allow-host <host>   repeatable

PDF (--pdf <out>)
  --paper <A3|A4|A5|Letter|WxHmm>   default A4; a custom size as width x height in millimetres, e.g. 100x150mm
  --landscape
  --margin <mm>         all four sides, default 0. Header and footer are drawn inside the margin
  --header-html <file>  Chromium's print template: elements with class pageNumber, totalPages, date, title, url
  --footer-html <file>  are filled in; give the text a font-size
  --background          print background colours and images

PNG (--png <out>): the whole page
  --viewport <WxH>      default 1280x800
  --scale <n>           device pixel ratio, default 1

Readiness
  --wait-ready <js>     wait until this expression is truthy in the page
  --wait-ms <ms>        then wait this long
  --timeout <seconds>   for the whole run, default 60

  --version             prints: page-render <version> chromium <version> playwright-core <version>

On success prints one line of JSON: {"pages": n, "bytes": n, "sandbox": bool, "warnings": [text...]}.
pages is 1 for a PNG. Every request that was blocked is listed in warnings.
Exit codes: 0 ok, 2 usage, 3 timed out, 4 rendering failed, 5 the browser did not start.
The last line on stderr is the reason.`;

function firstLine(text) {
  return String(text).split('\n')[0];
}

function readOptions(argv) {
  const { values, positionals } = parse(argv, {
    html: { type: 'string' },
    url: { type: 'string' },
    'allow-host': { type: 'string', multiple: true },
    pdf: { type: 'string' },
    png: { type: 'string' },
    paper: { type: 'string' },
    landscape: { type: 'boolean' },
    margin: { type: 'string' },
    'header-html': { type: 'string' },
    'footer-html': { type: 'string' },
    background: { type: 'boolean' },
    viewport: { type: 'string' },
    scale: { type: 'string' },
    'wait-ready': { type: 'string' },
    'wait-ms': { type: 'string' },
    timeout: { type: 'string' },
    version: { type: 'boolean' },
  });
  if (values.help || values.version) return values;
  if (positionals.length) throw usage(`unexpected argument: ${positionals[0]}`);
  if (Boolean(values.html) === Boolean(values.url)) throw usage('give one of --html and --url');
  if (Boolean(values.pdf) === Boolean(values.png)) throw usage('give one of --pdf and --png');
  const only = (kind, names) => {
    for (const name of names) {
      if (values[name] !== undefined) throw usage(`--${name} is for --${kind} only`);
    }
  };
  if (values.html) only('url', ['allow-host']);
  if (values.png) only('pdf', ['paper', 'landscape', 'margin', 'header-html', 'footer-html', 'background']);
  if (values.pdf) only('png', ['viewport', 'scale']);
  return values;
}

function readInput(values) {
  if (values.url) {
    let url;
    try {
      url = new URL(values.url);
    } catch {
      throw usage(`--url: not an address: ${values.url}`);
    }
    if (url.protocol !== 'http:' && url.protocol !== 'https:') throw usage('--url takes an http(s) address');
    const hosts = new Set((values['allow-host'] ?? []).map((host) => host.toLowerCase()));
    if (!hosts.has(url.hostname)) throw usage(`--url: ${url.hostname} is not among the --allow-host hosts`);
    return { url: url.href, hosts };
  }
  let stat;
  try {
    stat = fs.statSync(values.html);
  } catch {
    throw usage(`--html: no such file or directory: ${values.html}`);
  }
  const root = fs.realpathSync(stat.isDirectory() ? values.html : path.dirname(values.html));
  const file = path.join(root, stat.isDirectory() ? 'index.html' : path.basename(values.html));
  if (!fs.existsSync(file)) throw usage(`--html: no index.html in ${values.html}`);
  return { url: pathToFileURL(file).href, root };
}

function readPaper(text = 'A4') {
  const named = PAPERS.find((name) => name.toLowerCase() === text.toLowerCase());
  if (named) return { format: named };
  const size = /^(\d+(?:\.\d+)?)x(\d+(?:\.\d+)?)(?:mm)?$/i.exec(text);
  if (!size) throw usage(`--paper: one of ${PAPERS.join(', ')} or <width>x<height>mm, got: ${text}`);
  return { width: `${size[1]}mm`, height: `${size[2]}mm` };
}

function readViewport(text = '1280x800') {
  const size = /^(\d+)x(\d+)$/i.exec(text);
  if (!size || size[1] === '0' || size[2] === '0') throw usage(`--viewport: <width>x<height> in pixels, got: ${text}`);
  return { width: Number(size[1]), height: Number(size[2]) };
}

function readTemplate(file, name) {
  if (file === undefined) return undefined;
  try {
    return fs.readFileSync(file, 'utf8');
  } catch (e) {
    throw usage(`--${name}: cannot read ${file}: ${e.message}`);
  }
}

// Whether the page may load `address`. data: and blob: never leave the browser and are not asked about.
function permits(input, address) {
  let url;
  try {
    url = new URL(address);
  } catch {
    return false;
  }
  if (input.hosts) return (url.protocol === 'http:' || url.protocol === 'https:') && input.hosts.has(url.hostname);
  if (url.protocol !== 'file:') return false;
  let file = fileURLToPath(url);
  try {
    file = fs.realpathSync(file);
  } catch {}
  return file.startsWith(input.root + path.sep);
}

// With its sandbox where the host allows one: not as root, and not where user namespaces are restricted.
async function launch(chromium, args) {
  try {
    return { browser: await chromium.launch({ channel: 'chromium', chromiumSandbox: true, args }), sandbox: true };
  } catch (e) {
    if (!/no usable sandbox|no-sandbox is not supported/i.test(e.message)) throw e;
    process.stderr.write('page-render: this host gives the browser no sandbox, rendering without it\n');
    return { browser: await chromium.launch({ channel: 'chromium', chromiumSandbox: false, args }), sandbox: false };
  }
}

// Chromium's own word on why it gave up, when it left one in the launch log.
function launchFailure(message) {
  const fatal = /:FATAL:[^\]]*\]\s*([^\n\x1b]+)/.exec(message);
  return fatal ? fatal[1].trim() : firstLine(message);
}

function countPages(file) {
  return (fs.readFileSync(file, 'latin1').match(/\/Type\s*\/Page(?![A-Za-z])/g) ?? []).length;
}

async function render(values) {
  const input = readInput(values);
  const pdf = values.pdf && {
    ...readPaper(values.paper),
    landscape: Boolean(values.landscape),
    printBackground: Boolean(values.background),
    margin: Object.fromEntries(['top', 'right', 'bottom', 'left'].map((side) => [side, `${number(values.margin ?? '0', '--margin')}mm`])),
    header: readTemplate(values['header-html'], 'header-html'),
    footer: readTemplate(values['footer-html'], 'footer-html'),
  };
  const png = values.png && {
    viewport: readViewport(values.viewport),
    scale: number(values.scale ?? '1', '--scale', { min: 0.1 }),
  };
  const waitMs = number(values['wait-ms'] ?? '0', '--wait-ms');
  const timeout = number(values.timeout ?? '60', '--timeout', { min: 1 }) * 1000;
  const out = path.resolve(values.pdf ?? values.png);

  const { chromium } = loadPlaywright();
  const blocked = new Set();
  // Names outside the allowed hosts do not even resolve, for whatever the request interception does not see.
  const resolver = ['MAP * ~NOTFOUND', ...[...(input.hosts ?? [])].map((host) => `EXCLUDE ${host}`)].join(', ');
  let browser;
  const work = (async () => {
    let sandbox;
    try {
      ({ browser, sandbox } = await launch(chromium, [`--host-resolver-rules=${resolver}`]));
    } catch (e) {
      process.stderr.write(`${e.message}\n`);
      throw new CliError(`the browser did not start: ${launchFailure(e.message)}`, EXIT_BROWSER);
    }
    try {
      const context = await browser.newContext({
        serviceWorkers: 'block',
        acceptDownloads: false,
        ...(png ? { viewport: png.viewport, deviceScaleFactor: png.scale } : {}),
      });
      context.setDefaultTimeout(0);
      await context.route('**/*', (route) => {
        const address = route.request().url();
        if (permits(input, address)) return route.continue();
        blocked.add(address);
        return route.abort('blockedbyclient');
      });
      await context.routeWebSocket(/.*/, (socket) => {
        if (permits(input, socket.url().replace(/^ws/, 'http'))) return socket.connectToServer();
        blocked.add(socket.url());
        return socket.close();
      });
      const page = await context.newPage();
      const response = await page.goto(input.url);
      if (input.hosts && response && !response.ok()) throw new Error(`${input.url} answered HTTP ${response.status()}`);
      if (values['wait-ready']) await page.waitForFunction(values['wait-ready']);
      await page.evaluate(() => document.fonts.ready.then(() => undefined));
      if (waitMs) await sleep(waitMs);
      if (pdf) {
        await page.pdf({
          path: out,
          ...(pdf.format ? { format: pdf.format } : { width: pdf.width, height: pdf.height }),
          landscape: pdf.landscape,
          printBackground: pdf.printBackground,
          margin: pdf.margin,
          displayHeaderFooter: pdf.header !== undefined || pdf.footer !== undefined,
          headerTemplate: pdf.header ?? '<span></span>',
          footerTemplate: pdf.footer ?? '<span></span>',
        });
      } else {
        await page.screenshot({ path: out, type: 'png', fullPage: true });
      }
    } catch (e) {
      process.stderr.write(`${e.message}\n`);
      throw new CliError(`rendering failed: ${firstLine(e.message)}`, EXIT_RENDER);
    }
    return { pages: pdf ? countPages(out) : 1, bytes: fs.statSync(out).size, sandbox };
  })();
  work.catch(() => {});
  const expired = sleep(timeout).then(() => {
    throw new CliError(`timed out after ${timeout / 1000} seconds`, EXIT_TIMEOUT);
  });
  expired.catch(() => {});
  try {
    const result = await Promise.race([work, expired]);
    const warnings = [...blocked].slice(0, MAX_WARNINGS).map((address) => `blocked: ${address}`);
    if (blocked.size > MAX_WARNINGS) warnings.push(`blocked: ${blocked.size - MAX_WARNINGS} more requests`);
    process.stdout.write(`${JSON.stringify({ ...result, warnings })}\n`);
  } finally {
    if (browser) await Promise.race([browser.close().catch(() => {}), sleep(5000)]);
  }
}

main('page-render', async () => {
  const values = readOptions(process.argv.slice(2));
  if (values.help) {
    process.stdout.write(`${HELP}\n`);
  } else if (values.version) {
    const versions = readVersions();
    process.stdout.write(`page-render ${VERSION} chromium ${versions.chromium} playwright-core ${versions.playwright}\n`);
  } else {
    await render(values);
  }
});
