// agent-browser against a fake Agent Service and a real Chromium.
//   node --test "publish/exttool/ext-core/browser/test/*.test.mjs"
// playwright-core has to be resolvable (NODE_PATH in development) and its Chromium installed (PLAYWRIGHT_BROWSERS_PATH),
// or OPENDAN_CHROMIUM names the browser.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { after, before, beforeEach, test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';

import { AGENT_BROWSER, BROWSER_DIR, Cdp, jpegSize, node, pngSize, startAgentService, startChromium, startSite, tempDir } from './helpers.mjs';

const TOKEN = 'session-token-for-tests';
let chromium;
let service;
let site;
let dir;
let tab;

function agentBrowser(args, env = {}) {
  return node(AGENT_BROWSER, args, {
    env: { OPENDAN_BROWSER_RPC: service.url, BUCKYOS_APPCLIENT_SESSION_TOKEN: TOKEN, OPENDAN_SESSION_ID: 'session-1', ...env },
    cwd: dir,
  });
}

function script(name, source) {
  const file = path.join(dir, name);
  fs.writeFileSync(file, source);
  return file;
}

function called(method) {
  return service.calls.filter((call) => call.method === method);
}

// What the tab itself says, asked over a connection of our own.
async function visibility(target) {
  const cdp = await Cdp.open(target.webSocketDebuggerUrl);
  const { result } = await cdp.send('Runtime.evaluate', { expression: 'document.visibilityState' });
  cdp.close();
  return result.value;
}

before(async () => {
  dir = tempDir('agent-browser-test');
  chromium = await startChromium();
  service = await startAgentService(chromium);
  site = await startSite();
});

after(async () => {
  await site?.stop();
  await service?.stop();
  await chromium?.stop();
  fs.rmSync(dir, { recursive: true, force: true });
});

// Every test starts with one tab in the default group (session-1) and nothing else open.
beforeEach(async () => {
  service.reset();
  site.clear();
  const keep = await chromium.newTab(`${site.url}/blank`);
  for (const target of await chromium.targets()) {
    if (target.id !== keep.id) await chromium.closeTab(target.id);
  }
  tab = keep;
  service.group('default').tabs = [tab.id];
});

test('without OPENDAN_BROWSER_RPC the command says so and fails', async () => {
  const result = await agentBrowser(['status'], { OPENDAN_BROWSER_RPC: '' });
  assert.equal(result.code, 3);
  assert.match(result.stderr, /OPENDAN_BROWSER_RPC is not set/);
});

// 10080 is on fetch's list of ports it refuses, and it is a port an Agent Service is given (the Zone's sixth App).
test('an Agent Service on a port fetch refuses is reached', async (t) => {
  const answers = http.createServer((request, response) => {
    request.resume();
    request.on('end', () => response.end(JSON.stringify({ error: 'Failed due to reason: {"kind":"other","code":"browser_unavailable","message":"switched off"}', sys: [1] })));
  });
  const listening = await new Promise((resolve) => {
    answers.once('error', () => resolve(false));
    answers.listen(10080, '127.0.0.1', () => resolve(true));
  });
  if (!listening) return t.skip('port 10080 is taken on this machine');
  try {
    const result = await agentBrowser(['status'], { OPENDAN_BROWSER_RPC: 'http://127.0.0.1:10080/kapi/agent' });
    assert.equal(result.code, 3, result.stderr);
    assert.match(result.stderr, /switched off/);
  } finally {
    await new Promise((resolve) => answers.close(resolve));
  }
});

test('browser_unavailable is reported and fails', async () => {
  service.unavailable = 'chromium was not found on this node';
  const opened = await agentBrowser(['open', `${site.url}/form`]);
  assert.equal(opened.code, 3);
  assert.match(opened.stderr, /the Agent browser is unavailable: chromium was not found on this node/);
  const status = await agentBrowser(['status']);
  assert.equal(status.code, 3);
  assert.match(status.stdout, /browser: unavailable: chromium was not found/);
});

