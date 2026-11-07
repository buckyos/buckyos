// What the cross-Zone attachment DVs share (remote_attachment_dv.mjs, group_attachment_dv.mjs):
// the ssotest Zones over HTTPS and kRPC, uploads as Desktop does them, reads of the object route,
// and asking another Zone as a Zone would. See the head of those files for how to run them.
import { request } from 'node:https';
import { execSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { createHash, createPrivateKey, randomUUID, sign } from 'node:crypto';
import { Resolver } from 'node:dns';
import { homedir } from 'node:os';
import { ndn } from 'buckyos';

const dnsServer = process.env.BUCKYOS_TEST_DNS?.trim();
if (!dnsServer) throw new Error('BUCKYOS_TEST_DNS (the SN IP) is required');
const ca = readFileSync(process.env.BUCKYOS_TEST_CA?.trim() || `${homedir()}/buckycli/ca/buckyos_test_ca_ca_cert.pem`);
const password = process.env.BUCKYOS_TEST_PASSWORD?.trim() || 'bucky2025';
const deviceDir = process.env.BUCKYOS_TEST_DEVICE_DIR?.trim();
export const CHUNK = 32 * 1024 * 1024;
export const MAX_READ_BYTES = 64 * 1024 * 1024;
const PULL_DEADLINE_MS = 10 * 60 * 1000;

const resolver = new Resolver();
resolver.setServers([dnsServer]);
const resolved = new Map((process.env.BUCKYOS_TEST_HOST_MAP ?? '').split(',')
  .map(pair => pair.trim().split('=')).filter(pair => pair.length === 2));
export function lookup(host, options, callback) {
  const done = address => options.all ? callback(null, [{ address, family: 4 }]) : callback(null, address, 4);
  if (resolved.has(host)) return done(resolved.get(host));
  resolver.resolve4(host, (error, addresses) => {
    if (error || !addresses.length) return callback(error ?? new Error(`no A record for ${host}`), '', 4);
    resolved.set(host, addresses[0]);
    done(addresses[0]);
  });
}
export const zones = {
  alice: { host: process.env.BUCKYOS_TEST_ZONE_A?.trim() || 'alice.web3.devtests.org', did: 'did:bns:alice', zoneDid: 'did:bns:alice' },
  bob: { host: process.env.BUCKYOS_TEST_ZONE_B?.trim() || 'bob.web3.devtests.org', did: 'did:bns:bob', zoneDid: 'did:bns:bob' },
  charlie: { host: process.env.BUCKYOS_TEST_ZONE_C?.trim() || 'charlie.me', did: 'did:bns:charlie', zoneDid: 'did:web:charlie.me' },
};

let seq = Date.now();
export function http(user, method, path, { body, headers = {}, timeout = 120000 } = {}) {
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
export async function rpcRaw(user, service, method, params, token = null) {
  seq = Math.max(Date.now(), seq + 1);
  const r = await http(user, 'POST', `/kapi/${service}`, { body: { method, params, sys: token ? [seq, token] : [seq] } });
  if (r.status !== 200) return { error: `HTTP ${r.status} ${r.text().slice(0, 200)}` };
  return JSON.parse(r.text());
}
export async function rpc(user, service, method, params, token = null) {
  const result = await rpcRaw(user, service, method, params, token);
  if (result.error) throw new Error(`${user} ${method}: ${JSON.stringify(result.error)}`);
  return result.result;
}
const sha = value => createHash('sha256').update(value).digest('base64');
export const hex = value => createHash('sha256').update(value).digest('hex');
const passwordProof = (username, nonce) => sha(sha(`${password}${username}.buckyos`) + nonce);
/** `username`'s session at the Zone of `zone`. */
export async function login(username, zone = username) {
  const nonce = Date.now();
  const result = await rpc(zone, 'control-panel', 'auth.login', {
    username, password: passwordProof(username, nonce), login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
  });
  if (!result.session_token) throw new Error(`${username}: login failed`);
  return result.session_token;
}
/** An ordinary second account in `zone`, kept between runs; returns its session and DID. */
export async function secondAccount(zone, username) {
  const nonce = Date.now();
  const sudo = await rpc(zone, 'verify-hub', 'sudo_by_password', {
    username: zone, password: passwordProof(zone, nonce), login_nonce: nonce, aud: 'system-config',
    target: { kind: 'system', service_id: 'control-panel' },
  });
  const created = await rpcRaw(zone, 'control-panel', 'user.create', { user_id: username, user_type: 'user', groups: [], password_hash: sha(`${password}${username}.buckyos`) }, sudo.session_token);
  if (created.error && !/name_conflict|exist/i.test(JSON.stringify(created.error))) throw new Error(`user.create: ${JSON.stringify(created.error)}`);
  const token = await login(username, zone);
  const identity = await rpc(zone, 'control-panel', 'auth.current_identity', {}, token);
  return { token, did: identity.did };
}
/** A message as a Zone that lies would send it: signed by `user`'s device, past that Zone's own checks. */
export async function pushSigned(user, targetZone, recipient, msg) {
  const dev = device(user);
  if (!dev) return null;
  const session = await crossZoneSession(user, targetZone);
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const head = encode({ alg: 'EdDSA', typ: 'JWT', kid: `${dev.did}#main_key` });
  const body = encode(msg);
  const jwt = `${head}.${body}.${sign(null, Buffer.from(`${head}.${body}`), dev.key).toString('base64url')}`;
  const sorted = value => Array.isArray(value) ? value.map(sorted)
    : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sorted(value[key])])) : value;
  const id = `cymsg:${createHash('sha256').update(JSON.stringify(sorted(msg))).digest('hex')}`;
  return http(targetZone, 'PUT', `/kapi/msg-center/${recipient}/inbox`, {
    body: Buffer.from(jwt),
    headers: { Authorization: `Bearer ${session}`, 'Content-Type': 'application/cyfs-named-object+jwt', 'cyfs-obj-id': id },
  });
}
export const sleep = ms => new Promise(r => setTimeout(r, ms));
export function log(step, detail = {}) {
  console.log(JSON.stringify({ step, ...detail }));
}
let passed = 0;
export const notVerified = [];
export function check(condition, label, detail) {
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
export async function upload(user, token, name, bytes, declaredSize = bytes.length) {
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
  const [fileObjId, fileBody] = new ndn.FileObject(name, declaredSize, content).genObjId();
  const id = fileObjId.toString();
  objects.push({ obj_id: id, obj_data: fileBody });
  const r = await http(user, 'POST', '/ndm/v1/import/finalize', { body: { objects, roots: [id] }, headers: { Authorization: `Bearer ${token}` } });
  if (r.status !== 200 || JSON.parse(r.text()).ready !== true) throw new Error(`${user} finalize: HTTP ${r.status} ${r.text().slice(0, 300)}`);
  return { id, name, bytes, sha: hex(bytes) };
}
export async function send(from, to, files, token) {
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
export async function awaitMessage(to, from, text, token) {
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

export function contentPath(id, msgId) {
  return `/kapi/msg-center/objects/${id}/content?context_path=${encodeURIComponent(msgId)}`;
}
export function readAttachment(user, token, id, msgId, headers = {}) {
  return http(user, 'GET', contentPath(id, msgId), { timeout: 300000, headers: { Authorization: `Bearer ${token}`, ...headers } });
}
/** The first read starts the pull: 503 + Retry-After until the bytes came and verified. */
export async function readUntilPulled(user, token, file, msgId) {
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
export async function verifiedRead(user, token, file, msgId) {
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

export function device(user) {
  if (!deviceDir || !existsSync(`${deviceDir}/${user}.pem`)) return null;
  return { did: readFileSync(`${deviceDir}/${user}.did`, 'utf8').trim(), key: createPrivateKey(readFileSync(`${deviceDir}/${user}.pem`)) };
}
/** The device of `user`'s Zone logging in at `targetZone` for `principal`, as CrossZoneClient does; the raw answer. */
export async function crossZoneLogin(user, targetZone, principal = zones[user].did) {
  const dev = device(user);
  if (!dev) return null;
  const now = Math.floor(Date.now() / 1000);
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const head = encode({ alg: 'EdDSA', typ: 'JWT', kid: `${dev.did}#main_key` });
  const body = encode({
    iss: principal, sub: principal, dev: dev.did, aud: zones[targetZone].zoneDid,
    target: 'system:msg-center', jti: randomUUID(), iat: now, exp: now + 60,
  });
  const signature = sign(null, Buffer.from(`${head}.${body}`), dev.key).toString('base64url');
  return rpcRaw(targetZone, 'verify-hub', 'login_by_cross_zone_assertion', { assertion: `${head}.${body}.${signature}`, scopes: [] });
}
export async function crossZoneSession(user, targetZone, principal = zones[user].did) {
  const answer = await crossZoneLogin(user, targetZone, principal);
  if (!answer) return null;
  if (!answer.result?.session_token) throw new Error(`${user} at ${targetZone}: no session: ${JSON.stringify(answer.error)}`);
  return answer.result.session_token;
}
export const ticket = (zone, msgId, objId, token) => rpcRaw(zone, 'msg-center', 'msg.attachment_ticket', { msg_id: msgId, obj_id: objId }, token);
export const memberTicket = (zone, contextPath, objId, token) => rpcRaw(zone, 'msg-center', 'msg.attachment_ticket', { context_path: contextPath, obj_id: objId }, token);
export const ndnPath = (id, token) => `/ndn/${id}${token ? `?hint=${encodeURIComponent(JSON.stringify({ token }))}` : ''}`;

/** The object route's description: nothing is pulled for it. */
export function describe(user, token, id, context) {
  return http(user, 'GET', `/kapi/msg-center/objects/${id}?context_path=${encodeURIComponent(context)}`, { timeout: 120000, headers: { Authorization: `Bearer ${token}` } });
}
export const describedRemotely = response => response.headers['x-attachment-remote'] === '1';
/** How `owner` treats messages from `peer`, whatever earlier runs left. */
export const setAccess = (owner, peer, level, token) =>
  rpc(owner, 'msg-center', 'contact.update_contact', { did: zones[peer].did, patch: { access_level: level }, contact_mgr_owner: zones[owner].did }, token);
/** A shell command of the environment (taking a Zone off the network, say), if given. */
export function runGiven(env, purpose) {
  const command = process.env[env]?.trim();
  if (!command) {
    notVerified.push(`${purpose} (${env} not given)`);
    return false;
  }
  execSync(command, { stdio: 'inherit' });
  return true;
}
export function finish(title) {
  console.log(`${passed} checks passed`);
  if (notVerified.length) {
    for (const item of notVerified) console.log(`NOT VERIFIED: ${item}`);
    process.exit(2);
  }
  console.log(`${title}: all checks passed`);
}
