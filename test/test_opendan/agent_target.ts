/**
 * The agent an OpenDAN acceptance script talks to. Every agent runs as its
 * own app (app id = AgentId, instance `<agent_id>@<owner>`), so the scripts
 * first find the agent through control-panel:
 *
 * - BUCKYOS_TEST_AGENT_CREATE=<name>: use the user's agent of that name,
 *   creating it first (`agent.create`, default template) when there is none;
 * - BUCKYOS_TEST_AGENT_ID=<agent_id>: that agent;
 * - otherwise the user's first ready agent.
 *
 * OPENDAN_URL overrides where its Loader is reached (default: the agent's app
 * host through the zone gateway, `https://<app_host_name>.<zone>/kapi/opendan`;
 * `debug_jarvis.sh` prints the local service port instead).
 *
 * Run on its own it prints the selected agent (and creates it when asked):
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors agent_target.ts [--create <name>]
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };
/** A control-panel kRPC call with the user's session token. */
export type ControlPanelCall = (method: string, params: JsonRecord) => Promise<unknown>;

export type AgentTarget = {
  agentId: string;
  agentDid: string;
  /** The owner's DID: the only sender the agent answers. */
  ownerDid: string;
  opendanUrl: string;
};

const CREATE_TIMEOUT_MS = Number(Deno.env.get("BUCKYOS_TEST_AGENT_CREATE_TIMEOUT_S") || "600") * 1000;

function record(value: unknown, label: string): JsonRecord {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object: ${JSON.stringify(value)}`);
  }
  return value as JsonRecord;
}

function state(entry: JsonRecord): string {
  return String((entry.install as JsonRecord | undefined)?.state ?? "");
}

async function waitReady(cp: ControlPanelCall, agentId: string): Promise<JsonRecord> {
  const deadline = Date.now() + CREATE_TIMEOUT_MS;
  let last = "";
  while (Date.now() < deadline) {
    const status = record(await cp("agent.create.status", { agent_id: agentId }), "agent.create.status");
    const now = `${status.state}/${status.step}`;
    if (now !== last) {
      console.log(`  agent ${agentId}: ${now}${status.runtime_progress ? ` ${JSON.stringify(status.runtime_progress)}` : ""}`);
      last = now;
    }
    if (status.state === "ready") return record(await cp("agent.get", { agent_id: agentId }), "agent.get");
    if (status.state === "failed" || status.state === "removed") {
      throw new Error(`agent ${agentId} is ${status.state}: ${JSON.stringify(status.last_error ?? null)}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 3000));
  }
  throw new Error(`agent ${agentId} is not ready after ${CREATE_TIMEOUT_MS / 1000}s (${last})`);
}

async function ensureAgent(cp: ControlPanelCall, owner: string, name: string): Promise<JsonRecord> {
  const agents = (record(await cp("agent.list", {}), "agent.list").agents as JsonRecord[]) ?? [];
  const existing = agents.find((a) => a.owner_user_id === owner && a.name === name);
  if (existing) return await waitReady(cp, String(existing.agent_id));
  const templates = (record(await cp("agent.list_templates", {}), "agent.list_templates").templates as JsonRecord[]) ?? [];
  const template = templates.find((t) => t.is_default) ?? templates[0];
  if (!template) throw new Error("control-panel lists no agent template");
  const created = record(await cp("agent.create", {
    idempotency_key: crypto.randomUUID(),
    name,
    profile: { display_name: name },
    role_supplement: "",
    allow_group: false,
    allow_other_users: false,
    template_id: template.template_id,
    template_auto_update: true,
  }), "agent.create");
  console.log(`  creating ${created.agent_id} from ${template.template_id}`);
  return await waitReady(cp, String(created.agent_id));
}

export async function resolveAgent(cp: ControlPanelCall, owner: string, zoneHost: string): Promise<AgentTarget> {
  const create = Deno.env.get("BUCKYOS_TEST_AGENT_CREATE")?.trim();
  const wanted = Deno.env.get("BUCKYOS_TEST_AGENT_ID")?.trim();
  let entry: JsonRecord;
  if (create) {
    entry = await ensureAgent(cp, owner, create);
  } else if (wanted) {
    entry = record(await cp("agent.get", { agent_id: wanted }), "agent.get");
  } else {
    const agents = (record(await cp("agent.list", {}), "agent.list").agents as JsonRecord[]) ?? [];
    const ready = agents.find((a) => a.owner_user_id === owner && state(a) === "ready");
    if (!ready) {
      const known = agents.map((a) => `${a.agent_id} (${a.owner_user_id}, ${state(a)})`).join(", ") || "none";
      throw new Error(
        `${owner} has no ready agent (known: ${known}); set BUCKYOS_TEST_AGENT_CREATE=<name> to create one or BUCKYOS_TEST_AGENT_ID=<agent_id>`,
      );
    }
    entry = ready;
  }
  if (state(entry) !== "ready") throw new Error(`agent ${entry.agent_id} is ${state(entry)}, not ready`);
  const host = (entry.runtime as JsonRecord | null)?.app_host_name;
  const opendanUrl = Deno.env.get("OPENDAN_URL")?.trim() ||
    (host ? `https://${host}.${zoneHost}/kapi/opendan` : "");
  if (!opendanUrl) throw new Error(`agent ${entry.agent_id} has no runtime app; set OPENDAN_URL`);
  return {
    agentId: String(entry.agent_id),
    agentDid: String(entry.agent_did),
    ownerDid: String(entry.owner_did),
    opendanUrl,
  };
}

if (import.meta.main) {
  const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => RpcClient;
  const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;
  const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
  const user = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
  const password = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
  const createAt = Deno.args.indexOf("--create");
  if (createAt >= 0) {
    const name = Deno.args[createAt + 1];
    if (!name) throw new Error("--create needs a name");
    Deno.env.set("BUCKYOS_TEST_AGENT_CREATE", name);
  }
  let seq = Date.now();
  const url = `https://${zoneHost}/kapi/control-panel`;
  const nonce = ++seq;
  const login = record(await new KRpcClient(url, null, nonce).call("auth.login", {
    username: user,
    password: hashPassword(user, password, nonce),
    appid: "control-panel",
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  }), "auth.login");
  const token = String(login.session_token);
  const target = await resolveAgent((method, params) => new KRpcClient(url, token, ++seq).call(method, params), user, zoneHost);
  console.log(JSON.stringify(target, null, 2));
}
