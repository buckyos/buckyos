// Shared helpers of the sample: zone login, kRPC calls, and the Hook signature
// check (the same rule as buckyos-api `verify_group_hook_signature`).
import { buckyos } from "buckyos";

export type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;

export const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
export const appId = Deno.env.get("BUCKYOS_TEST_APP_ID")?.trim() || "crm";
export const panelEntryUrl = Deno.env.get("BUCKYOS_TEST_PANEL_URL")?.trim() || `https://${appId}.${zoneHost}/panel/`;

let lastNonce = Date.now();
export function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

export function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

export function asRecord(value: unknown, label: string): JsonRecord {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  return value as JsonRecord;
}

export function items(value: unknown, label: string): JsonRecord[] {
  return (asRecord(value, label).items as JsonRecord[]) ?? [];
}

export async function call(service: string, method: string, params: JsonRecord, token: string | null, origin = `https://${zoneHost}`): Promise<unknown> {
  const rpc = new KRpcClient(`${origin}/kapi/${service}`, token, nextNonce());
  return await rpc.call(method, params);
}

export async function login(username: string, password: string): Promise<string> {
  const nonce = nextNonce();
  const result = asRecord(
    await call("control-panel", "auth.login", {
      username,
      password: hashPassword(username, password, nonce),
      appid: "control-panel",
      target: { kind: "system", service_id: "control-panel" },
      login_nonce: nonce,
    }, null),
    `auth.login(${username})`,
  );
  assert(typeof result.session_token === "string" && result.session_token, `login(${username}) returned no token`);
  return result.session_token;
}

export async function expectReject(label: string, operation: () => Promise<unknown>): Promise<string> {
  try {
    await operation();
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    console.log(`  ✓ ${label}: ${reason}`);
    return reason;
  }
  throw new Error(`${label} should have been rejected`);
}

// ── Hook signature (D-3): hex HMAC-SHA256(secret, `${timestamp}.${body}`), ±300 s ──

const encoder = new TextEncoder();

export async function hookSignature(secret: string, timestamp: string, body: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = new Uint8Array(await crypto.subtle.sign("HMAC", key, encoder.encode(`${timestamp}.${body}`)));
  return Array.from(mac, (b) => b.toString(16).padStart(2, "0")).join("");
}

export async function verifyHookRequest(secret: string, req: Request): Promise<{ ok: boolean; body: string }> {
  const body = await req.text();
  const timestamp = req.headers.get("x-buckyos-hook-timestamp") ?? "";
  const signature = (req.headers.get("x-buckyos-hook-signature") ?? "").trim().toLowerCase();
  if (!/^\d+$/.test(timestamp) || Math.abs(Date.now() / 1000 - Number(timestamp)) > 300) return { ok: false, body };
  const expected = await hookSignature(secret, timestamp, body);
  let diff = expected.length ^ signature.length;
  for (let i = 0; i < expected.length; i++) diff |= expected.charCodeAt(i) ^ (signature.charCodeAt(i) || 0);
  return { ok: diff === 0, body };
}

/** The sample's "customer service group" configuration (L1). */
export function supportGroupConfig(hookUrl: string, ownerDid: string): JsonRecord {
  return {
    required_features: ["sessions", "session_guests", "hooks", "panels"],
    display: {
      announcement: "工作日 9:00–18:00 响应；紧急问题请在工单 Session 里 @值班客服。",
      links: [{ title: "帮助中心", url: `https://${appId}.${zoneHost}/help` }],
      featured_members: [ownerDid],
    },
    extensions: {
      panels: [
        { id: "tickets", title: "工单", entry_url: panelEntryUrl, scope: "group" },
        { id: "sla", title: "SLA", entry_url: `${panelEntryUrl}?view=sla`, scope: { sessions: ["tickets"] } },
      ],
    },
    session_templates: {
      support: {
        membership: { roles: ["owner", "admin"] },
        rules: {
          allow_guests: true,
          post: "all_participants",
          panels: ["tickets"],
          display: { announcement: "本 Session 由客服与客户共享。" },
          hooks: [
            { point: "join", service: hookUrl, on_unavailable: "native_only", timeout_ms: 2000 },
            { point: "post", service: hookUrl, may_grant: true, on_unavailable: "deny", timeout_ms: 2000 },
          ],
        },
      },
    },
    membership: { guest_entry: { session_template: "support", max_open_per_guest: 3, auth_methods: { password: true } } },
  };
}
