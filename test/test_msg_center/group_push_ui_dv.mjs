import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { login, lookup, rpc, sleep, zones } from '../nfs_server_test/attachment_dv_common.mjs';

const requireDesktop = createRequire(new URL('../../src/frame/desktop/package.json', import.meta.url));
const { chromium } = requireDesktop('@playwright/test');
const group = process.env.BUCKYOS_TEST_JOINED_GROUP;
if (!group) throw new Error('BUCKYOS_TEST_JOINED_GROUP must name a group already synced by the member');
const host = process.env.BUCKYOS_TEST_GROUP_HOST ?? 'bob';
if (!['alice', 'bob'].includes(host)) throw new Error('BUCKYOS_TEST_GROUP_HOST must be alice or bob');
const member = host === 'bob' ? 'alice' : 'bob';
const count = Number(process.env.BUCKYOS_TEST_UI_SAMPLES ?? 24);
if (!Number.isSafeInteger(count) || count < 2) throw new Error('BUCKYOS_TEST_UI_SAMPLES must be an integer >= 2');
const base = new URL(process.env.BUCKYOS_TEST_UI_BASE ?? `https://${zones[member].host}`);
const directory = process.env.BUCKYOS_TEST_UI_OUT || tmpdir();
await mkdir(directory, { recursive: true });
const out = await mkdtemp(join(directory, 'group-push-ui-'));
console.log(`Browser samples: ${out}`);

// Observe the existing WebSDK reader. Opening an additional subscription would measure a different connection.
function installProbe({ group, owner, origin, userInfo }) {
  if (window.top !== window || location.origin !== origin) return;
  localStorage.setItem('user_info', JSON.stringify(userInfo));
  const probe = window.groupPushProbe = { label: null, events: {}, dom: {}, frames: {}, acked: false };
  const parse = JSON.parse;
  JSON.parse = function (...args) {
    const frame = Reflect.apply(parse, this, args);
    if (frame?.type === 'ack' && frame.connection_id) probe.acked = true;
    if (frame?.type === 'event') {
      const event = frame.event?.data;
      if (event?.operation === 'upsert' && event.owner === owner && event.session_id === group &&
          String(event.box_kind).toUpperCase() === 'INBOX' && event.msg_id && !Object.hasOwn(probe.events, event.msg_id)) {
        probe.events[event.msg_id] = performance.now();
      }
    }
    return frame;
  };

  let scheduled;
  function findMessage() {
    const label = probe.label;
    if (!label || probe.frames[label] || scheduled === label) return;
    const history = document.querySelector('[data-testid="conversation-history"]');
    if (!history) return;
    const walker = document.createTreeWalker(history, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.textContent.trim() !== label) continue;
      probe.dom[label] ??= performance.now();
      scheduled = label;
      const range = document.createRange();
      range.selectNodeContents(node);
      function firstVisibleFrame() {
        if (probe.label !== label || !node.isConnected) {
          scheduled = null;
          findMessage();
          return;
        }
        const textBox = range.getBoundingClientRect();
        const panel = history.getBoundingClientRect();
        const top = Math.max(textBox.top, panel.top, 0), bottom = Math.min(textBox.bottom, panel.bottom, innerHeight);
        const left = Math.max(textBox.left, panel.left, 0), right = Math.min(textBox.right, panel.right, innerWidth);
        const element = bottom > top && right > left ? document.elementFromPoint((left + right) / 2, (top + bottom) / 2) : null;
        if (element && (element.contains(node) || node.parentElement?.contains(element))) {
          const at = performance.now();
          probe.frames[label] = { at, epoch: performance.timeOrigin + at };
          scheduled = null;
        } else {
          requestAnimationFrame(firstVisibleFrame);
        }
      }
      requestAnimationFrame(firstVisibleFrame);
      return;
    }
  }
  document.addEventListener('DOMContentLoaded', () => {
    new MutationObserver(findMessage).observe(document.body, { childList: true, subtree: true, characterData: true });
  }, { once: true });
}

