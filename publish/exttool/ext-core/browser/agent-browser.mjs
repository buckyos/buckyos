#!/usr/bin/env node
// The Agent's command for its persistent browser (doc/opendan/protocol/Agent Browser Protocol.md §2).
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';

import { CliError, main, number, parse, usage } from './cli.mjs';
import { KrpcClient, RpcError } from './krpc.mjs';
import { loadPlaywright, readVersions, store } from './runtime.mjs';

const EXIT_UNAVAILABLE = 3;
const EXIT_USER_IN_CONTROL = 4;
const EXIT_LEASE_LOST = 5;
const EXIT_INTERRUPTED = 130;

const ACTION_TIMEOUT_MS = 30_000;
const IMAGE_EDGE = 1280;
const HANDOVER_REASONS = ['password', 'captcha', 'other'];

const HELP = {
  '': `agent-browser: operate this Agent's persistent browser

  agent-browser status [--json]                  the browser and this session's tab group: who has control, which tabs
  agent-browser run <script.mjs> [--arg k=v]...  run a Playwright script on this session's tab
  agent-browser open <url>                       open an address in this session's tab
  agent-browser snapshot                         the current page as an accessibility tree (text)
  agent-browser screenshot <file.png>            the current viewport as a PNG file
  agent-browser tabs                             this session's tabs
  agent-browser handover --reason password|captcha|other [--note <text>]
                                                 ask the user to take over
  agent-browser done [--note <text>]             report the job finished; control returns to the user
  agent-browser stuck --note <text>              report being stuck; control returns to the user
  agent-browser env                              environment for a project with its own Playwright version
  agent-browser version                          versions of the browser and of playwright-core

run, open, snapshot, screenshot and tabs take the lease of this session's tab group, do their work and release it;
those that touch a page connect to the browser for that time only. While the user has control they fail with exit
code 4: wait until the user hands the browser back, then read the page again.

Exit codes: 0 ok, 1 failed, 2 usage, 3 browser unavailable, 4 the user has control, 5 lease lost while working.
\`agent-browser <command> --help\` describes one command.`,

  status: `agent-browser status [--json]

Whether the browser can be used, and for the tab group of this session: who has control (user or agent), the Agent's
state, a pending handover, and the tabs (* marks the group's current tab). Takes no lease and works while the user has
control.
--json prints the Status structure of the protocol instead. Exit code 3 when the browser is unavailable.`,

  run: `agent-browser run <script.mjs> [--arg k=v]...

Calls the script's default export once and prints what it returns as JSON:

  export default async ({ page, pages, context, args }) => { ...; return result; };

  page     the group's current tab (a new one when the group has none)
  pages    all tabs of the group, page among them
  context  the browser context; context.newPage() opens a tab in the group. context.pages() also has the tabs
           of other groups: leave those alone
  args     the --arg values, as strings

The lease is renewed while the script runs. If the user takes over, the script is stopped (exit code 5).
Actions time out after ${ACTION_TIMEOUT_MS / 1000} seconds unless the script sets its own timeout.
The viewport follows the user's device: do not change it.
page.screenshot() needs the tab in front of its window: call page.bringToFront() first.
Dialogs (alert, confirm, prompt): handle them with page.on('dialog', ...). A dialog the script does not handle is
dismissed (a beforeunload dialog is accepted) and reported on stderr. Dialogs in other groups' tabs are left open.`,

  open: `agent-browser open <url>

Navigates the group's current tab to the address (a new tab when the group has none) and prints the resulting
url and title. An address without a scheme gets https://, or http:// for localhost and IP addresses.
A page still loading after ${ACTION_TIMEOUT_MS / 1000} seconds is reported on stderr and is not an error.`,

  snapshot: `agent-browser snapshot

Prints the url and title of the group's current tab, an empty line, then the page's accessibility tree
(Playwright's ARIA snapshot of <body>).`,

  screenshot: `agent-browser screenshot <file.png>

Saves the current viewport of the group's current tab as a PNG file and prints its path.`,

  tabs: `agent-browser tabs

Lists the tabs of this session's group: number, url, title; * marks the current tab.`,

  handover: `agent-browser handover --reason password|captcha|other [--note <text>]

Asks the user to take over the browser. From then on the user has control: do not use the browser until they
hand it back.`,

  done: `agent-browser done [--note <text>]

Reports the job in the browser as finished; control returns to the user.`,

  stuck: `agent-browser stuck --note <text>

Reports that the job cannot go on, with what is in the way; control returns to the user.`,

  env: `agent-browser env

Prints, as \`export K=V\` lines, the environment a project needs when it uses a Playwright version other than the
system's: where to download its own browser, and on Linux where the libraries and fonts are.
Use it for that project's commands only, for example: ( eval "$(agent-browser env)"; npx playwright test )`,

  version: `agent-browser version

Prints the versions of the bundled browser and of playwright-core.`,

  'reflow-run': `agent-browser reflow-run --cdp <cdp_url> --url <url> --package <dir> --out <file> [--click <selector>] [--timeout <seconds>]

Internal: run by the Agent Service to refresh a reflow result. Opens the address in a new tab, runs
<dir>/refresh.mjs, writes what it returns to <file> as JSON and closes the tab. --timeout (default 60) bounds the
whole run; an action of the script waits a third of it, 15 seconds at most, unless the script says otherwise.`,
};

