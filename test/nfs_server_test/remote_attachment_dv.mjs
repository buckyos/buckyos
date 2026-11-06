// Attachments of private messages across real Zones (bucket design §21 N18, issue #689 P1):
// the recipient's Zone holds nothing when the message arrives and pulls the bytes from the sender's
// Zone on first read. A recipient reads them end to end; nobody else gets a token.
//
// Run against the ssotest VM group (src/dev_configs/ssotest.json), from test/nfs_server_test:
//   BUCKYOS_TEST_DNS=<sn ip> BUCKYOS_TEST_DEVICE_DIR=<dir> node remote_attachment_dv.mjs
//
// A non-recipient asks exactly as its Zone's msg-center would: a cross-Zone assertion signed with
// the device key, then `msg.attachment_ticket`. BUCKYOS_TEST_DEVICE_DIR holds `<user>.pem` (the
// node's private key) and `<user>.did` (its device DID) for bob and charlie; without it those
// checks are reported as NOT VERIFIED and the run does not count as a pass.
//
// The SN also answers AAAA for a NAT Zone: on a LAN with global IPv6, a Zone created before
// ssotest.json rejected IPv6 is reached directly and the relay is not exercised. Which path was
// taken shows in the sender's gateway log, as the source of the `/ndn/` requests.
// Other env as test/test_msg_center/test_native_routes_dv.mjs: BUCKYOS_TEST_CA,
// BUCKYOS_TEST_PASSWORD, BUCKYOS_TEST_ZONE_A/B/C, BUCKYOS_TEST_HOST_MAP.
import { request } from 'node:https';
import { existsSync, readFileSync } from 'node:fs';
import { createHash, createPrivateKey, randomBytes, randomUUID, sign } from 'node:crypto';
import { Resolver } from 'node:dns';
import { homedir } from 'node:os';
import { ndn } from 'buckyos';

const dnsServer = process.env.BUCKYOS_TEST_DNS?.trim();
if (!dnsServer) throw new Error('BUCKYOS_TEST_DNS (the SN IP) is required');
const ca = readFileSync(process.env.BUCKYOS_TEST_CA?.trim() || `${homedir()}/buckycli/ca/buckyos_test_ca_ca_cert.pem`);
const password = process.env.BUCKYOS_TEST_PASSWORD?.trim() || 'bucky2025';
const deviceDir = process.env.BUCKYOS_TEST_DEVICE_DIR?.trim();
const CHUNK = 32 * 1024 * 1024;
const MAX_READ_BYTES = 64 * 1024 * 1024;
const PULL_DEADLINE_MS = 10 * 60 * 1000;

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
  alice: { host: process.env.BUCKYOS_TEST_ZONE_A?.trim() || 'alice.web3.devtests.org', did: 'did:bns:alice', zoneDid: 'did:bns:alice' },
  bob: { host: process.env.BUCKYOS_TEST_ZONE_B?.trim() || 'bob.web3.devtests.org', did: 'did:bns:bob', zoneDid: 'did:bns:bob' },
  charlie: { host: process.env.BUCKYOS_TEST_ZONE_C?.trim() || 'charlie.me', did: 'did:bns:charlie', zoneDid: 'did:web:charlie.me' },
};

