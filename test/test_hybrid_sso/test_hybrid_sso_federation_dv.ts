/**
 * Three-Zone DID federation acceptance (notepads/hybrid-sso-implementation-todo.md SSO-09 / SSO-14)
 * against the `ssotest` VM group (src/dev_configs/ssotest.json): canonical Zone A (alice), target
 * Zones B (bob) and C (charlie), all resolved through the SN like the VMs themselves.
 *
 *   BUCKYOS_TEST_DNS=<sn ip> node test/test_hybrid_sso/test_hybrid_sso_federation_dv.ts
 *
 * Runs with Node (type stripping) so DNS can be pointed at the SN without touching the host's
 * resolver; TLS trusts the dev CA. Raw kRPC so the `code` / `data` denial fields are read off the wire.
 *
 * Env: BUCKYOS_TEST_DNS (SN IP, required), BUCKYOS_TEST_HOST_MAP (host=ip pairs that bypass the SN
 * DNS), BUCKYOS_TEST_CA (dev CA PEM, ~/buckycli/ca/...),
 * BUCKYOS_TEST_PASSWORD (owner password of the three Zones, bucky2025), HYBRID_SSO_STEPS (subset),
 * HYBRID_SSO_CANONICAL_STOP / HYBRID_SSO_CANONICAL_START (shell commands that stop / start Zone A;
 * the canonical-down step is skipped without them).
 */
import { execSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
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
const onlySteps = (process.env.HYBRID_SSO_STEPS ?? "").split(",").map((s) => s.trim()).filter(Boolean);
const stopCanonical = process.env.HYBRID_SSO_CANONICAL_STOP?.trim() ?? "";
const startCanonical = process.env.HYBRID_SSO_CANONICAL_START?.trim() ?? "";
const runId = Date.now().toString(36);
let seq = Date.now();

const resolver = new Resolver();
resolver.setServers([dnsServer]);
// BUCKYOS_TEST_HOST_MAP="charlie.me=192.168.1.113,…" explicitly pins a direct or relay path.
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
    request.on("timeout", () => request.destroy(new Error(`${url}: timeout`)));
    request.on("error", reject);
    if (init.body) request.write(init.body);
    request.end();
  });
}

function sha256b64(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("base64");
}

/** websdk `hashPassword`: base64 sha256(password + username + ".buckyos"), then with the nonce. */
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
  if (response.status !== 200) throw new RpcFailure(`${zone.name} ${service}.${method}: HTTP ${response.status} ${response.body.slice(0, 200)}`);
  const body = JSON.parse(response.body) as Json;
  if (body.error) {
    throw new RpcFailure(`${zone.name} ${service}.${method}: ${body.error}`, body.code as string | undefined, body.data as Json | undefined);
  }
  const result = (body.result ?? {}) as Json;
  // AI Workspace returns business failures as results: `{ok:false, error:{code, denial?}}`.
  if (result.ok === false && result.error && typeof result.error === "object") {
    const error = result.error as Json;
    const denial = error.denial as Json | undefined;
    throw new RpcFailure(`${zone.name} ${service}.${method}: ${error.code}`, denial ? "access_denied" : String(error.code), denial);
  }
  return result;
}