function print(text) {
  process.stdout.write(`${text}\n`);
}

function note(text) {
  process.stderr.write(`agent-browser: ${text}\n`);
}

function firstLine(text) {
  return String(text).split('\n')[0];
}

function explain(error) {
  if (!(error instanceof RpcError)) return error;
  switch (error.code) {
    case 'browser_unavailable':
      return new CliError(`the Agent browser is unavailable: ${error.message}`, EXIT_UNAVAILABLE);
    case 'user_in_control':
      return new CliError(
        'the user is operating the browser: wait until they hand it back (`agent-browser status` shows who has control)',
        EXIT_USER_IN_CONTROL,
      );
    case 'lease_lost':
      return new CliError('the lease on the browser is gone: the user took over, or it expired', EXIT_LEASE_LOST);
    default:
      return new CliError(error.message);
  }
}

function rpc() {
  const url = process.env.OPENDAN_BROWSER_RPC;
  if (!url) {
    throw new CliError(
      "the Agent browser is unavailable: OPENDAN_BROWSER_RPC is not set (only sessions on the Agent's own node reach its browser)",
      EXIT_UNAVAILABLE,
    );
  }
  const client = new KrpcClient(url, process.env.BUCKYOS_APPCLIENT_SESSION_TOKEN);
  return (method, params) => client.call(method, params).catch((e) => Promise.reject(explain(e)));
}

function sessionId() {
  const id = process.env.OPENDAN_SESSION_ID;
  if (!id) throw new CliError('the Agent browser is unavailable: OPENDAN_SESSION_ID is not set (not inside an Agent session)', EXIT_UNAVAILABLE);
  return id;
}

async function acquire(call) {
  await call('browser.ensure');
  return call('browser.acquire', { session_id: sessionId() });
}

// acquire, work(lease, stopped), release. `stopped` rejects when the lease is lost or the process is told to end.
async function withLease(work) {
  const call = rpc();
  const lease = await acquire(call);
  let lost = false;
  let stop;
  const stopped = new Promise((_, reject) => {
    stop = reject;
  });
  stopped.catch(() => {});
  const renewing = setInterval(() => {
    call('browser.renew', { lease_id: lease.lease_id }).catch((e) => {
      if (e.exitCode !== EXIT_LEASE_LOST) return;
      lost = true;
      stop(e);
    });
  }, Math.max(1000, lease.ttl_ms / 3));
  const onSignal = (signal) => stop(new CliError(`interrupted by ${signal}`, EXIT_INTERRUPTED));
  process.once('SIGINT', onSignal);
  process.once('SIGTERM', onSignal);
  try {
    return await work(lease, stopped);
  } finally {
    clearInterval(renewing);
    if (!lost) await call('browser.release', { lease_id: lease.lease_id }).catch((e) => note(`could not release the lease: ${e.message}`));
  }
}

// A tab showing a dialog nobody answered takes no new debugger session, and attaching to the browser waits on it.
async function silentTabs(cdpUrl) {
  let targets;
  try {
    targets = await (await fetch(`${cdpUrl}/json/list`)).json();
  } catch {
    return [];
  }
  const probe = (target) =>
    new Promise((resolve) => {
      const socket = new WebSocket(target.webSocketDebuggerUrl);
      const finish = (silent) => {
        clearTimeout(timer);
        socket.close();
        resolve(silent ? target.url : null);
      };
      const timer = setTimeout(() => finish(true), 2000);
      socket.onopen = () => socket.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression: '1' } }));
      socket.onmessage = () => finish(false);
      socket.onerror = () => finish(false);
    });
  return (await Promise.all(targets.filter((t) => t.type === 'page').map(probe))).filter(Boolean);
}

