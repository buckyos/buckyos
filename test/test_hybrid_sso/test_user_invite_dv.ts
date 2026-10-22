import { createHash } from "node:crypto";
import { Resolver } from "node:dns";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import https from "node:https";
import type { LookupFunction } from "node:net";

type Json = Record<string, unknown>;

interface Zone {
  name: string;
  host: string;
  owner: string;
  ownerDid: string;
  zoneDid: string;
  token: string;
}

const dnsServer = process.env.BUCKYOS_TEST_DNS?.trim();
if (!dnsServer) throw new Error("BUCKYOS_TEST_DNS (the SN IP) is required");
const ca = readFileSync(process.env.BUCKYOS_TEST_CA?.trim() || `${homedir()}/buckycli/ca/buckyos_test_ca_ca_cert.pem`);
const password = process.env.BUCKYOS_TEST_PASSWORD?.trim() || "bucky2025";
const runId = Date.now().toString(36);
let seq = Date.now();

const resolver = new Resolver();
resolver.setServers([dnsServer]);
const resolved = new Map<string, string>(
  (process.env.BUCKYOS_TEST_HOST_MAP ?? "").split(",").map((pair) => pair.trim().split("=")).filter((pair) => pair.length === 2) as [string, string][],
);

const lookup: LookupFunction = (hostname, options, callback) => {
  const done = (address: string) =>
    options.all ? (callback as (e: null, a: { address: string; family: number }[]) => void)(null, [{ address, family: 4 }]) : callback(null, address, 4);
  const cached = resolved.get(hostname);
  if (cached) return done(cached);
  resolver.resolve4(hostname, (error, addresses) => {
    if (error || !addresses.length) return callback(error ?? new Error(`no A record for ${hostname}`), "", 4);
    resolved.set(hostname, addresses[0]);
    done(addresses[0]);
  });
};

class RpcFailure extends Error {
  code?: string;
  data?: Json;
  constructor(message: string, code?: string, data?: Json) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

interface HttpResult {
  status: number;
  location: string;
  cookies: Record<string, string>;
  body: string;
}

function http(url: string, init: { method?: string; headers?: Record<string, string>; body?: string } = {}): Promise<HttpResult> {
  return new Promise((resolve, reject) => {
    const request = https.request(url, { method: init.method ?? "GET", headers: init.headers, ca, lookup, timeout: 30_000 }, (response) => {
      const chunks: Buffer[] = [];
      response.on("data", (chunk: Buffer) => chunks.push(chunk));
      response.on("end", () => {
        const cookies: Record<string, string> = {};
        for (const line of response.headers["set-cookie"] ?? []) {
          const [pair] = line.split(";");
          const index = pair.indexOf("=");
          cookies[pair.slice(0, index)] = pair.slice(index + 1);
        }
        resolve({
          status: response.statusCode ?? 0,
          location: String(response.headers.location ?? ""),
          cookies,
          body: Buffer.concat(chunks).toString("utf8"),
        });
      });
    });
    request.on("timeout", () => request.destroy(new Error(`${new URL(url).host}: timeout`)));
    request.on("error", reject);
    if (init.body) request.write(init.body);
    request.end();
  });
}

function sha256b64(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("base64");
}

function passwordHash(username: string, plain: string, nonce?: number): string {
  const base = sha256b64(`${plain}${username}.buckyos`);
  return nonce === undefined ? base : sha256b64(`${base}${nonce}`);
}

async function rpc(zone: Zone, service: string, method: string, params: Json, token: string | null = null): Promise<Json> {
  seq = Math.max(Date.now(), seq + 1);
  const response = await http(`https://${zone.host}/kapi/${service}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ method, params, sys: token ? [seq, token] : [seq] }),
  });
  assert(response.status === 200, `${zone.name} ${service}.${method}: HTTP ${response.status}`);
  const body = JSON.parse(response.body) as Json;
  if (body.error) {
    throw new RpcFailure(`${zone.name} ${service}.${method}: ${body.error}`, body.code as string | undefined, body.data as Json | undefined);
  }
  const result = (body.result ?? {}) as Json;
  return result;
}

async function login(zone: Zone, username: string, plain: string): Promise<string> {
  const nonce = Date.now();
  const result = await rpc(zone, "control-panel", "auth.login", {
    username,
    password: passwordHash(username, plain, nonce),
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  });
  assert(typeof result.session_token === "string", `${zone.name}: login(${username}) returned no token`);
  return result.session_token;
}

async function sudo(zone: Zone): Promise<string> {
  const nonce = Date.now();
  const result = await rpc(zone, "verify-hub", "sudo_by_password", {
    username: zone.owner,
    password: passwordHash(zone.owner, password, nonce),
    target: { kind: "system", service_id: "control-panel" },
    aud: "system-config",
    login_nonce: nonce,
  });
  assert(typeof result.session_token === "string", `${zone.name}: sudo returned no token`);
  return result.session_token;
}

interface Federation {
  params: Record<string, string>;
  canonicalHost: string;
}

async function begin(target: Zone, did: string): Promise<Federation> {
  const begun = await rpc(target, "control-panel", "auth.did.begin", { did, redirect_url: `https://${target.host}/` });
  assert(begun.status === "redirect", `${target.name} did.begin ${JSON.stringify(begun)}`);
  const url = new URL(String(begun.url));
  assert(url.pathname === "/sso/federate", `unexpected federation path ${url.pathname}`);
  return { params: Object.fromEntries(url.searchParams.entries()), canonicalHost: url.host };
}

async function approve(canonical: Zone, federation: Federation, token = canonical.token): Promise<string> {
  const approved = await rpc(canonical, "control-panel", "auth.federation.approve", { ...federation.params, remember: true }, token);
  assert(approved.status === "redirect", `${canonical.name} approve ${JSON.stringify(approved)}`);
  return String(approved.url);
}

async function callback(url: string): Promise<HttpResult> {
  const response = await http(url);
  assert(response.status === 302, `federation callback: HTTP ${response.status}`);
  return response;
}

function didError(response: HttpResult): string {
  return new URL(response.location, "https://x/").searchParams.get("did_error") ?? "";
}

async function waitFor(label: string, check: () => Promise<boolean>, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check().catch(() => false)) return;
    await new Promise((resolve) => setTimeout(resolve, 3_000));
  }
  throw new Error(`${label}: not ready after ${timeoutMs / 1000}s`);
}

