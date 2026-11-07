// Desktop against a real Zone (issue #689): a stranger's attachment says to accept the sender, accepting brings it by
// itself, a card moves no bytes, a sender that is away shows as being fetched and then offers a retry, which works once
// it is back. Charlie reads what alice sends, in the UI, through the dev server's Zone proxy:
//
//   (src/frame/desktop) VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://charlie.me VITE_ZONE_PROXY_IP=<charlie ip> \
//     VITE_ZONE_HOST=charlie.me pnpm exec vite --host 127.0.0.1 --port 4175 --strictPort
//   (test/nfs_server_test) BUCKYOS_TEST_DNS=<sn ip> BUCKYOS_TEST_DEVICE_DIR=<dir> \
//     BUCKYOS_TEST_AUTHOR_DOWN='<takes alice off the network>' BUCKYOS_TEST_AUTHOR_UP='<brings her back>' \
//     node attachment_ui_dv.mjs
//
// See group_attachment_dv.mjs for taking alice off the network. BUCKYOS_TEST_UI_SHOTS names a directory for screenshots.
import { createRequire } from 'node:module';
import { createHash, randomBytes } from 'node:crypto';
import { crc32, deflateSync } from 'node:zlib';
import { awaitMessage, check, describe, describedRemotely, finish, login, runGiven, send, setAccess, upload, zones } from './attachment_dv_common.mjs';
const require = createRequire('/home/lzc/projects/buckyos/src/frame/desktop/package.json');
const { chromium } = require('@playwright/test');

const shot = process.env.BUCKYOS_TEST_UI_SHOTS?.trim();
const base = process.env.BUCKYOS_TEST_UI_BASE?.trim() || 'http://127.0.0.1:4175';
const tokens = { alice: await login('alice'), charlie: await login('charlie') };
await setAccess('charlie', 'alice', 'stranger', tokens.charlie);

function png(width, height) {
  const chunk = (type, data) => { const t = Buffer.from(type); const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([t, data]))>>>0); return Buffer.concat([len, t, data, crc]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 2;
  const raw = Buffer.alloc((width * 3 + 1) * height, 0x7f); for (let y = 0; y < height; y++) raw[y * (width * 3 + 1)] = 0;
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
const image = await upload('alice', tokens.alice, `ui-${Date.now()}.png`, png(64, 48));
const doc = await upload('alice', tokens.alice, `ui-${Date.now()}.txt`, Buffer.from(`card only ${randomBytes(8).toString('hex')}\n`.repeat(128)));
const first = await send('alice', 'charlie', [image, doc], tokens.alice);
await awaitMessage('charlie', 'alice', first.text, tokens.charlie);

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1280, height: 860 } });
const page = await context.newPage();
const nonce = Date.now();
const sha = v => createHash('sha256').update(v, 'utf8').digest('base64');
const auth = await (await context.request.post(`${base}/kapi/control-panel`, { data: { method: 'auth.login', params: { username: 'charlie', password: sha(sha('bucky2025charlie.buckyos') + nonce), appid: 'control-panel', target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce, remember_me: true }, sys: [nonce] } })).json();
if (!auth.result) throw new Error(JSON.stringify(auth));
await context.addCookies([{ name: 'buckyos_refresh_token', value: auth.result.refresh_token, domain: '127.0.0.1', path: '/', httpOnly: true, secure: false, sameSite: 'Lax' }]);
await page.addInitScript(info => { try { window.localStorage.setItem('user_info', JSON.stringify(info)); } catch {} }, auth.result.user_info);
page.on('pageerror', error => console.log('pageerror', error.message));

await page.goto(`${base}/messagehub?${new URLSearchParams({ entityId: zones.alice.did, sessionId: `dm:${zones.alice.did}` })}`);
const history = page.getByTestId('conversation-history');
const waiting = history.getByTestId('attachment-awaiting').filter({ hasText: image.name });
await waiting.waitFor({ timeout: 60000 });
check(true, 'real Zone: a stranger\'s image shows "accept to view", no failure');
if (shot) await page.screenshot({ path: `${shot}/1-awaiting.png` });

await page.evaluate(async () => { const m = '/src/components/network/cache.ts'; const { networkActions } = await import(/* @vite-ignore */ m); await networkActions.allowMessages('did:bns:alice'); });
await history.getByRole('img', { name: image.name, exact: true }).waitFor({ timeout: 60000 });
check(true, 'real Zone: accepting brings the image, by itself');
const fileCard = history.getByTestId('attachment-file').filter({ hasText: doc.name });
await fileCard.waitFor({ timeout: 60000 });
check(true, 'real Zone: the other attachment is a card');
if (shot) await page.screenshot({ path: `${shot}/2-accepted.png` });
const card = await describe('charlie', tokens.charlie, doc.id, first.msgId);
check(describedRemotely(card), 'real Zone: the card fetched nothing');
const imageDescribed = await describe('charlie', tokens.charlie, image.id, first.msgId);
check(!describedRemotely(imageDescribed), 'real Zone: the image was fetched for showing');
await fileCard.getByTestId('attachment-download').click();
for (let i = 0; i < 40; i++) { if (!describedRemotely(await describe('charlie', tokens.charlie, doc.id, first.msgId))) break; await new Promise(r => setTimeout(r, 500)); }
check(!describedRemotely(await describe('charlie', tokens.charlie, doc.id, first.msgId)), 'real Zone: a click on download fetched it');

// The sender's Zone goes away: the card says it is trying, then offers a retry; back, a retry works.
await page.goto('about:blank');
const late = await upload('alice', tokens.alice, `ui-late-${Date.now()}.png`, png(32, 24));
const second = await send('alice', 'charlie', [late], tokens.alice);
await awaitMessage('charlie', 'alice', second.text, tokens.charlie);
const away = runGiven('BUCKYOS_TEST_AUTHOR_DOWN', 'the sender offline');
try {
  if (away) {
    await page.goto(`${base}/messagehub?${new URLSearchParams({ entityId: zones.alice.did, sessionId: `dm:${zones.alice.did}` })}`);
    const pending = history.getByTestId('attachment-fetching').filter({ hasText: late.name });
    await pending.waitFor({ timeout: 90000 });
    check(true, 'real Zone: the unreachable sender shows as being fetched first');
    if (shot) await page.screenshot({ path: `${shot}/3-fetching.png` });
    const failed = history.getByTestId('attachment-error').filter({ hasText: late.name });
    await failed.waitFor({ timeout: 240000 });
    check((await failed.getAttribute('data-failure')) === 'unreachable', 'real Zone: then gives up, with a retry');
    if (shot) await page.screenshot({ path: `${shot}/4-unreachable.png` });
    runGiven('BUCKYOS_TEST_AUTHOR_UP', 'the sender back');
    await new Promise(r => setTimeout(r, 30000));
    await failed.getByRole('button').click();
    await history.getByRole('img', { name: late.name, exact: true }).waitFor({ timeout: 180000 });
    check(true, 'real Zone: the retry works once the sender is back, no resend');
    if (shot) await page.screenshot({ path: `${shot}/5-retried.png` });
  }
} finally {
  if (away) runGiven('BUCKYOS_TEST_AUTHOR_UP', 'the sender back');
  await browser.close();
}
await setAccess('charlie', 'alice', 'stranger', tokens.charlie);
finish('Desktop UI against the real Zone');