test('status shows who has control and the tabs', async () => {
  const text = await agentBrowser(['status']);
  assert.equal(text.code, 0, text.stderr);
  assert.match(text.stdout, /^browser: running \(chromium 159\.0\.8154\.6, playwright 1\.65\.1\)$/m);
  assert.match(text.stdout, /^group default: user has control, agent idle$/m);
  assert.ok(text.stdout.includes(`  * 1 ${site.url}/blank  "blank"`), text.stdout);
  const json = await agentBrowser(['status', '--json']);
  assert.equal(json.code, 0, json.stderr);
  assert.equal(JSON.parse(json.stdout).groups[0].tabs[0].tab_id, tab.id);
  assert.deepEqual(service.calls.map((call) => call.method), ['browser.status', 'browser.status']);
  assert.equal(service.calls[0].token, TOKEN);
  assert.deepEqual(service.calls[0].params, { session_id: 'session-1' });
});

test('open navigates the group tab, snapshot reads it, the lease is released each time', async () => {
  // Without a scheme: an IP address gets http://.
  const opened = await agentBrowser(['open', `${site.url.replace('http://', '')}/form`]);
  assert.equal(opened.code, 0, opened.stderr);
  assert.equal(opened.stdout, `url: ${site.url}/form\ntitle: 登记表\n`);
  assert.deepEqual(called('browser.acquire')[0].params, { session_id: 'session-1' });
  assert.equal(called('browser.release').length, 1);
  assert.equal(service.group('default').lease, null);
  assert.equal((await chromium.targets()).find((target) => target.id === tab.id).url, `${site.url}/form`);

  const snapshot = await agentBrowser(['snapshot']);
  assert.equal(snapshot.code, 0, snapshot.stderr);
  assert.ok(snapshot.stdout.startsWith(`url: ${site.url}/form\ntitle: 登记表\n\n`), snapshot.stdout);
  assert.match(snapshot.stdout, /- heading "登记表" \[level=1\]/);
  assert.match(snapshot.stdout, /- textbox "姓名"/);
  assert.match(snapshot.stdout, /- button "提交"/);
  assert.equal(called('browser.release').length, 2);
});

test('run fills a form in a background tab and prints what the script returns', async () => {
  await agentBrowser(['open', `${site.url}/form`]);
  const file = script(
    'fill.mjs',
    `export default async ({ page, pages, context, args }) => {
  await page.getByLabel('姓名').fill(args.who);
  await page.getByLabel('城市').fill(args.city);
  await page.getByRole('button', { name: '提交' }).click();
  return { result: await page.locator('#result').textContent(), tabs: pages.length, all: context.pages().length };
};
`,
  );
  // A tab opened later in the same window, as the user's would be: ours is now behind it.
  const foreign = await chromium.newTab(`${site.url}/blank`);
  service.group('other').tabs = [foreign.id];
  assert.equal(await visibility(tab), 'hidden');
  const result = await agentBrowser(['run', file, '--arg', 'who=张三', '--arg', 'city=上海=浦东']);
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { result: '已提交：张三，上海=浦东', tabs: 1, all: 2 });
  assert.equal(service.group('default').lease, null);
  assert.equal(await visibility(foreign), 'visible');
  assert.equal(await visibility(tab), 'hidden');
});

test('while the user has control, commands fail and say to wait', async () => {
  service.group('default').user_hold = true;
  const opened = await agentBrowser(['open', `${site.url}/form`]);
  assert.equal(opened.code, 4);
  assert.match(opened.stderr, /the user is operating the browser: wait until they hand it back/);
  assert.equal((await chromium.targets()).find((target) => target.id === tab.id).url, `${site.url}/blank`);
  const done = await agentBrowser(['done', '--note', '办完了']);
  assert.equal(done.code, 4);
  assert.match(done.stderr, /"done" was not reported: the user is operating the browser/);
  assert.equal(called('browser.release').length, 0);
});

test('a script that throws still disconnects and releases the lease', async () => {
  const file = script('throws.mjs', "export default async ({ page }) => { await page.title(); throw new Error('表单没找到'); };\n");
  const result = await agentBrowser(['run', file]);
  assert.equal(result.code, 1);
  assert.match(result.stderr, /the script failed: Error: 表单没找到/);
  assert.equal(result.stdout, '');
  assert.equal(called('browser.release').length, 1);
  assert.equal(service.group('default').lease, null);
});