function zone(name: string, host: string, owner: string, zoneDid: string): Zone {
  return { name, host, owner, ownerDid: `did:bns:${owner}`, zoneDid, token: "" };
}

const A = zone("A", process.env.BUCKYOS_TEST_ZONE_A?.trim() || "alice.web3.devtests.org", "alice", "did:bns:alice");
const B = zone("B", process.env.BUCKYOS_TEST_ZONE_B?.trim() || "bob.web3.devtests.org", "bob", "did:bns:bob");
const C = zone("C", process.env.BUCKYOS_TEST_ZONE_C?.trim() || "charlie.me", "charlie", "did:web:charlie.me");
const userId = "multi_zone_charlie";
const stableInviteId = "multi-zone-charlie-f6";
const owner = JSON.parse(readFileSync(process.env.BUCKYOS_TEST_CHARLIE_CONFIG?.trim() || `${homedir()}/buckycli/charlie.me/user_config.json`, "utf8")) as Json;
assert(owner.id === C.ownerDid, "Charlie owner document has the wrong DID");
const ownerConfig = { ...owner, binded_zone_list: [A.zoneDid] };
for (const z of [A, B, C]) {
  await waitFor(`${z.name} control-panel`, async () => (await rpc(z, "control-panel", "auth.sso_config", {})).did_login_enabled === true, 120_000);
  z.token = await login(z, z.owner, password);
}
if (process.argv.includes("--preflight")) {
  console.log("Three Zone owner logins passed; invitation state unchanged");
  process.exit(0);
}
const su = await sudo(A);
async function federated(canonical: Zone): Promise<string> {
  const f = await begin(A, canonical.ownerDid);
  assert(f.canonicalHost === canonical.host, `${canonical.name}: wrong canonical host`);
  const result = await callback(await approve(canonical, f));
  const token = result.cookies.buckyos_session_token;
  assert(token, `${canonical.name}: federation failed ${didError(result)}`);
  const identity = await rpc(A, "control-panel", "auth.current_identity", {}, token);
  assert(identity.did === canonical.ownerDid && identity.auth_method === "did_federation", `${canonical.name}: wrong federated identity`);
  return token;
}
async function createInvite(inviteId: string, extra: Json): Promise<Json> {
  const existing = await optionalRecord("user.invite.get", { invite_id: inviteId });
  if (existing) {
    const invite = existing.invite as Json;
    assert(invite.target_did === extra.target_did && (invite.target_user_id ?? null) === (extra.user_id ?? null), `${inviteId}: retained invitation has different targets`);
    return existing;
  }
  return await rpc(A, "control-panel", "user.invite.create", { invite_id: inviteId, user_type: "user", ...extra }, su);
}
async function optionalRecord(method: string, params: Json): Promise<Json | null> {
  try {
    return await rpc(A, "control-panel", method, params, su);
  } catch (error) {
    if (error instanceof RpcFailure && /not found/i.test(error.message)) return null;
    throw error;
  }
}
async function pending(inviteId: string) {
  const result = await rpc(A, "control-panel", "user.invite.get", { invite_id: inviteId });
  assert((result.invite as Json).state === "pending", `${inviteId}: invitation was consumed`);
}
async function refuses(label: string, inviteId: string, token: string | null, expected: RegExp) {
  let failure = "";
  try {
    await rpc(A, "control-panel", "user.invite.accept", {
      invite_id: inviteId, owner_config: ownerConfig,
      password_hash: passwordHash(userId, password),
    }, token);
  } catch (error) {
    assert(error instanceof RpcFailure, `${label}: non-RPC failure`);
    failure = error.message;
  }
  assert(failure && expected.test(failure), `${label}: unexpected denial ${failure}`);
  await pending(inviteId);
  console.log(JSON.stringify({ step: label, denied: true, invite_still_pending: true }));
}
const existingUser = await optionalRecord("user.get", { user_id: userId });
const initial = await createInvite(stableInviteId, { target_did: C.ownerDid, user_id: userId });
const initialState = (initial.invite as Json).state;
assert(initialState === "pending" || (initialState === "accepted" && existingUser?.state === "active"), "unexpected retained invite/account state");
let denialInvite = stableInviteId;
if (initialState === "accepted") {
  denialInvite = `multi-zone-f6-denial-${runId}`;
  await createInvite(denialInvite, { target_did: C.ownerDid, ttl_secs: 3600 });
}
const bobToken = await federated(B);
await refuses("anonymous", denialInvite, null, /login|required|session|permission|token|authenticate/i);
await refuses("other-DID", denialInvite, bobToken, /DID federation session/i);
await refuses("password-session", denialInvite, A.token, /DID federation session/i);
const charlieToken = await federated(C);
if (initialState === "pending") {
  const accepted = await rpc(A, "control-panel", "user.invite.accept", {
    invite_id: stableInviteId, owner_config: ownerConfig, password_hash: passwordHash(userId, password),
  }, charlieToken);
  assert(accepted.ok === true, "federated invitation was not accepted");
}
const accepted = await rpc(A, "control-panel", "user.invite.get", { invite_id: stableInviteId });
assert((accepted.invite as Json).state === "accepted" && (accepted.invite as Json).accepted_user_id === userId, "invite not accepted for expected user");
const mappedToken = await federated(C);
const mapped = await rpc(A, "control-panel", "auth.current_identity", {}, mappedToken);
assert(mapped.kind === "account" && mapped.did === C.ownerDid && mapped.user_id === userId, `unexpected mapped identity ${JSON.stringify(mapped)}`);
console.log(JSON.stringify({ step: "federated-accept", invite: stableInviteId, user_id: userId, did: mapped.did, kind: mapped.kind }));
const exactPasswordProbe = `multi-zone-f6-password-${runId}`;
await createInvite(exactPasswordProbe, { target_did: C.ownerDid, ttl_secs: 3600 });
const passwordToken = await login(A, userId, password);
const passwordIdentity = await rpc(A, "control-panel", "auth.current_identity", {}, passwordToken);
assert(passwordIdentity.did === C.ownerDid && passwordIdentity.auth_method === "password", "same-DID password session not established");
await refuses("same-DID-password-session", exactPasswordProbe, passwordToken, /DID federation session/i);
const mismatchId = `multi-zone-f6-target-mismatch-${runId}`;
await createInvite(mismatchId, { target_did: B.ownerDid, ttl_secs: 3600 });
await refuses("target-DID-mismatch", mismatchId, mappedToken, /does not match owner_config/i);
const expiredId = `multi-zone-f6-expired-${runId}`;
await createInvite(expiredId, { target_did: C.ownerDid, expires_at: Math.floor(Date.now() / 1000) - 60 });
await refuses("expired-invite", expiredId, mappedToken, /expired/i);
console.log(JSON.stringify({ step: "complete", retained_account: userId, retained_main_invite: stableInviteId, additional_invites: [...(denialInvite === stableInviteId ? [] : [denialInvite]), exactPasswordProbe, mismatchId, expiredId] }));
