// Issue #760 in the browser, on real Zones: charlie, in the Desktop deployed on his own Zone, joins a
// group hosted by alice from My Network → Join Group by its DID, and accepts alice's directed
// invitation from My Network → Requests → Group invitations.
//
// Run against the ssotest VM group, charlie's OOD with the Desktop and msg-center of this tree and
// alice's with the msg-center (see cross_zone_join_dv.mjs):
//   BUCKYOS_TEST_DNS=<sn ip> node test/test_msg_center/cross_zone_join_ui_dv.mjs
// Uses the Chromium of src/frame/desktop's @playwright/test. BUCKYOS_TEST_UI_OUT is where the
// screenshots go (the system temp directory by default). Leaves two `#760 ui-…` groups on alice's Zone.
// BUCKYOS_TEST_CHROMIUM overrides the Chromium executable.
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { check, finish, http, log, login, lookup, rpc, sleep, zones } from '../nfs_server_test/attachment_dv_common.mjs';

const requireDesktop = createRequire(new URL('../../src/frame/desktop/package.json', import.meta.url));
const { chromium } = requireDesktop('@playwright/test');
const HOST = 'alice';
const MEMBER = 'charlie';
const did = user => zones[user].did;
const stamp = Date.now();
const base = new URL(`https://${zones[MEMBER].host}`);
const directory = process.env.BUCKYOS_TEST_UI_OUT || tmpdir();
await mkdir(directory, { recursive: true });
const out = await mkdtemp(join(directory, 'cross-zone-join-ui-'));
log('screenshots', { out });

const hostToken = await login(HOST);
const memberToken = await login(MEMBER);
const host = (method, params) => rpc(HOST, 'msg-center', method, params, hostToken);
const member = (method, params) => rpc(MEMBER, 'msg-center', method, params, memberToken);

async function createGroup(label, membership) {
  const created = await host('group.create', {
    idempotency_key: `g760-${label}-${stamp}`,
    configuration: { profile: { name: `#760 ${label} ${stamp}` }, default_session: { history: 'all' }, membership },
  });
  return created.group_did;
}
async function awaitPublished(group) {
  const name = group.startsWith('did:bns:') ? `${group.slice('did:bns:'.length)}.${zones[HOST].host.split('.').slice(1).join('.')}` : group.slice('did:web:'.length);
  for (let attempt = 0; attempt < 60; attempt++) {
    const answer = await http(HOST, 'GET', '/.well-known/did.json', { headers: { Host: name } });
    if (answer.status === 200 && answer.text().startsWith('ey')) return;
    await sleep(3000);
  }
  throw new Error(`the document of ${group} was not published`);
}
const joined = async group => ((await member('group.list_by_member', {})).joined ?? []).some(item => item.group_did === group);

const open = await createGroup('ui-open', { join_policy: 'open' });
const invited = await createGroup('ui-invite', {});
await Promise.all([awaitPublished(open), awaitPublished(invited)]);
const invitation = await host('group.invite_member', { group_did: invited, member_did: did(MEMBER) });
for (let attempt = 0; ; attempt++) {
  const listed = await member('group.list_by_member', {});
  if (listed.invitations?.some(item => item.group_did === invited && item.member?.invitation_id === invitation.invite_id)) break;
  if (attempt > 30) throw new Error('the invitation did not reach charlie\'s Zone');
  await sleep(2000);
}
log('groups', { open, invited });

async function browserLogin(context) {
  const nonce = Date.now();
  const sha = value => createHash('sha256').update(value).digest('base64');
  const password = process.env.BUCKYOS_TEST_PASSWORD ?? 'bucky2025';
  const auth = await rpc(MEMBER, 'control-panel', 'auth.login', {
    username: MEMBER, password: sha(sha(`${password}${MEMBER}.buckyos`) + nonce), login_nonce: nonce, remember_me: true,
    target: { kind: 'system', service_id: 'control-panel' }, appid: 'control-panel',
  });
  await context.addCookies([{ name: 'buckyos_refresh_token', value: auth.refresh_token, url: base.origin, httpOnly: true, secure: true, sameSite: 'Lax' }]);
  await context.addInitScript(({ origin, userInfo }) => {
    if (location.origin === origin) localStorage.setItem('user_info', JSON.stringify(userInfo));
  }, { origin: base.origin, userInfo: auth.user_info });
}

const domain = zones[MEMBER].host;
const address = await promisify(lookup)(domain, {});
// BUCKYOS_TEST_CHROMIUM: a Chromium to use when the one this @playwright/test expects is not installed.
const executablePath = process.env.BUCKYOS_TEST_CHROMIUM?.trim() || undefined;
const browser = await chromium.launch({ executablePath, args: ['--no-proxy-server', '--ignore-certificate-errors',
  `--host-resolver-rules=MAP ${domain} ${address},MAP *.${domain} ${address}`] });
const errors = [];
try {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, serviceWorkers: 'allow', viewport: { width: 1440, height: 900 }, locale: 'en-US' });
  await browserLogin(context);
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(base.href, { waitUntil: 'domcontentloaded' });
  await page.getByTestId('desktop-app-my-network').click({ timeout: 90_000 });
  const win = page.getByTestId('window-my-network');
  await win.getByTestId('my-network-home').waitFor({ timeout: 60_000 });

  // Join Group by DID: the preview of a group hosted by another Zone has a Join button.
  await win.getByTestId('my-network-join-group').click();
  const dialog = win.getByRole('dialog', { name: 'Join Group' });
  await dialog.getByLabel('Group DID or invite link').fill(open);
  await dialog.getByRole('button', { name: 'Find', exact: true }).click();
  const preview = dialog.getByTestId('join-group-preview');
  await preview.waitFor({ timeout: 60_000 });
  await dialog.getByTestId('join-group-submit').waitFor({ timeout: 60_000 });
  const previewText = await preview.innerText();
  await page.screenshot({ path: join(out, '1-join-preview.png') });
  check(previewText.includes('Hosted by another Zone'), 'preview: hosted by another Zone', previewText);
  check(previewText.includes(`#760 ui-open ${stamp}`), 'preview: the name from the host\'s document', previewText);
  check(previewText.includes('Anyone can join'), 'preview: the host\'s join policy', previewText);
  await dialog.getByTestId('join-group-submit').click();
  const outcome = dialog.getByTestId('join-group-outcome');
  await outcome.waitFor({ timeout: 60_000 });
  check(await outcome.getAttribute('data-outcome') === 'active', 'Join: joined');
  await page.screenshot({ path: join(out, '2-join-outcome.png') });
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  check(await joined(open), 'charlie\'s Zone lists the group as joined');

  // Requests → Group invitations: alice's invitation, named, joins on Accept.
  await win.getByTestId('nav-requests').click();
  const card = win.getByTestId('request-group-invitation').filter({ hasText: `#760 ui-invite ${stamp}` });
  await card.waitFor({ timeout: 60_000 });
  await page.screenshot({ path: join(out, '3-requests-invitation.png') });
  check(true, 'Requests: the cross-Zone invitation is listed with the group name');
  await card.locator('[data-action="accept"]').click();
  await card.waitFor({ state: 'detached', timeout: 60_000 });
  await page.screenshot({ path: join(out, '4-requests-after-accept.png') });
  const actionError = await win.getByTestId('requests-action-error').count();
  check(actionError === 0, 'Requests: accepting shows no error');
  check(await joined(invited), 'charlie\'s Zone lists the invited group as joined');
  check(errors.length === 0, 'no page errors', errors);
} finally {
  await browser.close();
}
finish('#760 cross-Zone join UI DV');
