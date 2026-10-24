/**
 * L1: create the customer-service group once and exit. Writes `state.json`
 * (group DID, template Hook secrets) next to this file; the secrets appear only in
 * this response, so keep the file out of version control.
 *
 *   deno run --config ../../deno.json --allow-net --allow-env --allow-read --allow-write \
 *     --unsafely-ignore-certificate-errors setup_group.ts
 */
import { asRecord, assert, call, login, supportGroupConfig } from "./common.ts";

const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const hookUrl = Deno.env.get("BUCKYOS_TEST_HOOK_URL")?.trim() || "http://127.0.0.1:8787/hook";

const token = await login(adminUser, adminPassword);
const identity = asRecord(await call("control-panel", "auth.current_identity", {}, token), "auth.current_identity");
const ownerDid = String(identity.did);
const created = asRecord(
  await call("msg-center", "group.create", {
    idempotency_key: `group-ext-sample-${Date.now()}`,
    profile: { name: "客服群（样例）" },
    configuration: supportGroupConfig(hookUrl, ownerDid),
    sessions: [{ session_id: "tickets", template: "support" }],
  }, token),
  "group.create",
);
const hookSecrets = (created.hook_secrets as Array<{ scope: string; index: number; secret: string }>) ?? [];
assert(hookSecrets.length === 2, "the two template hooks got secrets");
const state = {
  zone_host: Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io",
  group_did: String(created.group_did),
  revision: String(created.revision),
  hook_url: hookUrl,
  hook_secrets: Object.fromEntries(hookSecrets.map((h) => [`${h.scope}#${h.index}`, h.secret])),
  hook_secret: hookSecrets.find((h) => h.index === 1)?.secret ?? hookSecrets[0].secret,
};
await Deno.writeTextFile(new URL("./state.json", import.meta.url), JSON.stringify(state, null, 2));
console.log(`created ${state.group_did}; hook secrets written to state.json (not shown again by the service)`);