async function connect(cdpUrl, timeout) {
  const { chromium } = loadPlaywright();
  try {
    // noDefaults: the browser is also on the user's screen; Playwright's own emulation and download settings stay off it.
    return await chromium.connectOverCDP(cdpUrl, { noDefaults: true, isLocal: true, timeout });
  } catch (e) {
    const silent = await silentTabs(cdpUrl);
    const why = silent.length
      ? `; these tabs do not answer, probably because of a dialog (alert, confirm, prompt) nobody answered: ${silent.join(', ')}`
      : '';
    throw new CliError(`cannot attach to the browser: ${firstLine(e.message)}${why}`);
  }
}

// The connection sees every tab of the browser, the other groups' and the one the user is watching among them. This
// keeps what it does to the tabs that are ours (`tabIds`, those opened here, and their popups):
// - Playwright closes within milliseconds every dialog nobody listens for. One listener on the context keeps them all
//   open; ours are then closed here unless the script handles them, the others stay for the user.
// - A tab behind another one in its window gets no animation frames, which Playwright's actions wait for. Ours are
//   told they are in front, for as long as the connection lasts.
// - A new tab is opened in a window of its own: in an existing window it would push that window's tab into the
//   background.
async function joinTabs(browser, tabIds) {
  const context = browser.contexts()[0];
  const own = new Set();
  let settle;
  const settled = new Promise((resolve) => {
    settle = resolve;
  });
  const isOwn = async (page) => {
    if (own.has(page)) return true;
    const opener = await page.opener().catch(() => null);
    return opener ? isOwn(opener) : false;
  };
  context.on('dialog', async (dialog) => {
    const page = dialog.page();
    if (!page || page.listenerCount('dialog') > 0 || context.listenerCount('dialog') > 1) return;
    await settled;
    if (!(await isOwn(page))) return;
    note(`${dialog.type()} dialog dismissed: ${dialog.message()}`);
    await (dialog.type() === 'beforeunload' ? dialog.accept() : dialog.dismiss()).catch(() => {});
  });

  const ids = new Map();
  const targetId = (page) => {
    if (!ids.has(page)) {
      ids.set(
        page,
        (async () => {
          const session = await context.newCDPSession(page);
          try {
            return (await session.send('Target.getTargetInfo')).targetInfo.targetId;
          } finally {
            await session.detach().catch(() => {});
          }
        })(),
      );
    }
    return ids.get(page);
  };
  const adopt = async (page) => {
    if (own.has(page)) return;
    own.add(page);
    const session = await context.newCDPSession(page);
    await session.send('Emulation.setFocusEmulationEnabled', { enabled: true });
  };
  const find = async (id) => {
    for (const page of context.pages()) {
      if ((await targetId(page).catch(() => null)) === id) return page;
    }
    return undefined;
  };

  const pages = (await Promise.all(tabIds.map(find))).filter(Boolean);
  await Promise.all(pages.map(adopt));
  context.on('page', async (page) => {
    if (await isOwn(page)) await adopt(page).catch(() => {});
  });
  const control = await browser.newBrowserCDPSession();
  context.newPage = async () => {
    const created = await control.send('Target.createTarget', { url: 'about:blank', newWindow: true });
    for (const until = Date.now() + ACTION_TIMEOUT_MS; Date.now() < until; await sleep(20)) {
      const page = await find(created.targetId);
      if (page) {
        await adopt(page);
        return page;
      }
    }
    throw new Error('the new tab did not appear');
  };
  settle();
  return { context, pages, find };
}

