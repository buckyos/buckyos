/**
 * User types, system user groups and contact projection against a running Zone
 * (product/users_and_agents/UserType.md, notepads/user-type-definition-todo.md §5):
 * RBAC lines, what a Limited account can reach before and after joining a group,
 * the projected address books on both sides, read-only projected entries, and a
 * direct message to an account without a personal inbox. Everything it creates is
 * removed at the end (users are soft-deleted).
 *
 *   bash test/user_groups_dv.sh
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025).
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (
  url: string,
  token?: string | null,
  seq?: number,
) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (
  username: string,
  password: string,
  nonce?: number,
) => string;

const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const stamp = Date.now().toString(36);
const customerGroup = `utcust${stamp}`;
const serviceGroup = `utsvc${stamp}`;
const carol = `utcarol${stamp}`;
const sam = `utsam${stamp}`;
const password = `Ut-${stamp}-pass`;
const authTarget = { kind: "system", service_id: "control-panel" };
let lastNonce = Date.now();

function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

function asRecord(value: unknown, label: string): JsonRecord {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  return value as JsonRecord;
}

async function call(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  const rpc = new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce());
  return await rpc.call(method, params);
}

async function expectReject(label: string, operation: () => Promise<unknown>, expected?: string | string[]): Promise<void> {
  try {
    await operation();
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    const codes = expected === undefined ? [] : [expected].flat();
    assert(codes.length === 0 || codes.some((code) => reason.includes(code)), `${label}: expected "${codes.join(" | ")}", got "${reason}"`);
    console.log(`  ✓ ${label}: ${reason}`);
    return;
  }
  throw new Error(`${label} should have been rejected`);
}

async function login(username: string, secret: string): Promise<string> {
  const nonce = nextNonce();
  const result = asRecord(await call("control-panel", "auth.login", {
    username,
    password: hashPassword(username, secret, nonce),
    appid: "control-panel",
    target: authTarget,
    login_nonce: nonce,
  }, null), `auth.login(${username})`);
  assert(typeof result.session_token === "string" && result.session_token, `login(${username}) returned no token`);
  return result.session_token;
}

async function sudo(username: string, secret: string): Promise<string> {
  const nonce = nextNonce();
  const result = asRecord(await call("verify-hub", "sudo_by_password", {
    username,
    password: hashPassword(username, secret, nonce),
    target: authTarget,
    aud: "system-config",
    login_nonce: nonce,
  }, null), "sudo_by_password");
  assert(typeof result.session_token === "string" && result.session_token, "sudo token is missing");
  return result.session_token;
}

async function waitFor<T>(label: string, probe: () => Promise<T | undefined>, attempts = 30, delayMs = 500): Promise<T> {
  for (let i = 0; i < attempts; i++) {
    const found = await probe();
    if (found !== undefined) return found;
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  throw new Error(`${label}: not observed after ${attempts} attempts`);
}

const adminToken = await login(adminUser, adminPassword);
const cp = (method: string, params: JsonRecord = {}, token = adminToken) =>
  call("control-panel", method, params, token).then((value) => asRecord(value, method));
const policyText = async () =>
  String(asRecord(await call("system_config", "sys_config_get", { key: "system/rbac/policy" }, adminToken), "policy").value);
const before = await cp("user_group.list");
const savedProjections = asRecord(before.projections, "projections");

try {
  console.log("1. groups and accounts");
  await cp("user_group.create", { group_id: customerGroup, name: "DV customers" });
  await cp("user_group.create", { group_id: serviceGroup, name: "DV service" });
  await expectReject("duplicate group", () => cp("user_group.create", { group_id: customerGroup, name: "again" }));
  await expectReject("bad group id", () => cp("user_group.create", { group_id: "Bad Id", name: "x" }));
  const adminSudo = await sudo(adminUser, adminPassword);
  const createUser = (user_id: string, user_type: string, groups: string[]) =>
    cp("user.create", { user_id, user_type, groups, password_hash: hashPassword(user_id, password) }, adminSudo);
  await expectReject("create admin directly", () => createUser(`utadm${stamp}`, "admin", []));
  await expectReject("create with an undefined group", () => createUser(`utghost${stamp}`, "user", ["no-such-group"]));
  await createUser(carol, "limited", [customerGroup]);
  await createUser(sam, "user", [serviceGroup]);
  const users = (asRecord(await cp("user.list"), "user.list").users as JsonRecord[]) ?? [];
  const row = (id: string) => {
    const found = users.find((u) => u.user_id === id);
    assert(found, `user.list has ${id}`);
    return found;
  };
  assert(row(carol).user_type === "limited" && row(carol).can_edit_contacts === false && row(carol).can_change_password === false, "Limited defaults");
  assert(JSON.stringify(row(carol).groups) === JSON.stringify([customerGroup]), "carol is in the customer group");
  const carolDid = String(row(carol).did);
  const samDid = String(row(sam).did);
  console.log(`  ✓ ${carol} (limited, ${carolDid}) and ${sam} (user, ${samDid})`);

  console.log("2. RBAC lines");
  const policy = await waitFor("RBAC lines", async () => {
    const text = await policyText();
    return [`g, ${carol}, limited`, `g, ${carol}, group:${customerGroup}`, `g, ${sam}, users`, `g, ${sam}, group:${serviceGroup}`]
      .every((line) => text.split("\n").some((l) => l.trim() === line)) ? text : undefined;
  });
  assert(!policy.split("\n").some((l) => l.startsWith("p,") && l.includes(`group:${customerGroup}`)), "groups add no p rules");
  console.log("  ✓ type and group lines present, no group policy");

  console.log("3. what a Limited account reaches");
  const carolToken = await login(carol, password);
  const samToken = await login(sam, password);
  const sysGet = (token: string, key: string) => call("system_config", "sys_config_get", { key }, token);
  await sysGet(carolToken, `users/${carol}/settings`);
  await sysGet(carolToken, `users/${carol}/profile`);
  console.log("  ✓ own settings and profile readable");
  await expectReject("read another user's settings", () => sysGet(carolToken, `users/${sam}/settings`));
  await expectReject("read system config", () => sysGet(carolToken, "boot/config"));
  await expectReject("change own password", () =>
    cp("user.change_password", { new_password_hash: hashPassword(carol, `${password}-2`) }, carolToken));
  await expectReject("manage groups", () => cp("user_group.list", {}, carolToken));
  await expectReject("create an Agent", () => cp("agent.create", { name: `utagent${stamp}` }, carolToken), "limited");
  await expectReject("list own sessions (no mailbox)", () =>
    call("msg-center", "msg.list_sessions", { owner: carolDid }, carolToken));

  console.log("4. projection: defaults");
  const contactsOf = async (token: string, owner: string) =>
    (await call("msg-center", "contact.list_contacts", { query: { limit: 1000 }, contact_mgr_owner: owner }, token)) as JsonRecord[];
  const samView = await contactsOf(samToken, samDid);
  assert(!samView.some((c) => c.did === carolDid), "sam does not see the Limited carol by default");
  assert(samView.some((c) => (c.projected_from as string[] | undefined)?.includes("type:admin")), "sam sees admins by default");
  const carolView = await contactsOf(carolToken, carolDid);
  assert(carolView.length === 0, `carol sees nobody by default, got ${carolView.map((c) => c.did).join(",")}`);
  console.log(`  ✓ sam sees ${samView.length} entries without carol; carol sees none`);

  console.log("5. projection: customer → service");
  const rules = [
    ...(savedProjections.rules as JsonRecord[]),
    { source: `group:${customerGroup}`, target: `group:${serviceGroup}` },
  ];
  await cp("user_group.set_projections", { rules });
  const projected = await waitFor("carol in sam's address book", async () =>
    (await contactsOf(samToken, samDid)).find((c) => c.did === carolDid));
  assert((projected.projected_from as string[]).includes(`group:${customerGroup}`), "projected_from names the source group");
  assert((projected.tags as string[]).includes(`group:${customerGroup}`), "tags carry the source group");
  assert(!(await contactsOf(carolToken, carolDid)).some((c) => c.did === samDid), "carol still does not see sam");
  console.log(`  ✓ sam sees carol via ${JSON.stringify(projected.projected_from)}; carol does not see sam`);
  await expectReject("remove a projected entry", () =>
    call("msg-center", "contact.remove_contact", { did: carolDid, contact_mgr_owner: samDid }, samToken), ["projected-contact-read-only", "system-relation"]);
  await expectReject("carol edits her address book", () =>
    call("msg-center", "contact.update_tags", { did: samDid, add: ["dv"], contact_mgr_owner: carolDid }, carolToken), "contact-edit-not-allowed");

  console.log("6. direct message to an account without inbox");
  const dm = asRecord(await call("msg-center", "msg.post_send", {
    msg: { from: samDid, to: [carolDid], kind: "chat", created_at_ms: Date.now(), nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER), content: { format: "text/plain", content: "DV: should not be delivered" } },
    idempotency_key: `ut-dm-${stamp}`,
  }, samToken), "post_send");
  assert(dm.ok === false && String(dm.reason ?? JSON.stringify(dm.deliveries)).includes("recipient-has-no-inbox"), `DM fails clearly: ${JSON.stringify(dm)}`);
  console.log(`  ✓ ${dm.reason ?? JSON.stringify(dm.deliveries)}`);

  console.log("7. removing the rule leaves nothing behind");
  await cp("user_group.set_projections", { rules: savedProjections.rules });
  await waitFor("carol gone from sam's address book", async () =>
    (await contactsOf(samToken, samDid)).some((c) => c.did === carolDid) ? undefined : true);
  console.log("  ✓ projected entry disappeared");

  console.log("8. membership and type changes");
  await cp("user_group.set_members", { group_id: customerGroup, remove: [carol] });
  await waitFor("group line removed", async () =>
    (await policyText()).includes(`g, ${carol}, group:${customerGroup}`) ? undefined : true);
  await expectReject("change type to root", () => cp("user.change_type", { user_id: carol, user_type: "root" }, adminSudo));
  await expectReject("change type to guest", () => cp("user.change_type", { user_id: carol, user_type: "guest" }, adminSudo));
  await cp("user.change_type", { user_id: carol, user_type: "user" }, adminSudo);
  await waitFor("carol becomes users", async () =>
    (await policyText()).includes(`g, ${carol}, users`) ? true : undefined);
  console.log("  ✓ group line removed, type change reflected in RBAC");
} finally {
  console.log("9. cleanup");
  const adminSudo = await sudo(adminUser, adminPassword);
  if (savedProjections.is_default === true) {
    await cp("user_group.reset_projections").catch((e) => console.log(`  ! reset_projections: ${e}`));
  } else {
    await cp("user_group.set_projections", { rules: savedProjections.rules }).catch((e) => console.log(`  ! set_projections: ${e}`));
  }
  for (const group_id of [customerGroup, serviceGroup]) {
    await cp("user_group.delete", { group_id }).catch((e) => console.log(`  ! delete ${group_id}: ${e}`));
  }
  for (const user_id of [carol, sam]) {
    await cp("user.delete", { user_id }, adminSudo).catch((e) => console.log(`  ! delete ${user_id}: ${e}`));
  }
  console.log("  ✓ done");
}