let seq = Date.now();
function http(user, method, path, { body, headers = {}, timeout = 120000 } = {}) {
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
    const all = { ...headers };
    if (payload) {
      all['Content-Length'] = payload.length;
      if (!all['Content-Type']) all['Content-Type'] = 'application/json';
    }
    const req = request(`https://${zones[user].host}${path}`, { method, ca, timeout, lookup, headers: all }, res => {
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, bytes: Buffer.concat(chunks), text: () => Buffer.concat(chunks).toString('utf8') }));
    });
    req.on('timeout', () => req.destroy(new Error(`${user} ${method} ${path.split('?')[0]}: timeout`)));
    req.on('error', reject);
    req.end(payload);
  });
}
/** kRPC; a business error comes back as `{error}` so that denials can be compared. */
async function rpcRaw(user, service, method, params, token = null) {
  seq = Math.max(Date.now(), seq + 1);
  const r = await http(user, 'POST', `/kapi/${service}`, { body: { method, params, sys: token ? [seq, token] : [seq] } });
  if (r.status !== 200) return { error: `HTTP ${r.status} ${r.text().slice(0, 200)}` };
  return JSON.parse(r.text());
}
async function rpc(user, service, method, params, token = null) {
  const result = await rpcRaw(user, service, method, params, token);
  if (result.error) throw new Error(`${user} ${method}: ${JSON.stringify(result.error)}`);
  return result.result;
}
const sha = value => createHash('sha256').update(value).digest('base64');
const hex = value => createHash('sha256').update(value).digest('hex');
async function login(user) {
  const nonce = Date.now();
  const result = await rpc(user, 'control-panel', 'auth.login', {
    username: user, password: sha(sha(`${password}${user}.buckyos`) + nonce), login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
  });
  if (!result.session_token) throw new Error(`${user}: login failed`);
  return result.session_token;
}
const sleep = ms => new Promise(r => setTimeout(r, ms));
function log(step, detail = {}) {
  console.log(JSON.stringify({ step, ...detail }));
}
let passed = 0;
const notVerified = [];
function check(condition, label, detail) {
  if (!condition) throw new Error(`FAILED ${label}${detail === undefined ? '' : `: ${typeof detail === 'string' ? detail : JSON.stringify(detail)}`}`);
  passed++;
  console.log(`PASS ${label}`);
}

// ── sending ─────────────────────────────────────────────────────────────────

async function writeChunk(user, token, piece) {
  const chunk = ndn.ChunkId.fromMix256Result(piece.length, ndn.sha256Bytes(piece));
  const r = await http(user, 'PUT', `/ndm/proxy/v1/write/chunk/${chunk.toString()}`, {
    body: Buffer.from(piece),
    timeout: 600000,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/octet-stream', 'NDM-Chunk-Size': String(piece.length) },
  });
  if (r.status >= 300) throw new Error(`${user} write chunk: HTTP ${r.status} ${r.text().slice(0, 200)}`);
  return chunk;
}
/** What Desktop does for an attachment: chunks, then finalize without binding anything. */
async function upload(user, token, name, bytes) {
  const objects = [];
  let content;
  if (bytes.length <= CHUNK) {
    content = (await writeChunk(user, token, bytes)).toString();
  } else {
    const list = new ndn.SimpleChunkList();
    for (let at = 0; at < bytes.length; at += CHUNK) list.appendChunk(await writeChunk(user, token, bytes.subarray(at, Math.min(at + CHUNK, bytes.length))));
    const [listId, listBody] = list.genObjId();
    objects.push({ obj_id: listId.toString(), obj_data: listBody });
    content = listId.toString();
  }
  const [fileObjId, fileBody] = new ndn.FileObject(name, bytes.length, content).genObjId();
  const id = fileObjId.toString();
  objects.push({ obj_id: id, obj_data: fileBody });
  const r = await http(user, 'POST', '/ndm/v1/import/finalize', { body: { objects, roots: [id] }, headers: { Authorization: `Bearer ${token}` } });
  if (r.status !== 200 || JSON.parse(r.text()).ready !== true) throw new Error(`${user} finalize: HTTP ${r.status} ${r.text().slice(0, 300)}`);
  return { id, name, bytes, sha: hex(bytes) };
}
async function send(from, to, files, token) {
  const stamp = Date.now();
  const text = `N18 remote attachment ${from}->${to} ${stamp}`;
  const sent = await rpc(from, 'msg-center', 'msg.post_send', {
    msg: {
      from: zones[from].did, to: [zones[to].did], kind: 'chat', created_at_ms: stamp, nonce: stamp,
      content: {
        format: 'text/plain', content: text,
        refs: files.map(f => ({ role: 'input', target: { type: 'data_obj', obj_id: f.id, uri_hint: `cyfs://${f.id}` }, label: f.name })),
      },
    },
    idempotency_key: `n18-${stamp}`,
  }, token);
  check(sent.ok === true, `${from}→${to}: the message is accepted`, sent);
  return { msgId: sent.msg_id, text };
}
async function awaitMessage(to, from, text, token) {
  for (let attempt = 0; attempt < 60; attempt++) {
    const page = await rpc(to, 'msg-center', 'msg.list_session', {
      owner: zones[to].did, session_id: `dm:${zones[from].did}`, limit: 50, descending: true, with_object: true,
    }, token);
    const found = page.items?.find(item => item.msg?.content?.content === text);
    if (found) return found;
    await sleep(2000);
  }
  throw new Error(`${from}→${to}: message not received in 120 seconds`);
}