async function browserLogin(context) {
  const nonce = Date.now();
  const sha = value => createHash('sha256').update(value).digest('base64');
  const password = process.env.BUCKYOS_TEST_PASSWORD ?? 'bucky2025';
  const auth = await rpc(member, 'control-panel', 'auth.login', {
    username: member, password: sha(sha(`${password}${member}.buckyos`) + nonce), login_nonce: nonce, remember_me: true,
    target: { kind: 'system', service_id: 'control-panel' }, appid: 'control-panel',
  });
  if (!auth.session_token || !auth.refresh_token) throw new Error(`${member} did not receive a session and refresh token`);
  await context.addCookies([{ name: 'buckyos_refresh_token', value: auth.refresh_token, url: base.origin,
    httpOnly: true, secure: base.protocol === 'https:', sameSite: 'Lax' }]);
  await context.addInitScript(installProbe, { group, owner: zones[member].did, origin: base.origin, userInfo: auth.user_info });
  return auth.session_token;
}

function stats(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  const q = p => sorted[Math.ceil(sorted.length * p) - 1] ?? null;
  return { n: sorted.length, p50_ms: q(0.50), p95_ms: q(0.95), max_ms: q(1) };
}

const domain = zones[member].host;
const address = await promisify(lookup)(domain, {});
const browser = await chromium.launch({ args: ['--no-proxy-server', '--ignore-certificate-errors',
  '--disable-background-timer-throttling', '--disable-renderer-backgrounding',
  `--host-resolver-rules=MAP ${domain} ${address},MAP *.${domain} ${address}`] });
try {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, serviceWorkers: 'block', viewport: { width: 1365, height: 900 } });
  const [memberToken, hostToken] = await Promise.all([browserLogin(context), login(host)]);
  const run = Date.now();
  const report = { group, host, member, run, base: base.origin, browser: browser.version(), samples: [] };
  const save = () => writeFile(join(out, 'samples.json'), JSON.stringify(report, null, 2) + '\n');
  const page = await context.newPage();
  const url = new URL('/messagehub', base);
  url.search = new URLSearchParams({ entityId: group, sessionId: group });
  await page.goto(url.href, { waitUntil: 'domcontentloaded' });
  await page.getByTestId('conversation-history').waitFor({ timeout: 90_000 });
  try {
    await page.waitForFunction(() => window.groupPushProbe?.acked, null, { timeout: 10_000 });
  } catch (error) {
    if (error.name !== 'TimeoutError') throw error;
    console.warn('No event subscription acknowledgement in 10 seconds; recording available display timings');
  }
  for (let i = 0; i < count; i++) {
    if (i) await sleep(550);
    const label = `ui737-${run}-${i + 1}`;
    await page.evaluate(label => { window.groupPushProbe.label = label; }, label);
    const at = Date.now();
    const sent = await rpc(host, 'msg-center', 'msg.post_send', { idempotency_key: label, msg: {
      from: zones[host].did, to: [group], kind: 'group_msg', created_at_ms: at, nonce: at,
      content: { format: 'text/plain', content: label },
    } }, hostToken);
    if (!sent.ok) throw new Error(`sample ${i + 1} refused: ${sent.reason}`);
    const id = sent.msg_id;
    await page.waitForFunction(label => Boolean(window.groupPushProbe.frames[label]), label, { timeout: 45_000 });
    const { frame, dom, event } = await page.evaluate(({ label, id }) => {
      const { frames, dom, events } = window.groupPushProbe;
      return { frame: frames[label], dom: dom[label], event: events[id] ?? null };
    }, { label, id });
    const records = await rpc(member, 'msg-center', 'msg.peek_box', {
      mailbox: `${zones[member].did}/${group}`, box_kind: 'INBOX', limit: 50, with_object: false,
    }, memberToken);
    const record = records.find(item => item.record.msg_id === id)?.record;
    if (!record) throw new Error(`sample ${i + 1} missing from member INBOX`);
    const row = { n: i + 1, id, member_db_ms: record.created_at_ms, host_accepted_ms: record.sort_key,
      ui_epoch_ms: Math.round(frame.epoch),
      host_to_visible_ms: Math.round(frame.epoch - record.sort_key),
      member_db_to_visible_ms: Math.round(frame.epoch - record.created_at_ms),
      kevent_to_visible_ms: event === null ? null : Math.round(frame.at - event),
      dom_to_visible_ms: Math.round(frame.at - dom) };
    report.samples.push(row);
    console.log(JSON.stringify(row));
    await save();
  }
  report.summary = Object.fromEntries(['host_to_visible_ms', 'member_db_to_visible_ms', 'kevent_to_visible_ms', 'dom_to_visible_ms']
    .map(key => [key, stats(report.samples.map(row => row[key]))]));
  await save();
  await page.screenshot({ path: join(out, 'last-frame.png') });
  console.log(JSON.stringify(report.summary));
} finally {
  await browser.close();
}
