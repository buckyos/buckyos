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
async function rpc(user, service, method, params, token = null) {
  const zone = zones[user];
  const body = JSON.stringify({ method, params, sys: token ? [++seq, token] : [++seq] });
  const result = await new Promise((resolve, reject) => {
    const req = request(`https://${zone.host}/kapi/${service}`, {
      method: 'POST', ca, timeout: 30000,
      lookup,
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) },
    }, res => {
      let content = '';
      res.on('data', chunk => content += chunk);
      res.on('end', () => res.statusCode === 200 ? resolve(JSON.parse(content)) : reject(new Error(`${user} ${method}: HTTP ${res.statusCode} ${content.slice(0, 200)}`)));
    });
    req.on('timeout', () => req.destroy(new Error(`${user} ${method}: timeout`)));
    req.on('error', reject);
    req.end(body);
  });
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
const bobToken = await login('bob');
for (const input of ['did:bns:alice', 'did:bns:charlie']) {
  const result = await rpc('bob', 'msg-center', 'contact.lookup', { input }, bobToken);
  console.log(JSON.stringify({ step: 'lookup', input, candidates: result.candidates, error: result.error }));
  {
    const candidate = result.candidates?.find(c => c.did === input);
    const expected = input.endsWith('alice') ? zones.alice.host : (process.env.BUCKYOS_TEST_ZONE_C?.trim() || 'charlie.me');
    if (candidate?.reachability?.state !== 'routable' || candidate.reachability.zone !== expected) {
      throw new Error(`${input}: expected routable at ${expected}`);
    }
  }
}
{
  const aliceToken = await login('alice');
  const stamp = Date.now();
  const content = `multi-zone F-4 native-route acceptance ${stamp}`;
  const sent = await rpc('bob', 'msg-center', 'msg.post_send', {
    msg: { from: zones.bob.did, to: [zones.alice.did], kind: 'chat', created_at_ms: stamp, nonce: stamp,
      content: { format: 'text/plain', content } },
    idempotency_key: `native-route-dv-${stamp}`,
  }, bobToken);
  console.log(JSON.stringify({ step: 'send', result: sent }));
  if (!sent.ok) throw new Error(`message not accepted: ${sent.reason}`);
  for (let attempt = 0; attempt < 45; attempt++) {
    const page = await rpc('alice', 'msg-center', 'msg.list_session', {
      owner: zones.alice.did, session_id: `dm:${zones.bob.did}`, limit: 50, descending: true, with_object: true,
    }, aliceToken);
    const found = page.items?.find(item => item.msg?.content?.content === content);
    if (found) {
      console.log(JSON.stringify({ step: 'received', box_kind: found.box_kind, direction: found.direction, content }));
      process.exit(0);
    }
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
  throw new Error('native message not received in 90 seconds');
}