// ── reading ─────────────────────────────────────────────────────────────────

function contentPath(id, msgId) {
  return `/kapi/msg-center/objects/${id}/content?context_path=${encodeURIComponent(msgId)}`;
}
function readAttachment(user, token, id, msgId, headers = {}) {
  return http(user, 'GET', contentPath(id, msgId), { timeout: 300000, headers: { Authorization: `Bearer ${token}`, ...headers } });
}
/** The first read starts the pull: 503 + Retry-After until the bytes came and verified. */
async function readUntilPulled(user, token, file, msgId) {
  const started = Date.now();
  let waits = 0;
  for (;;) {
    const r = await readAttachment(user, token, file.id, msgId);
    if (r.status === 200) return { response: r, waits, ms: Date.now() - started };
    check(r.status === 503 && r.headers['retry-after'], `${user} ${file.name}: while pulling, 503 with Retry-After`, `HTTP ${r.status} ${r.text().slice(0, 300)}`);
    if (Date.now() - started > PULL_DEADLINE_MS) throw new Error(`${user} ${file.name}: not pulled within ${PULL_DEADLINE_MS / 1000} s`);
    waits++;
    await sleep(Number(r.headers['retry-after']) * 1000 || 3000);
  }
}
async function verifiedRead(user, token, file, msgId) {
  const { response, waits, ms } = await readUntilPulled(user, token, file, msgId);
  log('pulled', { reader: user, file: file.name, size: file.bytes.length, waits, ms });
  check(hex(response.bytes) === file.sha, `${user} reads ${file.name} (${file.bytes.length} bytes) byte for byte`, `${response.bytes.length} bytes`);
  check(response.headers['x-content-type-options'] === 'nosniff', `${user} ${file.name}: nosniff`);
  const again = Date.now();
  const second = await readAttachment(user, token, file.id, msgId);
  check(second.status === 200 && hex(second.bytes) === file.sha, `${user} ${file.name}: a second read comes from the local binding`, `HTTP ${second.status}, ${Date.now() - again} ms`);
  return response;
}

// ── asking as another Zone would ────────────────────────────────────────────

function device(user) {
  if (!deviceDir || !existsSync(`${deviceDir}/${user}.pem`)) return null;
  return { did: readFileSync(`${deviceDir}/${user}.did`, 'utf8').trim(), key: createPrivateKey(readFileSync(`${deviceDir}/${user}.pem`)) };
}
/** `user`'s Zone logging in at `targetZone` for `user`, as CrossZoneClient does. */
async function crossZoneSession(user, targetZone) {
  const dev = device(user);
  if (!dev) return null;
  const now = Math.floor(Date.now() / 1000);
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const head = encode({ alg: 'EdDSA', typ: 'JWT', kid: `${dev.did}#main_key` });
  const body = encode({
    iss: zones[user].did, sub: zones[user].did, dev: dev.did, aud: zones[targetZone].zoneDid,
    target: 'system:msg-center', jti: randomUUID(), iat: now, exp: now + 60,
  });
  const signature = sign(null, Buffer.from(`${head}.${body}`), dev.key).toString('base64url');
  const pair = await rpc(targetZone, 'verify-hub', 'login_by_cross_zone_assertion', { assertion: `${head}.${body}.${signature}`, scopes: [] });
  if (!pair.session_token) throw new Error(`${user} at ${targetZone}: no session`);
  return pair.session_token;
}
const ticket = (zone, msgId, objId, token) => rpcRaw(zone, 'msg-center', 'msg.attachment_ticket', { msg_id: msgId, obj_id: objId }, token);
const ndnPath = (id, token) => `/ndn/${id}${token ? `?hint=${encodeURIComponent(JSON.stringify({ token }))}` : ''}`;

// ── the run ─────────────────────────────────────────────────────────────────

const tokens = { alice: await login('alice'), bob: await login('bob'), charlie: await login('charlie') };
for (const user of Object.keys(zones)) log('zone', { user, host: zones[user].host, address: await new Promise((ok, no) => lookup(zones[user].host, {}, (e, a) => e ? no(e) : ok(a))) });

