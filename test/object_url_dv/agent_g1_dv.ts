/**
 * G1 DV (notepads/agent-object-url-todo.md §5): the Agent runtime's own session observes its
 * owner's sessions read-only and is refused another owner's mailbox, through the msg-center
 * Provider and the kRPC it maps to.
 *
 * The session is obtained like the runtime does: a device-signed app login assertion through
 * verify-hub `login_by_jwt` for the app instance.
 *
 *   DEVICE_KEY_PEM=$BUCKYOS_ROOT/security/ood1.<zone>/authentication.private.pem \
 *   deno run --config ../deno.json --allow-net --allow-env --allow-read --unsafely-ignore-certificate-errors agent_g1_dv.ts
 *
 * (The container's own `BUCKYOS_APP_TOKEN` is single-use once its runtime logged in, so the
 * script signs a fresh assertion with the node's device key the same way node-daemon does.)
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), AGENT_INSTANCE (jarvis.test.buckyos.io@devtest),
 * BUCKYOS_TEST_ADMIN_USER (devtest), BUCKYOS_TEST_USER (bob).
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
const KRpc = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => { call(m: string, p: JsonRecord): Promise<unknown> };
const env = (n: string, d: string) => Deno.env.get(n)?.trim() || d;
const BASE = `https://${env("BUCKYOS_TEST_ZONE_HOST", "test.buckyos.io")}`;
const instance = env("AGENT_INSTANCE", "jarvis.test.buckyos.io@devtest");
const owner = `did:bns:${env("BUCKYOS_TEST_ADMIN_USER", "devtest")}`;
const other = `did:bns:${env("BUCKYOS_TEST_USER", "bob")}`;
const agentDid = `did:web:${instance.split("@")[0]}`;
/** A fresh device-signed LoginAssertion, as node-daemon gives the App container (`generate_service_login_assertion`). */
async function mintAssertion(pemPath: string, device: string): Promise<string> {
  const pem = await Deno.readTextFile(pemPath);
  const der = Uint8Array.from(atob(pem.replace(/-----[^-]+-----/g, "").replace(/\s+/g, "")), (c) => c.charCodeAt(0));
  const key = await crypto.subtle.importKey("pkcs8", der, { name: "Ed25519" }, false, ["sign"]);
  const b64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const text = (value: unknown) => b64(new TextEncoder().encode(JSON.stringify(value)));
  const now = Math.floor(Date.now() / 1000);
  const [appid, sub] = instance.split("@");
  const body = `${text({ alg: "EdDSA", kid: device })}.${text({ appid, exp: now + 1800, iss: device, jti: String(Math.floor(Math.random() * 1e15)), sub })}`;
  const signature = new Uint8Array(await crypto.subtle.sign("Ed25519", key, new TextEncoder().encode(body)));
  return `${body}.${b64(signature)}`;
}

const devicePem = Deno.env.get("DEVICE_KEY_PEM")?.trim();
const appToken = Deno.env.get("AGENT_APP_TOKEN")?.trim() || (devicePem ? await mintAssertion(devicePem, env("DEVICE_NAME", "ood1")) : "");
if (!appToken) throw new Error("AGENT_APP_TOKEN or DEVICE_KEY_PEM is required");

let seq = Date.now();
const pair = await new KRpc(`${BASE}/kapi/verify-hub`, null, seq++).call("login_by_jwt", {
  type: "jwt",
  jwt: appToken,
  target: { kind: "app", app_instance_id: instance },
}) as JsonRecord;
const token = String(pair.session_token);
const claims = JSON.parse(atob(token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/")));
console.log(`[auth] agent runtime session sub=${claims.sub} appid=${claims.appid}`);

let passed = 0;
const failures: string[] = [];
async function check(name: string, fn: () => Promise<void>) {
  try {
    await fn();
    passed++;
    console.log(`PASS ${name}`);
  } catch (e) {
    failures.push(`${name}: ${e instanceof Error ? e.message : e}`);
    console.log(`FAIL ${name}: ${e instanceof Error ? e.message : e}`);
  }
}
async function status(path: string): Promise<number> {
  const r = await fetch(`${BASE}${path}`, { headers: { authorization: `Bearer ${token}` } });
  await r.body?.cancel();
  return r.status;
}
const msg = (method: string, params: JsonRecord) => new KRpc(`${BASE}/kapi/msg-center`, token, seq++).call(method, params);
async function refused(fn: () => Promise<unknown>): Promise<string> {
  try {
    await fn();
  } catch (e) {
    return String(e);
  }
  throw new Error("expected a refusal");
}

await check("the agent reads its own mailbox", async () => {
  const s = await status(`/kapi/msg-center/obj/owners/${agentDid}/sessions`);
  if (s !== 200) throw new Error(`own sessions: ${s}`);
});
await check("the agent observes its owner's sessions (Provider and kRPC)", async () => {
  const s = await status(`/kapi/msg-center/obj/owners/${owner}/sessions`);
  if (s !== 200) throw new Error(`owner sessions: ${s}`);
  await msg("msg.list_sessions", { owner });
});
await check("the agent is refused another owner's sessions", async () => {
  const s = await status(`/kapi/msg-center/obj/owners/${other}/sessions`);
  if (s !== 403) throw new Error(`other owner sessions: ${s}`);
  const e = await refused(() => msg("msg.list_sessions", { owner: other }));
  console.log(`  kRPC: ${e.slice(0, 120)}`);
});
await check("observation is read-only: no session created as the owner", async () => {
  const e = await refused(() => msg("msg.create_session", { owner, peer_did: other, title: "agent must not write as owner" }));
  console.log(`  kRPC: ${e.slice(0, 120)}`);
});

console.log(`\n${passed} passed, ${failures.length} failed`);
for (const f of failures) console.log(`  - ${f}`);
if (failures.length) Deno.exit(1);
