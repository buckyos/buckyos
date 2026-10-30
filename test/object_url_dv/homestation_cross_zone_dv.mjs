// Cross-zone HomeStation over https references (notepads/agent-object-url-todo.md D10 / D11):
// bob publishes in his zone, alice follows and pulls it into hers, and her comment is delivered
// to bob's inbox through the dispatch target `https://<bob zone>/home/bob/inbox`. Every entry,
// stream and inbox address is the https form; no `cyfs://<zone>/home` survives.
//
// Run against the ssotest VM group (src/dev_configs/ssotest.json):
//   BUCKYOS_TEST_DNS=<sn ip> node test/object_url_dv/homestation_cross_zone_dv.mjs
// Env as test/test_msg_center/test_native_routes_dv.mjs: BUCKYOS_TEST_CA, BUCKYOS_TEST_PASSWORD,
// BUCKYOS_TEST_ZONE_A / BUCKYOS_TEST_ZONE_B, BUCKYOS_TEST_HOST_MAP.
import { request } from 'node:https';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { Resolver } from 'node:dns';
import { homedir } from 'node:os';

const dnsServer = process.env.BUCKYOS_TEST_DNS?.trim();
if (!dnsServer) throw new Error('BUCKYOS_TEST_DNS (the SN IP) is required');
const ca = readFileSync(process.env.BUCKYOS_TEST_CA?.trim() || `${homedir()}/buckycli/ca/buckyos_test_ca_ca_cert.pem`);
const password = process.env.BUCKYOS_TEST_PASSWORD?.trim() || 'bucky2025';
const resolver = new Resolver();
resolver.setServers([dnsServer]);
const resolved = new Map((process.env.BUCKYOS_TEST_HOST_MAP ?? '').split(',')
  .map(pair => pair.trim().split('=')).filter(pair => pair.length === 2));
function lookup(host, options, callback) {
  const done = address => options.all ? callback(null, [{ address, family: 4 }]) : callback(null, address, 4);
  if (resolved.has(host)) return done(resolved.get(host));
  resolver.resolve4(host, (error, addresses) => {
    if (error || !addresses.length) return callback(error ?? new Error(`no A record for ${host}`), '', 4);
    resolved.set(host, addresses[0]);
    done(addresses[0]);
  });
}
const zones = {
  alice: { host: process.env.BUCKYOS_TEST_ZONE_A?.trim() || 'alice.web3.devtests.org', did: 'did:bns:alice' },
  bob: { host: process.env.BUCKYOS_TEST_ZONE_B?.trim() || 'bob.web3.devtests.org', did: 'did:bns:bob' },
};
let seq = Date.now();
function http(user, method, path, body, token) {
  const zone = zones[user];
  const payload = body === undefined ? undefined : JSON.stringify(body);
  return new Promise((resolve, reject) => {
    const headers = payload ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) } : {};
    if (token) headers.Authorization = `Bearer ${token}`;
    const req = request(`https://${zone.host}${path}`, { method, ca, timeout: 30000, lookup, headers }, res => {
      let content = '';
      res.on('data', chunk => content += chunk);
      res.on('end', () => resolve({ status: res.statusCode, body: content }));
    });
    req.on('timeout', () => req.destroy(new Error(`${user} ${path}: timeout`)));
    req.on('error', reject);
    req.end(payload);
  });
}
async function rpc(user, service, method, params, token = null) {
  const r = await http(user, 'POST', `/kapi/${service}`, { method, params, sys: token ? [++seq, token] : [++seq] });
  if (r.status !== 200) throw new Error(`${user} ${method}: HTTP ${r.status} ${r.body.slice(0, 200)}`);
  const result = JSON.parse(r.body);
  if (result.error) throw new Error(`${user} ${method}: ${JSON.stringify(result.error)}`);
  return result.result;
}
function sha(value) { return createHash('sha256').update(value).digest('base64'); }
async function login(user) {
  const nonce = Date.now();
  const result = await rpc(user, 'control-panel', 'auth.login', {
    username: user, password: sha(sha(`${password}${user}.buckyos`) + nonce), login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
  });
  if (!result.session_token) throw new Error(`${user}: login failed`);
  return result.session_token;
}
function assert(cond, message) {
  if (!cond) throw new Error(`assert failed: ${message}`);
}
const sleep = ms => new Promise(r => setTimeout(r, ms));