// A file over one chunk goes through the typed walk (`File.content`, `ChunkList[i]`).
const small = await upload('alice', tokens.alice, 'note.txt', Buffer.from(`hello from alice ${randomBytes(8).toString('hex')}\n`.repeat(4096)));
const big = await upload('alice', tokens.alice, 'big.bin', randomBytes(CHUNK + 5 * 1024 * 1024));
log('uploaded', { small: small.id, big: big.id, big_size: big.bytes.length });

// 1. alice → bob: the sender's Zone is behind NAT, reached through the SN relay
const toBob = await send('alice', 'bob', [small, big], tokens.alice);
const arrived = await awaitMessage('bob', 'alice', toBob.text, tokens.bob);
const wire = JSON.stringify(arrived);
check(wire.includes(small.id) && wire.includes(big.id), 'the message names both attachments');
check(!wire.includes('bkt_') && !wire.includes('hint='), 'no token travels in the message');

const smallRead = await verifiedRead('bob', tokens.bob, small, toBob.msgId);
check(String(smallRead.headers['content-type']).startsWith('text/plain') && String(smallRead.headers['content-disposition']).startsWith('inline'), 'text stays inline', smallRead.headers);
const bigRead = await verifiedRead('bob', tokens.bob, big, toBob.msgId);
check(bigRead.headers['content-type'] === 'application/octet-stream' && String(bigRead.headers['content-disposition']).startsWith('attachment'), 'an unknown type downloads', bigRead.headers);

// 2. who else can get at it, at the sender's Zone
const unrelated = new ndn.FileObject('nothing', 1, small.id).genObjId()[0].toString();
check((await http('alice', 'GET', ndnPath(small.id))).status === 404, 'anonymous /ndn/ at the sender: 404');
check((await http('alice', 'GET', ndnPath(small.id, `bkt_${randomBytes(16).toString('hex')}`))).status === 404, 'a made-up token at the sender: 404');
const anonymous = await ticket('alice', toBob.msgId, small.id, null);
check(anonymous.error && !anonymous.result, 'no session, no ticket', anonymous);
const senderAsked = await ticket('alice', toBob.msgId, small.id, tokens.alice);
check(senderAsked.error && !senderAsked.result, 'the sender herself is no recipient: no ticket', senderAsked);

const bobAtAlice = await crossZoneSession('bob', 'alice');
const charlieAtAlice = await crossZoneSession('charlie', 'alice');
if (!bobAtAlice || !charlieAtAlice) {
  notVerified.push('non-recipient and recipient asking across Zones (BUCKYOS_TEST_DEVICE_DIR not given)');
} else {
  const issued = await ticket('alice', toBob.msgId, small.id, bobAtAlice);
  check(issued.result?.token?.startsWith('bkt_'), 'the recipient, asking across Zones, gets a ticket', issued.error ?? 'no token');
  const left = issued.result.expires_at - Date.now();
  // One hour, give or take the clock difference between this machine and the Zone.
  check(left > 0 && left <= 3660 * 1000, 'the ticket is short-lived', left);
  const viaTicket = await http('alice', 'GET', ndnPath(small.id, issued.result.token));
  check(viaTicket.status === 200 && hex(viaTicket.bytes) === small.sha, 'the ticket reads that attachment at /ndn/', `HTTP ${viaTicket.status}`);
  const otherFile = await http('alice', 'GET', ndnPath(big.id, issued.result.token));
  check(otherFile.status === 404, 'and no other attachment of the same message', `HTTP ${otherFile.status}`);
  const bigTicket = await ticket('alice', toBob.msgId, big.id, bobAtAlice);
  const from = CHUNK - 100;
  const across = await http('alice', 'GET', ndnPath(big.id, bigTicket.result?.token), { headers: { Range: `bytes=${from}-${from + 199}` } });
  check(across.status === 206 && across.bytes.equals(big.bytes.subarray(from, from + 200)), '/ndn/ answers a Range across the chunk boundary', `HTTP ${across.status}`);

  const charlieAsked = await ticket('alice', toBob.msgId, small.id, charlieAtAlice);
  check(charlieAsked.error && !charlieAsked.result, 'a non-recipient, asking across Zones, gets no ticket', charlieAsked.result ? 'a token was issued' : charlieAsked);
  const charlieOther = await ticket('alice', toBob.msgId, big.id, charlieAtAlice);
  check(charlieOther.error && !charlieOther.result, 'nor for the other attachment');
  const noSuch = await ticket('alice', unrelated, small.id, charlieAtAlice);
  check(noSuch.error && JSON.stringify(noSuch.error) === JSON.stringify(charlieAsked.error), 'the refusal does not tell that the message exists', { asked: charlieAsked.error, none: noSuch.error });
  const bobOther = await ticket('alice', toBob.msgId, unrelated, bobAtAlice);
  check(bobOther.error && !bobOther.result, 'a recipient gets no ticket for what the message does not reference');
  log('denials', { non_recipient: charlieAsked.error, sender: senderAsked.error, anonymous: anonymous.error });
}