test("another group's dialog stays open; the group's own unhandled dialog is dismissed", async () => {
  const foreign = await chromium.newTab(`${site.url}/blank`);
  service.group('other').tabs = [foreign.id];
  const watcher = await Cdp.open(foreign.webSocketDebuggerUrl);
  await watcher.send('Page.enable');
  const file = script(
    'wait.mjs',
    `export default async ({ page, args }) => {
  await fetch(args.site + '/signal/connected');
  await fetch(args.site + '/wait/proceed');
  await page.evaluate(() => alert('own alert'));
  return 'finished';
};
`,
  );
  const running = agentBrowser(['run', file, '--arg', `site=${site.url}`]);
  await site.received('connected');
  watcher.send('Runtime.evaluate', { expression: "setTimeout(() => alert('for the user'), 0)" }).catch(() => {});
  await watcher.waitFor('Page.javascriptDialogOpening');
  await sleep(500);
  assert.ok(!watcher.events.includes('Page.javascriptDialogClosed'), 'closed while the Agent was connected');
  site.send('proceed');
  const result = await running;
  assert.equal(result.code, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout), 'finished');
  assert.match(result.stderr, /agent-browser: alert dialog dismissed: own alert/);
  await sleep(300);
  assert.ok(!watcher.events.includes('Page.javascriptDialogClosed'), 'closed when the Agent disconnected');
  await watcher.send('Page.handleJavaScriptDialog', { accept: true });
  watcher.close();
});

test('a script handles its own dialog', async () => {
  const file = script(
    'confirm.mjs',
    `export default async ({ page }) => {
  page.on('dialog', (dialog) => dialog.accept());
  return page.evaluate(() => confirm('继续吗'));
};
`,
  );
  const result = await agentBrowser(['run', file]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout), true);
  assert.doesNotMatch(result.stderr, /dismissed/);
});

test('the lease is renewed while a script runs, and losing it stops the script', async () => {
  service.ttlMs = 3000;
  const file = script(
    'long.mjs',
    `export default async ({ page, args }) => {
  await fetch(args.site + '/signal/connected');
  await page.waitForTimeout(60_000);
};
`,
  );
  const started = Date.now();
  const running = agentBrowser(['run', file, '--arg', `site=${site.url}`]);
  await site.received('connected');
  while (!called('browser.renew').length) await sleep(50);
  service.revoke('default');
  const result = await running;
  assert.equal(result.code, 5);
  assert.match(result.stderr, /the lease on the browser is gone/);
  assert.ok(Date.now() - started < 20_000);
  assert.equal(called('browser.release').length, 0);
});

test('a group without tabs gets one, and tabs lists it', async () => {
  service.sessions.set('session-2', 'trip');
  const opened = await agentBrowser(['open', `${site.url}/form`], { OPENDAN_SESSION_ID: 'session-2' });
  assert.equal(opened.code, 0, opened.stderr);
  assert.equal(service.group('trip').tabs.length, 1);
  assert.equal((await chromium.targets()).find((target) => target.id === tab.id).url, `${site.url}/blank`);
  assert.equal(await visibility(tab), 'visible', 'the new tab pushed the other group\'s tab into the background');
  const tabs = await agentBrowser(['tabs'], { OPENDAN_SESSION_ID: 'session-2' });
  assert.equal(tabs.code, 0, tabs.stderr);
  assert.equal(tabs.stdout, `group trip\n* 1 ${site.url}/form  "登记表"\n`);
});

test('screenshot writes a PNG of the viewport, also of a tab that was in the background', async () => {
  await agentBrowser(['open', `${site.url}/form`]);
  await chromium.newTab(`${site.url}/blank`);
  assert.equal(await visibility(tab), 'hidden');
  const result = await agentBrowser(['screenshot', 'shot.png']);
  assert.equal(result.code, 0, result.stderr);
  const file = path.join(dir, 'shot.png');
  assert.equal(result.stdout, `saved: ${file}\n`);
  const size = pngSize(fs.readFileSync(file));
  assert.ok(size.width > 0 && size.height > 0);
  assert.equal((await agentBrowser(['screenshot', 'shot.jpg'])).code, 2);
});