const aliceToken = await login('alice');
const bobToken = await login('bob');
const bobHome = `https://${zones.bob.host}/home/bob`;

// 1. bob's home addresses and his Provider's default url are https
const profile = JSON.parse((await http('bob', 'GET', '/home/bob/profile')).body);
assert(profile.stream === `${bobHome}/feed` && profile.inbox === `${bobHome}/inbox`, `bob profile ${JSON.stringify(profile)}`);
const card = await http('bob', 'GET', '/kapi/homestation/obj/home/bob/feed', undefined, bobToken);
assert(card.status === 200 && JSON.parse(card.body).alsoKnownAs[0] === `${bobHome}/feed`, `bob feed Card ${card.status} ${card.body.slice(0, 200)}`);
console.log(JSON.stringify({ step: 'addresses', stream: profile.stream, inbox: profile.inbox }));

// 2. bob publishes a public post
const stamp = Date.now();
const post = await rpc('bob', 'homestation', 'publish.create', {
  key: `xz-${stamp}`,
  input: { text: `cross-zone https entry ${stamp}`, attachments: [], link: null, audience: { kind: 'public' } },
}, bobToken);
const objId = post.objId;
assert(post.stage === 'published' && objId, `publish ${JSON.stringify(post).slice(0, 200)}`);
const display = JSON.parse((await http('bob', 'GET', '/home/bob/feed?mode=display&limit=10')).body);
const entry = (display.items ?? []).find(item => item.current === objId)?.entry;
assert(String(entry).startsWith(`${bobHome}/feed/@/`), `bob's entry ${entry}`);
console.log(JSON.stringify({ step: 'published', entry, objId }));

// 3. alice follows bob across zones and pulls the post
const resolution = await rpc('alice', 'homestation', 'sources.resolve', { kind: 'follow', text: zones.bob.did }, aliceToken);
await rpc('alice', 'homestation', 'sources.follow', { resolution }, aliceToken).catch(e => console.log(`  follow: ${e.message.slice(0, 160)}`));
let card2 = null;
for (let i = 0; i < 30 && !card2; i++) {
  await rpc('alice', 'homestation', 'sources.sync', {}, aliceToken).catch(() => {});
  card2 = await rpc('alice', 'homestation', 'item.get', { objId }, aliceToken).catch(() => null);
  if (!card2) await sleep(2000);
}
assert(card2, 'alice has bob\'s post after following and pulling');
const seenEntry = card2.item?.entry?.url ?? card2.item?.entry?.entry ?? card2.item?.entry;
console.log(JSON.stringify({ step: 'pulled', entry: seenEntry }));
assert(JSON.stringify(card2).includes(`${bobHome}/feed/@/`) && !JSON.stringify(card2).includes('cyfs://'), `alice's card refers to bob's https entry: ${JSON.stringify(card2).slice(0, 400)}`);

// 4. alice comments; the comment is delivered to bob's inbox (dispatch target https)
const text = `cross-zone comment ${stamp}`;
await rpc('alice', 'homestation', 'interact.comment', { objId, text }, aliceToken);
let delivered = false;
for (let i = 0; i < 45 && !delivered; i++) {
  const comments = await rpc('bob', 'homestation', 'comments.list', { objId, view: 'local', type: 'text' }, bobToken).catch(() => ({ comments: [] }));
  delivered = JSON.stringify(comments).includes(text);
  if (!delivered) await sleep(2000);
}
assert(delivered, 'alice\'s comment reached bob\'s inbox');
const status = await http('bob', 'GET', `/home/bob/inbox?dispatch-status=x`, undefined, bobToken);
console.log(JSON.stringify({ step: 'delivered', inbox_status_probe: status.status }));
console.log('HomeStation cross-zone https DV: all checks passed');