// 3. who else can get at it, at the recipient's Zone
const charlieAtHome = await readAttachment('charlie', tokens.charlie, small.id, toBob.msgId);
check(charlieAtHome.status === 404, 'charlie\'s own Zone has no such message', `HTTP ${charlieAtHome.status}`);
const charlieAtBob = await crossZoneSession('charlie', 'bob');
if (charlieAtBob) {
  const r = await readAttachment('bob', charlieAtBob, small.id, toBob.msgId);
  check(r.status !== 200 && !r.bytes.includes(small.bytes.subarray(0, 32)), 'charlie cannot read it from bob\'s Zone', `HTTP ${r.status}`);
} else {
  notVerified.push('non-recipient reading at the recipient\'s Zone (BUCKYOS_TEST_DEVICE_DIR not given)');
}
check((await http('bob', 'GET', ndnPath(small.id))).status === 404, 'pulled does not mean public: anonymous /ndn/ at the recipient: 404');

// 4. the same file to another recipient is another message and another key
const toCharlie = await send('alice', 'charlie', [small], tokens.alice);
await awaitMessage('charlie', 'alice', toCharlie.text, tokens.charlie);
await verifiedRead('charlie', tokens.charlie, small, toCharlie.msgId);
if (charlieAtAlice) {
  const own = await ticket('alice', toCharlie.msgId, small.id, charlieAtAlice);
  check(own.result?.token?.startsWith('bkt_'), 'charlie gets a ticket for the message sent to him', own.error ?? 'no token');
  const stillNot = await ticket('alice', toBob.msgId, small.id, charlieAtAlice);
  check(stillNot.error && !stillNot.result, 'and still none for the message sent to bob');
}

// 5. the other way: a Zone behind NAT pulls from one reached directly
const backFile = await upload('bob', tokens.bob, 'reply.txt', Buffer.from(`hello from bob ${randomBytes(8).toString('hex')}\n`.repeat(1024)));
const toAlice = await send('bob', 'alice', [backFile], tokens.bob);
await awaitMessage('alice', 'bob', toAlice.text, tokens.alice);
await verifiedRead('alice', tokens.alice, backFile, toAlice.msgId);

// 6. more than a read can hand over (64 MiB, Desktop's limit too): refused without fetching it
const huge = await upload('bob', tokens.bob, 'huge.bin', randomBytes(MAX_READ_BYTES + 1024));
const hugeMsg = await send('bob', 'alice', [huge], tokens.bob);
await awaitMessage('alice', 'bob', hugeMsg.text, tokens.alice);
const refused = await readAttachment('alice', tokens.alice, huge.id, hugeMsg.msgId);
check(refused.status === 413, 'an attachment over the read limit: 413, not "try again"', `HTTP ${refused.status}`);
const refusedAgain = await readAttachment('alice', tokens.alice, huge.id, hugeMsg.msgId);
check(refusedAgain.status === 413, 'and the same on the next read', `HTTP ${refusedAgain.status}`);
log('not_fetched', { file: huge.id, note: 'the sender\'s gateway log must show this object and no chunk of it' });

console.log(`${passed} checks passed`);
if (notVerified.length) {
  for (const item of notVerified) console.log(`NOT VERIFIED: ${item}`);
  process.exit(2);
}
console.log('Remote attachment DV: all checks passed');