// acquire, connect, work({ page, pages, context }), disconnect, release.
function withPage(work) {
  return withLease(async (lease, stopped) => {
    let browser;
    let ended = false;
    const running = (async () => {
      browser = await connect(lease.cdp_url);
      if (ended) return undefined;
      const { context, pages, find } = await joinTabs(browser, lease.tabs.map((tab) => tab.tab_id));
      context.setDefaultTimeout(ACTION_TIMEOUT_MS);
      const active = lease.active_tab_id && (await find(lease.active_tab_id));
      let page = pages.includes(active) ? active : pages[0];
      if (!page) {
        // The server counts a tab opened under the lease into the lease's group.
        page = await context.newPage();
        pages.push(page);
      }
      return work({ page, pages, context });
    })();
    running.catch(() => {});
    try {
      return await Promise.race([running, stopped]);
    } finally {
      ended = true;
      // Disconnecting also fails whatever the script is still waiting for.
      if (browser) await Promise.race([browser.close().catch(() => {}), sleep(5000)]);
    }
  });
}

function describeTab(tab, index) {
  const title = tab.title ? `  "${tab.title}"` : '';
  return `${tab.active ? '*' : ' '} ${index + 1} ${tab.url}${title}${tab.loading ? '  (loading)' : ''}`;
}

function describeStatus(status) {
  const lines = [];
  if (!status.enabled || !status.available) {
    lines.push(`browser: unavailable${status.unavailable ? `: ${status.unavailable}` : ''}`);
  } else {
    const version = status.version ? ` (chromium ${status.version.browser}, playwright ${status.version.playwright})` : '';
    lines.push(`browser: ${status.running ? 'running' : 'stopped, starts on demand'}${version}`);
  }
  for (const group of status.groups ?? []) {
    const facts = [`${group.controller} has control`, `agent ${group.agent_state}`];
    if (group.user_hold) facts.push('held by the user');
    if (group.lease) facts.push(`lease of session ${group.lease.session_id}`);
    lines.push(`group ${group.group_id}: ${facts.join(', ')}`);
    if (group.handover) lines.push(`  handover asked: ${group.handover.reason}${group.handover.note ? `: ${group.handover.note}` : ''}`);
    if (group.note) lines.push(`  note: ${group.note}`);
    if (group.tabs.length) lines.push(...group.tabs.map((tab, i) => `  ${describeTab(tab, i)}`));
    else lines.push('  (no tabs)');
  }
  return lines.join('\n');
}

async function status(argv) {
  const { values } = parse(argv, { json: { type: 'boolean' } });
  const session = process.env.OPENDAN_SESSION_ID;
  const result = await rpc()('browser.status', session ? { session_id: session } : {});
  print(values.json ? JSON.stringify(result, null, 2) : describeStatus(result));
  return result.enabled && result.available ? 0 : EXIT_UNAVAILABLE;
}

async function run(argv) {
  const { values, positionals } = parse(argv, { arg: { type: 'string', multiple: true } });
  if (positionals.length !== 1) throw usage('run takes one script file');
  const file = path.resolve(positionals[0]);
  if (!fs.existsSync(file)) throw usage(`no such script: ${file}`);
  const args = {};
  for (const pair of values.arg ?? []) {
    const eq = pair.indexOf('=');
    if (eq < 1) throw usage(`--arg takes k=v, got: ${pair}`);
    args[pair.slice(0, eq)] = pair.slice(eq + 1);
  }
  const script = (await import(pathToFileURL(file).href)).default;
  if (typeof script !== 'function') throw new CliError(`${file}: the default export is not a function`);
  const result = await withPage(async (browser) => {
    try {
      return await script({ ...browser, args });
    } catch (e) {
      throw new CliError(`the script failed: ${e?.stack ?? e}`);
    }
  });
  if (result === undefined) return;
  let text;
  try {
    text = JSON.stringify(result, null, 2);
  } catch (e) {
    throw new CliError(`the script's result is not JSON: ${e.message}`);
  }
  if (text !== undefined) print(text);
}

async function open(argv) {
  const { positionals } = parse(argv, {});
  if (positionals.length !== 1) throw usage('open takes one address');
  let url = positionals[0];
  if (!/^(about:|data:|file:|[a-z][a-z0-9+.-]*:\/\/)/i.test(url)) {
    url = `${/^(localhost|\d+\.\d+\.\d+\.\d+|\[)/i.test(url) ? 'http' : 'https'}://${url}`;
  }
  await withPage(async ({ page }) => {
    try {
      await page.goto(url);
    } catch (e) {
      if (e.name !== 'TimeoutError') throw new CliError(`cannot open ${url}: ${firstLine(e.message)}`);
      note(`${url} is still loading after ${ACTION_TIMEOUT_MS / 1000} seconds`);
    }
    print(`url: ${page.url()}\ntitle: ${await page.title()}`);
  });
}