test('a screenshot is in CSS pixels while the screen paints the tab at twice its pixels', async () => {
  await agentBrowser(['open', `${site.url}/form`]);
  // What the screen does for a viewer on a sharp screen: the tab's window goes to the second screen, of pixel ratio 2.
  const { webSocketDebuggerUrl } = await (await fetch(`${chromium.cdpUrl}/json/version`)).json();
  const browser = await Cdp.open(webSocketDebuggerUrl);
  const { windowId } = await browser.send('Browser.getWindowForTarget', { targetId: tab.id });
  await browser.send('Browser.setWindowBounds', { windowId, bounds: { left: 1600, top: 0 } });
  const page = await Cdp.open(tab.webSocketDebuggerUrl);
  const { result: ratio } = await page.send('Runtime.evaluate', { expression: 'devicePixelRatio' });
  const { result: viewport } = await page.send('Runtime.evaluate', { expression: '[innerWidth, innerHeight]', returnByValue: true });
  page.close();
  try {
    assert.equal(ratio.value, 2);
    const result = await agentBrowser(['screenshot', 'dense.png']);
    assert.equal(result.code, 0, result.stderr);
    const [width, height] = viewport.value;
    assert.deepEqual(pngSize(fs.readFileSync(path.join(dir, 'dense.png'))), { width, height });
  } finally {
    await browser.send('Browser.setWindowBounds', { windowId, bounds: { left: 0, top: 0 } });
    browser.close();
  }
});

test('handover gives the user control; done and stuck report the state', async () => {
  const bad = await agentBrowser(['handover', '--reason', 'tired']);
  assert.equal(bad.code, 2);
  const handover = await agentBrowser(['handover', '--reason', 'password', '--note', '要输登录密码']);
  assert.equal(handover.code, 0, handover.stderr);
  assert.match(handover.stdout, /^handover asked \(group default\): the user has control now/);
  assert.deepEqual(called('browser.request_handover')[0].params, { session_id: 'session-1', reason: 'password', note: '要输登录密码' });
  const status = await agentBrowser(['status']);
  assert.match(status.stdout, /group default: user has control, agent waiting_takeover, held by the user/);
  assert.match(status.stdout, /handover asked: password: 要输登录密码/);
  const blocked = await agentBrowser(['run', script('noop.mjs', 'export default async () => 1;\n')]);
  assert.equal(blocked.code, 4);

  service.group('default').user_hold = false;
  service.group('default').handover = null;
  const done = await agentBrowser(['done', '--note', '订单已提交']);
  assert.equal(done.code, 0, done.stderr);
  assert.equal(done.stdout, 'reported done (group default)\n');
  assert.equal(service.group('default').agent_state, 'done');
  assert.equal(service.group('default').note, '订单已提交');
  assert.equal((await agentBrowser(['stuck'])).code, 2);
  const stuck = await agentBrowser(['stuck', '--note', '验证码过不去']);
  assert.equal(stuck.code, 0, stuck.stderr);
  assert.equal(service.group('default').agent_state, 'stuck');
});