async function expectDenied(label: string, reason: string, operation: () => Promise<unknown>): Promise<Json> {
  try {
    await operation();
  } catch (error) {
    assert(error instanceof RpcFailure, `${label}: unexpected ${error}`);
    assert(error.code === "access_denied", `${label}: code ${error.code} (${error.message})`);
    assert(error.data?.reason === reason, `${label}: reason ${JSON.stringify(error.data)} != ${reason}`);
    console.log(`    ✓ ${label} → ${reason}`);
    return error.data!;
  }
  throw new Error(`${label} should have been denied`);
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

/** A short-lived sudo (su_) session for admin methods that write with the caller's token. */
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

/** `auth.did.begin` at the target Zone: the canonical consent URL it sends the browser to. */
async function begin(target: Zone, did: string): Promise<Federation> {
  const begun = await rpc(target, "control-panel", "auth.did.begin", { did, redirect_url: `https://${target.host}/` });
  assert(begun.status === "redirect", `${target.name} did.begin ${JSON.stringify(begun)}`);
  const url = new URL(String(begun.url));
  assert(url.pathname === "/sso/federate", `federate path ${url}`);
  return { params: Object.fromEntries(url.searchParams.entries()), canonicalHost: url.host };
}

/** Approve at the canonical Zone; returns the callback URL on the target Zone (not yet followed). */
async function approve(canonical: Zone, federation: Federation, token = canonical.token): Promise<string> {
  const approved = await rpc(canonical, "control-panel", "auth.federation.approve", { ...federation.params, remember: true }, token);
  assert(approved.status === "redirect", `${canonical.name} approve ${JSON.stringify(approved)}`);
  return String(approved.url);
}

/** Follow a DID callback like a browser that holds no session there. */
async function callback(url: string): Promise<HttpResult> {
  const response = await http(url);
  assert(response.status === 302, `callback ${url} → HTTP ${response.status}: ${response.body.slice(0, 200)}`);
  return response;
}

function didError(response: HttpResult): string {
  return new URL(response.location, "https://x/").searchParams.get("did_error") ?? "";
}

async function ssoRefresh(zone: Zone, refreshCookie: string): Promise<HttpResult> {
  return await http(`https://${zone.host}/sso_refresh`, {
    method: "POST",
    headers: { Cookie: `buckyos_refresh_token=${refreshCookie}` },
  });
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

const steps: Array<[string, () => Promise<void>]> = [];
const step = (name: string, run: () => Promise<void>) => steps.push([name, run]);
const state: Json = {};

step("zones", async () => {
  for (const z of [A, B, C]) {
    const config = await rpc(z, "control-panel", "auth.sso_config", {});
    assert(config.did_login_enabled === true, `${z.name} did_login_enabled ${JSON.stringify(config)}`);
    z.token = await login(z, z.owner, password);
    const identity = await rpc(z, "control-panel", "auth.current_identity", {}, z.token);
    assert(identity.kind === "account" && identity.did === z.ownerDid, `${z.name} owner identity ${JSON.stringify(identity)}`);
    console.log(`    ${z.name} ${z.host} → ${resolved.get(z.host)}, owner ${identity.did}`);
  }
});

step("consent-a-b", async () => {
  const federation = await begin(B, A.ownerDid);
  assert(federation.canonicalHost === A.host, `canonical host ${federation.canonicalHost} != ${A.host}`);
  assert(federation.params.client_zone === B.zoneDid, `client_zone ${federation.params.client_zone}`);
  assert(federation.params.redirect_uri === `https://${B.host}/sso/did_callback`, `redirect_uri ${federation.params.redirect_uri}`);
  const consent = await rpc(A, "control-panel", "auth.federation.describe", federation.params, A.token);
  assert(consent.status === "consent", `describe ${JSON.stringify(consent)}`);
  console.log(`    consent at A: ${JSON.stringify(consent)}`);
  state.firstGrantAB = consent.already_granted;
  const response = await callback(await approve(A, federation));
  assert(response.cookies.buckyos_session_token && response.cookies.buckyos_refresh_token, `no cookies; → ${response.location}`);
  const identity = await rpc(B, "control-panel", "auth.current_identity", {}, response.cookies.buckyos_session_token);
  assert(
    identity.kind === "external" && identity.did === A.ownerDid && identity.auth_method === "did_federation" && identity.restricted === true,
    `identity at B ${JSON.stringify(identity)}`,
  );
  state.extB = response.cookies.buckyos_session_token;
  state.extBRefresh = response.cookies.buckyos_refresh_token;
  console.log(`    ✓ alice signed in to B as External ${identity.did}, back to ${response.location}`);
});

step("revisit-a-b", async () => {
  const federation = await begin(B, A.ownerDid);
  const consent = await rpc(A, "control-panel", "auth.federation.describe", federation.params, A.token);
  assert(consent.status === "consent" && consent.already_granted === true, `revisit ${JSON.stringify(consent)}`);
  const url = await approve(A, federation);
  const response = await callback(url);
  assert(response.cookies.buckyos_session_token, `revisit callback → ${response.location}`);
  state.usedCallback = url;
  console.log("    ✓ the remembered consent shows as granted, a second login works");
});

step("consent-a-c", async () => {
  const federation = await begin(C, A.ownerDid);
  assert(federation.params.client_zone === C.zoneDid && federation.params.redirect_uri === `https://${C.host}/sso/did_callback`, `C params ${JSON.stringify(federation.params)}`);
  const consent = await rpc(A, "control-panel", "auth.federation.describe", federation.params, A.token);
  // On a fresh Zone A the B consent above was the only one, so C must still ask; later runs see both remembered.
  assert(consent.status === "consent" && (state.firstGrantAB !== false || consent.already_granted === false), `consent for C is separate: ${JSON.stringify(consent)}`);
  const response = await callback(await approve(A, federation));
  assert(response.cookies.buckyos_session_token, `C callback → ${response.location}`);
  const identity = await rpc(C, "control-panel", "auth.current_identity", {}, response.cookies.buckyos_session_token);
  assert(identity.kind === "external" && identity.did === A.ownerDid, `identity at C ${JSON.stringify(identity)}`);
  console.log("    ✓ C asks for its own consent and signs alice in as External");
});

step("cancel", async () => {
  const federation = await begin(B, A.ownerDid);
  const denied = await rpc(A, "control-panel", "auth.federation.deny", federation.params, A.token);
  assert(denied.status === "redirect", `deny ${JSON.stringify(denied)}`);
  const url = new URL(String(denied.url));
  assert(url.searchParams.get("error") === "access_denied" && url.host === B.host, `deny url ${url}`);
  const response = await callback(url.toString());
  assert(didError(response) === "access_denied" && !response.cookies.buckyos_session_token, `cancel callback → ${response.location}`);
  console.log("    ✓ declining at A returns to B's sign-in page with access_denied, no cookie");
});

step("replay", async () => {
  const url = state.usedCallback as string | undefined;
  assert(url, "revisit-a-b must run first");
  const response = await callback(url);
  assert(didError(response) === "authorization_unavailable" && !response.cookies.buckyos_session_token, `replayed callback → ${response.location}`);
  const code = new URL(url).searchParams.get("code") ?? "";
  const exchanged = await rpc(A, "verify-hub", "exchange_federation_code", {
    code,
    code_verifier: randomBytes(32).toString("base64url"),
    client_zone: B.zoneDid,
    redirect_uri: `https://${B.host}/sso/did_callback`,
  });
  assert(exchanged.status === "invalid_code", `used code at A ${JSON.stringify(exchanged)}`);
  console.log("    ✓ a used callback and a used code are refused");
});

step("forged-source", async () => {
  // B is canonical only for bob: it must neither vouch for alice nor have its code accepted for alice.
  const forAlice = await begin(C, A.ownerDid);
  const atB = await rpc(B, "control-panel", "auth.federation.describe", forAlice.params, B.token);
  const refusal = atB.denial as Json | undefined;
  assert(atB.status === "denied" && refusal?.reason === "session_ceiling_exceeded" && refusal.detail === "did_mismatch", `B describing alice's request ${JSON.stringify(atB)}`);
  console.log("    ✓ B refuses to vouch for alice → session_ceiling_exceeded/did_mismatch");

  const forBob = await begin(C, B.ownerDid);
  assert(forBob.canonicalHost === B.host, `bob's canonical ${forBob.canonicalHost}`);
  const bobCallback = new URL(await approve(B, forBob));
  const swapped = new URL(bobCallback);
  swapped.searchParams.set("state", forAlice.params.state);
  const forged = await callback(swapped.toString());
  // C exchanges the code only with alice's canonical Zone A, which never issued it.
  assert(didError(forged) === "credential_state" && !forged.cookies.buckyos_session_token, `forged → ${forged.location}`);
  console.log("    ✓ bob's code presented as alice's login at C → credential_state, no cookie");

  const genuine = await callback(bobCallback.toString());
  assert(genuine.cookies.buckyos_session_token, `bob at C → ${genuine.location}`);
  const identity = await rpc(C, "control-panel", "auth.current_identity", {}, genuine.cookies.buckyos_session_token);
  assert(identity.kind === "external" && identity.did === B.ownerDid, `bob at C ${JSON.stringify(identity)}`);
  console.log("    ✓ the same code used for bob's own pending login at C signs bob in (B as canonical)");
});

step("bound-canonical", async () => {
  // did:bns:charlie is no Zone: its OwnerDocument binds did:web:charlie.me, which vouches for it.
  const federation = await begin(B, C.ownerDid);
  assert(federation.canonicalHost === C.host, `charlie's canonical ${federation.canonicalHost}`);
  const response = await callback(await approve(C, federation));
  assert(response.cookies.buckyos_session_token, `charlie at B → ${response.location}`);
  const identity = await rpc(B, "control-panel", "auth.current_identity", {}, response.cookies.buckyos_session_token);
  assert(identity.kind === "external" && identity.did === C.ownerDid, `charlie at B ${JSON.stringify(identity)}`);
  console.log(`    ✓ charlie signs in to B through its bound Zone ${C.zoneDid}`);
});

step("external-gate", async () => {
  const token = state.extB as string;
  assert(token, "consent-a-b must run first");
  await expectDenied("B control-panel user.list", "session_kind_not_allowed", () => rpc(B, "control-panel", "user.list", {}, token));
  await expectDenied("B task-manager list_tasks", "session_kind_not_allowed", () => rpc(B, "task-manager", "list_tasks", {}, token));
});

step("entries-at-b", async () => {
  const token = state.extB as string;
  assert(token, "consent-a-b must run first");
  const ws = await rpc(B, "aiworkspace", "ws.create", { title: `联合入口 ${runId}` }, B.token);
  const workspaceId = String(ws.workspace_id);
  const entry = async (didLogin: boolean) => {
    const created = await rpc(B, "aiworkspace", "ws.create_entry", {
      workspace_id: workspaceId,
      capabilities: ["read", "comment"],
      methods: { guest: false, password: true, self_register: false, did_login: didLogin },
    }, B.token);
    return String((created.entry as Json).entry_id);
  };
  const passwordOnly = await entry(false);
  await expectDenied("AIWS join of a password-only entry", "entry_unavailable", () => rpc(B, "aiworkspace", "ws.join_entry", { entry_id: passwordOnly }, token));
  const didEntry = await entry(true);
  const joined = await rpc(B, "aiworkspace", "ws.join_entry", { entry_id: didEntry }, token);
  assert(joined.granted === true && joined.path === `/workspace/${workspaceId}`, `join ${JSON.stringify(joined)}`);
  const info = await rpc(B, "aiworkspace", "ws.get_info", { workspace_id: workspaceId }, token);
  assert(info.principal === A.ownerDid, `ws.get_info principal ${info.principal}`);
  console.log(`    ✓ AIWS: alice joins the did_login entry and reads as ${info.principal}`);

  const group = async (didLogin: boolean) => {
    const created = await rpc(B, "msg-center", "group.create", {
      idempotency_key: `fed-${didLogin}-${runId}`,
      configuration: {
        profile: { name: `联合客服 ${runId}` },
        membership: { guest_entry: { session_template: "tickets", max_open_per_guest: 2, auth_methods: { password: true, self_register: false, did_login: didLogin } } },
        session_templates: { tickets: { membership: "explicit", rules: { allow_guests: true } } },
      },
    }, B.token);
    return String(created.group_did);
  };
  const closedGroup = await group(false);
  await expectDenied("Message Hub guest request without did_login", "entry_unavailable", () =>
    rpc(B, "msg-center", "group.submit_guest_request", { group_did: closedGroup, request_id: `req-${runId}` }, token));
  const openGroup = await group(true);
  const described = await rpc(B, "msg-center", "group.describe_entry", { group_did: openGroup });
  assert(described.state === "active" && (described.methods as Json).did_login === true, `describe_entry ${JSON.stringify(described)}`);
  const request = await rpc(B, "msg-center", "group.submit_guest_request", { group_did: openGroup, request_id: `req-${runId}` }, token);
  assert(request.session_id, `guest request ${JSON.stringify(request)}`);
  console.log(`    ✓ Message Hub: alice opens guest Session ${request.session_id} through the did_login entry`);
});

step("mapped-account", async () => {
  // did:bns:bob onboarded at A as account `bobfed` (users/bobfed/doc names the DID): federation
  // maps to that account instead of an External identity, and refuses while it is suspended.
  const userId = "bobfed";
  const su = await sudo(A);
  const existing = await rpc(A, "control-panel", "user.get", { user_id: userId }, su).catch(() => null);
  if (!existing) {
    const invite = await rpc(A, "control-panel", "user.invite.create", { target_did: B.ownerDid, user_id: userId, user_type: "user" }, su);
    const inviteId = String(invite.invite_id ?? (invite.invite as Json | undefined)?.invite_id);
    const owner = JSON.parse(readFileSync(`${homedir()}/buckycli/bob.bns.did/user_config.json`, "utf8")) as Json;
    const params = { invite_id: inviteId, owner_config: { ...owner, binded_zone_list: [A.zoneDid] } };
    const charlieLogin = await callback(await approve(C, await begin(A, C.ownerDid)));
    const charlieToken = charlieLogin.cookies.buckyos_session_token;
    assert(charlieToken, "Charlie must have a verified session for the mismatched-DID test");
    for (const [label, token] of [["anonymous", undefined], ["another DID", charlieToken]] as const) {
      let denied = false;
      try {
        await rpc(A, "control-panel", "user.invite.accept", params, token);
      } catch {
        denied = true;
      }
      assert(denied, `${label} must not accept Bob's invitation`);
    }
    const bobLogin = await callback(await approve(B, await begin(A, B.ownerDid)));
    const bobToken = bobLogin.cookies.buckyos_session_token;
    assert(bobToken, `Bob must first sign in with his DID: ${bobLogin.location}`);
    await rpc(A, "control-panel", "user.invite.accept", params, bobToken);
  }
  const login = async () => await callback(await approve(B, await begin(A, B.ownerDid)));
  const mapped = await login();
  assert(mapped.cookies.buckyos_session_token, `mapped login → ${mapped.location}`);
  const identity = await rpc(A, "control-panel", "auth.current_identity", {}, mapped.cookies.buckyos_session_token);
  assert(identity.kind === "account" && identity.user_id === userId && identity.auth_method === "did_federation", `mapped identity ${JSON.stringify(identity)}`);
  console.log(`    ✓ bob signs in to A as its local account ${identity.user_id}`);
  await rpc(A, "control-panel", "user.change_state", { user_id: userId, state: "suspended:dv" }, su);
  try {
    const refused = await login();
    assert(didError(refused) === "account_state" && !refused.cookies.buckyos_session_token, `suspended → ${refused.location}`);
    console.log("    ✓ suspended account → account_state, no External fallback");
  } finally {
    await rpc(A, "control-panel", "user.change_state", { user_id: userId, state: "active" }, su);
  }
});

step("refresh", async () => {
  const refreshed = await ssoRefresh(B, state.extBRefresh as string);
  assert(refreshed.status === 200 && refreshed.cookies.buckyos_refresh_token, `refresh at B → ${refreshed.status} ${refreshed.body.slice(0, 200)}`);
  state.extBRefresh = refreshed.cookies.buckyos_refresh_token;
  if (refreshed.cookies.buckyos_session_token) state.extB = refreshed.cookies.buckyos_session_token;
  console.log("    ✓ the External family refreshes at B");
});

step("zone-switch", async () => {
  await rpc(C, "control-panel", "sso.settings.set", { did_login_enabled: false }, C.token);
  try {
    const begun = await rpc(C, "control-panel", "auth.did.begin", { did: A.ownerDid, redirect_url: `https://${C.host}/` });
    assert(begun.status === "denied" && (begun.denial as Json).reason === "zone_method_disabled", `begin when closed ${JSON.stringify(begun)}`);
    console.log("    ✓ C closes DID login: begin → zone_method_disabled");
  } finally {
    await rpc(C, "control-panel", "sso.settings.set", { did_login_enabled: true }, C.token);
  }
});

step("canonical-down", async () => {
  if (!stopCanonical || !startCanonical) {
    console.log("    (HYBRID_SSO_CANONICAL_STOP/START unset; skipped)");
    return;
  }
  const pending = await approve(A, await begin(B, A.ownerDid));
  execSync(stopCanonical, { stdio: "inherit" });
  try {
    const response = await callback(pending);
    assert(didError(response) === "canonical_unavailable" && !response.cookies.buckyos_session_token, `canonical down → ${response.location}`);
    console.log("    ✓ new DID login at B while A is down → canonical_unavailable");
    const identity = await rpc(B, "control-panel", "auth.current_identity", {}, state.extB as string);
    assert(identity.kind === "external", `existing session ${JSON.stringify(identity)}`);
    const refreshed = await ssoRefresh(B, state.extBRefresh as string);
    assert(refreshed.status === 200, `refresh while A is down → ${refreshed.status} ${refreshed.body.slice(0, 200)}`);
    state.extBRefresh = refreshed.cookies.buckyos_refresh_token ?? state.extBRefresh;
    console.log("    ✓ the existing External session and its refresh keep working");
  } finally {
    execSync(startCanonical, { stdio: "inherit" });
    await waitFor("Zone A", async () => (await rpc(A, "control-panel", "auth.sso_config", {})).did_login_enabled === true, 300_000);
  }
});

let failed = 0;
for (const [name, run] of steps) {
  if (onlySteps.length && !onlySteps.includes(name) && name !== "zones") continue;
  console.log(`- ${name}`);
  try {
    await run();
  } catch (error) {
    failed += 1;
    console.log(`  ✗ ${name}: ${error instanceof RpcFailure ? `${error.message} code=${error.code} data=${JSON.stringify(error.data)}` : error}`);
  }
}
console.log(failed ? `${failed} step(s) failed` : "all steps passed");
process.exit(failed ? 1 : 0);