async function snapshot(argv) {
  const { positionals } = parse(argv, {});
  if (positionals.length) throw usage('snapshot takes no arguments');
  await withPage(async ({ page }) => {
    const tree = await page.locator('body').ariaSnapshot();
    print(`url: ${page.url()}\ntitle: ${await page.title()}\n\n${tree}`);
  });
}

async function screenshot(argv) {
  const { positionals } = parse(argv, {});
  if (positionals.length !== 1 || !positionals[0].toLowerCase().endsWith('.png')) throw usage('screenshot takes one .png file');
  const file = path.resolve(positionals[0]);
  await withPage(async ({ page }) => {
    // Only the tab in front of its window is painted.
    await page.bringToFront();
    await page.screenshot({ path: file, type: 'png' });
    print(`saved: ${file}`);
  });
}

async function tabs(argv) {
  const { positionals } = parse(argv, {});
  if (positionals.length) throw usage('tabs takes no arguments');
  await withLease(async (lease) => {
    print(`group ${lease.group_id}`);
    print(lease.tabs.length ? lease.tabs.map(describeTab).join('\n') : '(no tabs)');
  });
}

async function handover(argv) {
  const { values } = parse(argv, { reason: { type: 'string' }, note: { type: 'string' } });
  if (!HANDOVER_REASONS.includes(values.reason)) throw usage(`--reason must be one of: ${HANDOVER_REASONS.join(', ')}`);
  const result = await rpc()('browser.request_handover', { session_id: sessionId(), reason: values.reason, note: values.note });
  print(`handover asked (group ${result.group_id}): the user has control now; do not use the browser until they hand it back`);
}

// A lease lives only as long as one command, so reporting takes one just to release it with the state.
async function report(state, argv) {
  const { values } = parse(argv, { note: { type: 'string' } });
  if (state === 'stuck' && !values.note) throw usage('stuck needs --note: say what is in the way');
  const call = rpc();
  let lease;
  try {
    lease = await acquire(call);
  } catch (e) {
    if (e.exitCode !== EXIT_USER_IN_CONTROL) throw e;
    throw new CliError(
      `"${state}" was not reported: the user is operating the browser, so the Agent has no control to give back; tell them in the conversation`,
      EXIT_USER_IN_CONTROL,
    );
  }
  await call('browser.release', { lease_id: lease.lease_id, state, note: values.note });
  print(`reported ${state} (group ${lease.group_id})`);
}

function shellQuote(text) {
  return `'${text.replaceAll("'", "'\\''")}'`;
}

function env(argv) {
  parse(argv, {});
  // The system's browser directory is read-only and holds one version: another version downloads into the project.
  const lines = ['export PLAYWRIGHT_BROWSERS_PATH=0'];
  if (process.platform === 'linux') {
    const lib = path.join(store, 'conda', 'lib');
    if (!fs.existsSync(lib)) throw new CliError(`the bundled libraries are not at ${lib}`);
    lines.push(
      'export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1',
      `export LD_LIBRARY_PATH=${shellQuote([lib, process.env.LD_LIBRARY_PATH].filter(Boolean).join(':'))}`,
      `export FONTCONFIG_FILE=${shellQuote(path.join(store, 'conda', 'etc', 'fonts', 'fonts.conf'))}`,
      `export FONTCONFIG_PATH=${shellQuote(path.join(store, 'conda', 'etc', 'fonts'))}`,
    );
  } else {
    lines.push(`# ${process.platform}: a downloaded browser needs no library or font settings`);
  }
  print(lines.join('\n'));
}

function version(argv) {
  parse(argv, {});
  const versions = readVersions();
  print(`chromium ${versions.chromium} (revision ${versions.revision})\nplaywright-core ${versions.playwright}`);
}