test('version, env and help need no Agent Service', async () => {
  const none = { OPENDAN_BROWSER_RPC: '', OPENDAN_SESSION_ID: '' };
  const version = await agentBrowser(['version'], none);
  assert.equal(version.code, 0, version.stderr);
  assert.match(version.stdout, /^chromium \d+\.\d+\.\d+\.\d+ \(revision \d+\)\nplaywright-core \d+\.\d+\.\d+\n$/);
  for (const args of [['--help'], ['run', '--help'], ['reflow-run', '--help']]) {
    const help = await agentBrowser(args, none);
    assert.equal(help.code, 0);
    assert.match(help.stdout, /^agent-browser/);
  }
  assert.equal((await agentBrowser([], none)).code, 2);
  assert.equal((await agentBrowser(['fly'], none)).code, 2);

  // The payload's layout: store/ext-core/{browser,conda,playwright}.
  const store = path.join(dir, 'store', 'ext-core');
  fs.mkdirSync(path.join(store, 'conda', 'lib'), { recursive: true });
  fs.cpSync(BROWSER_DIR, path.join(store, 'browser'), { recursive: true, filter: (source) => path.basename(source) !== 'test' });
  fs.writeFileSync(path.join(store, 'browser', 'versions.json'), '{"playwright": "9.8.7", "chromium": "1.2.3.4", "revision": "42"}');
  const packaged = path.join(store, 'browser', 'agent-browser.mjs');
  const packagedVersion = await node(packaged, ['version']);
  assert.equal(packagedVersion.stdout, 'chromium 1.2.3.4 (revision 42)\nplaywright-core 9.8.7\n');
  const env = await node(packaged, ['env'], { env: { LD_LIBRARY_PATH: '' } });
  assert.equal(env.code, 0, env.stderr);
  if (process.platform === 'linux') {
    assert.equal(
      env.stdout,
      [
        'export PLAYWRIGHT_BROWSERS_PATH=0',
        'export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1',
        `export LD_LIBRARY_PATH='${store}/conda/lib'`,
        `export FONTCONFIG_FILE='${store}/conda/etc/fonts/fonts.conf'`,
        `export FONTCONFIG_PATH='${store}/conda/etc/fonts'`,
        '',
      ].join('\n'),
    );
  } else {
    assert.match(env.stdout, /^export PLAYWRIGHT_BROWSERS_PATH=0\n# /);
  }
});

test('reflow-run opens the address in a new tab, runs refresh.mjs, writes the data and closes the tab', async () => {
  const pack = path.join(dir, 'pack');
  fs.mkdirSync(pack);
  fs.writeFileSync(
    path.join(pack, 'refresh.mjs'),
    `export default async ({ page, url, save }) => ({
  url,
  headline: await page.locator('#headline').textContent(),
  photo: await save.image(await page.locator('#photo').getAttribute('src')),
});
`,
  );
  const out = path.join(dir, 'result.json');
  const reflow = (extra) => node(AGENT_BROWSER, ['reflow-run', '--cdp', chromium.cdpUrl, '--url', `${site.url}/article`, '--package', pack, '--out', out, ...extra]);
  const plain = await reflow([]);
  assert.equal(plain.code, 0, plain.stderr);
  const data = JSON.parse(fs.readFileSync(out, 'utf8'));
  assert.equal(data.url, `${site.url}/article`);
  assert.equal(data.headline, '原标题');
  assert.match(data.photo, /^data:image\/jpeg;base64,/);
  assert.deepEqual(jpegSize(Buffer.from(data.photo.split(',')[1], 'base64')), { width: 1280, height: 320 });
  assert.deepEqual((await chromium.targets()).map((target) => target.id), [tab.id]);

  const clicked = await reflow(['--click', '#more']);
  assert.equal(clicked.code, 0, clicked.stderr);
  assert.equal(JSON.parse(fs.readFileSync(out, 'utf8')).headline, '点过之后的标题');

  fs.writeFileSync(path.join(pack, 'refresh.mjs'), "export default async () => { throw new Error('页面改版了'); };\n");
  const failed = await reflow([]);
  assert.equal(failed.code, 1);
  assert.equal(failed.stderr.trimEnd().split('\n').at(-1), 'agent-browser: reflow failed: 页面改版了');
  assert.deepEqual((await chromium.targets()).map((target) => target.id), [tab.id]);

  // What a page that changed looks like: the reason names the part that is missing, well before the run is over.
  fs.writeFileSync(path.join(pack, 'refresh.mjs'), "export default async ({ page }) => page.locator('#gone-after-redesign').innerText();\n");
  const started = Date.now();
  const missing = await reflow(['--timeout', '9']);
  assert.equal(missing.code, 1);
  assert.match(missing.stderr.trimEnd().split('\n').at(-1), /^agent-browser: reflow failed: locator\.innerText: Timeout 3000ms exceeded/);
  assert.match(missing.stderr, /#gone-after-redesign/);
  assert.ok(Date.now() - started < 8000, 'fails before the run times out');
  assert.deepEqual((await chromium.targets()).map((target) => target.id), [tab.id]);
});

test('a tab with an unanswered dialog is named when attaching fails', async () => {
  const watcher = await Cdp.open(tab.webSocketDebuggerUrl);
  await watcher.send('Page.enable');
  watcher.send('Runtime.evaluate', { expression: "alert('nobody answers')" }).catch(() => {});
  await watcher.waitFor('Page.javascriptDialogOpening');
  const pack = path.join(dir, 'pack-blocked');
  fs.mkdirSync(pack);
  fs.writeFileSync(path.join(pack, 'refresh.mjs'), 'export default async () => 1;\n');
  const result = await node(AGENT_BROWSER, ['reflow-run', '--cdp', chromium.cdpUrl, '--url', `${site.url}/article`, '--package', pack, '--out', path.join(dir, 'never.json'), '--timeout', '3']);
  assert.equal(result.code, 1);
  assert.match(result.stderr, /cannot attach to the browser: .*these tabs do not answer, probably because of a dialog .*\/blank/);
  await watcher.send('Page.handleJavaScriptDialog', { accept: true });
  watcher.close();
});