// Fetched with the context's cookies, scaled down in the page and re-encoded, so that a result stays small.
async function saveImage(page, source) {
  const url = new URL(source, page.url()).href;
  if (url.startsWith('data:')) return url;
  const response = await page.context().request.get(url, { headers: { referer: page.url() } });
  if (!response.ok()) throw new Error(`save.image: ${url}: HTTP ${response.status()}`);
  const type = (response.headers()['content-type'] ?? '').split(';')[0].trim();
  const base64 = (await response.body()).toString('base64');
  if (type === 'image/svg+xml') return `data:${type};base64,${base64}`;
  return page.evaluate(
    async ({ base64, type, edge }) => {
      const bitmap = await createImageBitmap(new Blob([Uint8Array.fromBase64(base64)], { type }));
      const scale = Math.min(1, edge / Math.max(bitmap.width, bitmap.height));
      const canvas = new OffscreenCanvas(Math.max(1, Math.round(bitmap.width * scale)), Math.max(1, Math.round(bitmap.height * scale)));
      const painter = canvas.getContext('2d');
      painter.fillStyle = '#fff';
      painter.fillRect(0, 0, canvas.width, canvas.height);
      painter.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      const jpeg = await canvas.convertToBlob({ type: 'image/jpeg', quality: 0.8 });
      return `data:image/jpeg;base64,${new Uint8Array(await jpeg.arrayBuffer()).toBase64()}`;
    },
    { base64, type, edge: IMAGE_EDGE },
  );
}

async function reflowRun(argv) {
  const { values } = parse(argv, {
    cdp: { type: 'string' },
    url: { type: 'string' },
    package: { type: 'string' },
    out: { type: 'string' },
    click: { type: 'string' },
    timeout: { type: 'string', default: '60' },
  });
  for (const name of ['cdp', 'url', 'package', 'out']) {
    if (!values[name]) throw usage(`reflow-run needs --${name}`);
  }
  const timeout = number(values.timeout, '--timeout', { min: 1 }) * 1000;
  const script = path.join(path.resolve(values.package), 'refresh.mjs');
  if (!fs.existsSync(script)) throw new CliError(`no refresh script: ${script}`);
  const refresh = (await import(pathToFileURL(script).href)).default;
  if (typeof refresh !== 'function') throw new CliError(`${script}: the default export is not a function`);

  const browser = await connect(values.cdp, timeout);
  let page;
  try {
    const running = (async () => {
      const { context } = await joinTabs(browser, []);
      page = await context.newPage();
      // An action that finds nothing fails well before the run does, so that the reason names what was missing.
      page.setDefaultTimeout(Math.min(15_000, Math.max(1000, Math.floor(timeout / 3))));
      page.setDefaultNavigationTimeout(timeout);
      await page.goto(values.url);
      if (values.click) {
        await page.locator(values.click).first().click();
        // The click may navigate or fetch: give that a moment to start, then wait for it to finish.
        await sleep(500);
        await page.waitForLoadState('load');
        await page.waitForLoadState('networkidle', { timeout: 10_000 }).catch(() => {});
      }
      const data = await refresh({ page, url: values.url, save: { image: (source) => saveImage(page, source) } });
      const text = JSON.stringify(data);
      if (text === undefined) throw new Error('refresh.mjs returned nothing');
      fs.writeFileSync(values.out, text);
    })();
    running.catch(() => {});
    const expired = sleep(timeout).then(() => {
      throw new Error(`timed out after ${timeout / 1000} seconds`);
    });
    expired.catch(() => {});
    await Promise.race([running, expired]);
  } catch (e) {
    process.stderr.write(`${e?.stack ?? e}\n`);
    throw new CliError(`reflow failed: ${firstLine(e?.message ?? e)}`);
  } finally {
    const closing = (async () => {
      await page?.close().catch(() => {});
      await browser.close().catch(() => {});
    })();
    await Promise.race([closing, sleep(5000)]);
  }
}

const COMMANDS = {
  status,
  run,
  open,
  snapshot,
  screenshot,
  tabs,
  handover,
  done: (argv) => report('done', argv),
  stuck: (argv) => report('stuck', argv),
  env,
  version,
  'reflow-run': reflowRun,
};

main('agent-browser', async () => {
  const [command, ...argv] = process.argv.slice(2);
  if (command === undefined || command === '--help' || command === '-h' || command === 'help') {
    print(HELP['']);
    return command === undefined ? 2 : 0;
  }
  if (!Object.hasOwn(COMMANDS, command)) throw usage(`unknown command: ${command}`);
  if (argv.includes('--help') || argv.includes('-h')) {
    print(HELP[command]);
    return 0;
  }
  return COMMANDS[command](argv);
});
