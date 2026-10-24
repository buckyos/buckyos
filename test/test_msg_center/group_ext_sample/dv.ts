/**
 * DV of the Self-host Group extension layers against one running zone (TODO EXT-08):
 * L1 declarations and secrets, L3 Hook signatures / denial / rotation (a Hook service runs
 * inside this script), the external customer through an exclusive invite, and, when an App
 * service token is given, App 代登录 issuance and exchange. See README.md for the env.
 *
 *   deno run --config test/deno.json --allow-net --allow-env --allow-read \
 *     --unsafely-ignore-certificate-errors test/test_msg_center/group_ext_sample/dv.ts
 */
import { appId, asRecord, assert, call, expectReject, items, login, supportGroupConfig, verifyHookRequest, zoneHost } from "./common.ts";

const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const memberUser = Deno.env.get("BUCKYOS_TEST_MEMBER_USER")?.trim() || "bob";
const memberPassword = Deno.env.get("BUCKYOS_TEST_MEMBER_PASSWORD")?.trim() || "bucky2025";
const hookUrl = Deno.env.get("BUCKYOS_TEST_HOOK_URL")?.trim() || "http://127.0.0.1:8787/hook";
const hookPort = Number(new URL(hookUrl).port || 8787);
const appInstance = Deno.env.get("BUCKYOS_TEST_APP_INSTANCE")?.trim() ?? "";
/** The App's service session: given directly, or exchanged from the login assertion the node
 * daemon hands the App container (`BUCKYOS_APP_TOKEN`) through verify-hub `login_by_jwt`. */
const appToken = await (async () => {
  const direct = Deno.env.get("BUCKYOS_TEST_APP_TOKEN")?.trim();
  if (direct) return direct;
  const assertion = Deno.env.get("BUCKYOS_TEST_APP_LOGIN_JWT")?.trim();
  if (!assertion || !appInstance) return "";
  const pair = asRecord(await call("verify-hub", "login_by_jwt", { type: "jwt", jwt: assertion, target: { kind: "app", app_instance_id: appInstance } }, null), "login_by_jwt");
  return String(pair.session_token);
})();

// ── the Hook service, with a secret the script swaps during the run ──
let hookSecret = "";
let hookDecision: "allow" | "deny" = "allow";
const hookCalls: string[] = [];
const hookServer = Deno.serve({ port: hookPort, onListen: () => {} }, async (req) => {
  const { ok, body } = await verifyHookRequest(hookSecret, req);
  if (!ok) {
    hookCalls.push("unauthorized");
    return new Response("{}", { status: 401 });
  }
  const parsed = JSON.parse(body) as Record<string, unknown>;
  assert(!req.headers.has("authorization"), "hook requests carry no service token");
  assert(typeof parsed.group_host === "string", "hook body names the group host");
  hookCalls.push(`${parsed.operation}:${hookDecision}`);
  console.log(`  hook ${parsed.operation} ${parsed.actor} → ${hookDecision}`);
  return Response.json({ decision: hookDecision });
});

const adminToken = await login(adminUser, adminPassword);
const memberToken = await login(memberUser, memberPassword);
const did = async (token: string) => String(asRecord(await call("control-panel", "auth.current_identity", {}, token), "identity").did);
const adminDid = await did(adminToken);
const memberDid = await did(memberToken);
const asAdmin = (m: string, p: Record<string, unknown>) => call("msg-center", m, p, adminToken);
const asMember = (m: string, p: Record<string, unknown>) => call("msg-center", m, p, memberToken);
let groupDid = "";

function groupMessage(from: string, sid: string, text: string) {
  return { from, to: [groupDid], kind: "group_msg", to_session: sid, created_at_ms: Date.now(), nonce: Math.floor(Math.random() * 1e9), content: { format: "text/plain", content: text } };
}
async function post(token: string, from: string, sid: string, text: string) {
  return asRecord(await call("msg-center", "msg.post_send", { idempotency_key: crypto.randomUUID(), msg: groupMessage(from, sid, text) }, token), "msg.post_send");
}

try {
  console.log("1. L1: create the support group with panels, display and hooks");
  const created = asRecord(await asAdmin("group.create", {
    idempotency_key: `group-ext-dv-${Date.now()}`,
    profile: { name: `客服群 DV ${new Date().toISOString()}` },
    configuration: supportGroupConfig(hookUrl, adminDid),
    sessions: [{ session_id: "tickets", template: "support" }],
    invitations: [{ member_did: memberDid, role: "admin" }],
  }), "group.create");
  groupDid = String(created.group_did);
  const secrets = created.hook_secrets as Array<{ scope: string; index: number; secret: string }>;
  assert(secrets?.length === 2 && secrets.every((s) => s.scope === "template:support"), "hook secrets returned once");
  hookSecret = secrets.find((s) => s.index === 1)!.secret;
  console.log(`  ✓ created ${groupDid}, 2 hook secrets returned`);

  const sessions = items(await asAdmin("group.list_sessions", { group_did: groupDid }), "list_sessions");
  const tickets = sessions.find((s) => s.session_id === "tickets")!;
  const panels = tickets.panels as Array<Record<string, unknown>>;
  assert(panels.map((p) => p.id).join(",") === "tickets,sla", `tickets panels: ${JSON.stringify(panels)}`);
  assert((sessions.find((s) => s.session_id === null)!.panels as unknown[]).length === 1, "default session shows only the group-scoped panel");
  assert(asRecord(tickets.display, "display").announcement === "本 Session 由客服与客户共享。", "session display overrides the group announcement");
  assert((asRecord(tickets.display, "display").links as unknown[]).length === 1, "group links show through");
  assert(JSON.stringify(sessions).indexOf(hookSecret) < 0, "list_sessions never contains a hook secret");
  const config = asRecord(await asAdmin("group.get_config", { group_did: groupDid }), "get_config");
  assert(JSON.stringify(config).indexOf(hookSecret) < 0, "get_config never contains a hook secret");
  const doc = asRecord(await asAdmin("group.get_doc", { group_did: groupDid }), "get_doc");
  assert(doc.panels === undefined && doc.extensions === undefined, "public doc has no panels");
  console.log("  ✓ list_sessions carries panels / display; secrets are hidden");

  await expectReject("foreign panel origin", () => asAdmin("group.apply_config", {
    group_did: groupDid, expected_revision: config.revision, idempotency_key: crypto.randomUUID(),
    patch: { extensions: { panels: [{ id: "evil", title: "x", entry_url: "https://evil.example/p" }, ...(asRecord(config.extensions, "ext").panels as unknown[])] } },
  }));

  const member = await (async () => {
    const pending = asRecord(await asMember("group.list_by_member", {}), "list_by_member");
    const inv = ((pending.invitations as Array<Record<string, unknown>>) ?? []).find((i) => i.group_did === groupDid);
    if (inv) await asMember("group.accept_invitation", { group_did: groupDid, invitation_id: inv.invite_id });
    return memberDid;
  })();
  const access = asRecord(await asMember("group.check_access", { group_did: groupDid, session_id: "tickets", action: "session.post" }), "check_access");
  assert(access.allowed === "hook_dependent", `check_access on a may_grant session: ${JSON.stringify(access)}`);
  console.log("  ✓ check_access(session.post) = hook_dependent");

  console.log("2. L3: signed hooks, business deny, rotation");
  const ok = await post(memberToken, member, "tickets", "hello from the member");
  assert(ok.ok === true, `signed post accepted: ${JSON.stringify(ok)}`);
  assert(hookCalls.at(-1) === "post:allow", `hook saw the post: ${hookCalls}`);
  hookDecision = "deny";
  const denied = await post(memberToken, member, "tickets", "this one is refused");
  assert(denied.ok === false && denied.reason === "hook-denied", `business deny: ${JSON.stringify(denied)}`);
  hookDecision = "allow";
  const rotated = asRecord(await asAdmin("group.rotate_hook_secret", { group_did: groupDid, session_id: "tickets", index: 1 }), "rotate");
  await expectReject("stale secret → signature refused → unavailable (deny policy)", () => post(memberToken, member, "tickets", "after rotation"));
  assert(hookCalls.at(-1) === "unauthorized", `service rejected the stale signature: ${hookCalls}`);
  hookSecret = String(rotated.secret);
  const after = await post(memberToken, member, "tickets", "with the new secret");
  assert(after.ok === true, `post accepted with the rotated secret: ${JSON.stringify(after)}`);
  console.log("  ✓ HMAC verified, deny is final, rotation changes the signature");

  console.log("3. external customer through an exclusive invite sees the same panels");
  const invite = asRecord(await call("control-panel", "user.exclusive_invite.create", {
    new_user: { user_id: `dvcust${Date.now().toString(36)}`, show_name: "DV 客户" },
    channel: { kind: "email", target_hint: "d***@example.com" },
    resource: { kind: "group_session", group_did: groupDid, session_id: "tickets", title: "工单" },
  }, adminToken), "exclusive_invite.create");
  const link = new URL(String(invite.link));
  const inviteId = link.pathname.split("/").pop()!;
  const secret = new URLSearchParams(link.hash.slice(1)).get("s")!;
  const loginResult = asRecord(await call("control-panel", "auth.invite.login", { invite_id: inviteId, secret }, null), "auth.invite.login");
  assert(loginResult.status === "ready" && typeof loginResult.callback_url === "string", `invite login: ${JSON.stringify(loginResult)}`);
  // Follow the callback without redirects to obtain the host-only cookie, then use it as the guest's session.
  const callback = await fetch(String(loginResult.callback_url), { redirect: "manual" });
  const cookie = callback.headers.getSetCookie().map((c) => c.split(";")[0]).find((c) => c.startsWith("buckyos_session_token="));
  assert(cookie, `callback set the session cookie (status ${callback.status})`);
  const guestToken = cookie.split("=").slice(1).join("=");
  await call("msg-center", "group.accept_session_invitation", { group_did: groupDid, session_id: "tickets" }, guestToken);
  const guestView = asRecord(await call("msg-center", "group.list_sessions", { group_did: groupDid }, guestToken), "guest list_sessions");
  const guestItems = guestView.items as Array<Record<string, unknown>>;
  assert(guestItems.length === 1 && guestItems[0].session_id === "tickets", "guest sees only its Session");
  assert((guestItems[0].panels as unknown[]).length === 2 && guestView.group_doc === null, "guest gets the panels and display but no group doc");
  const guestPost = await post(guestToken, await did(guestToken), "tickets", "customer here");
  assert(guestPost.ok === true, `guest can post through the hook: ${JSON.stringify(guestPost)}`);
  console.log("  ✓ exclusive-invite customer: panels + display visible, no viewer identity passed");

  if (appToken && appInstance) {
    console.log("4. App 代登录");
    await call("control-panel", "apps.app_login.set", { app_instance_id: appInstance, enabled: true }, adminToken);
    const issued = asRecord(await call("control-panel", "user.app_login.issue", {
      external_account_id: `crm:${Date.now()}`,
      show_name: "CRM 客户",
      resource: { kind: "group_session", group_did: groupDid, session_id: "tickets", title: "工单" },
      return_path: "/tickets/tickets",
    }, appToken), "user.app_login.issue");
    assert(typeof issued.code === "string" && issued.new_account === true, `issued: ${JSON.stringify({ ...issued, code: "…" })}`);
    const again = asRecord(await call("control-panel", "user.app_login.issue", { external_account_id: `crm:${Date.now()}` }, appToken), "issue 2");
    assert(again.user_id !== issued.user_id, "another external account is another Limited account");
    const nonce = crypto.randomUUID();
    const exchange = asRecord(await call("control-panel", "auth.app_login.exchange", { code: issued.code, nonce }, null), "exchange");
    assert(exchange.status === "ready" && String(exchange.callback_url).startsWith(`https://${appId}.${zoneHost}/sso_callback`), `exchange: ${JSON.stringify(exchange)}`);
    // Follow the callback once: the customer's cookie lands on the App origin and that
    // restricted session reaches its Session on msg-center (scope msg_group), nothing else.
    const cb = await fetch(String(exchange.callback_url), { redirect: "manual" });
    const customerCookie = cb.headers.getSetCookie().map((c) => c.split(";")[0]).find((c) => c.startsWith("buckyos_session_token="));
    assert(customerCookie, `App callback set the customer cookie (status ${cb.status})`);
    const customerToken = customerCookie.split("=").slice(1).join("=");
    const customerIdentity = asRecord(await call("control-panel", "auth.current_identity", {}, customerToken), "customer identity");
    assert(customerIdentity.auth_method === "app_access_token" && customerIdentity.restricted === true, `customer identity: ${JSON.stringify(customerIdentity)}`);
    await call("msg-center", "group.accept_session_invitation", { group_did: groupDid, session_id: "tickets" }, customerToken);
    const customerView = asRecord(await call("msg-center", "group.list_sessions", { group_did: groupDid }, customerToken), "customer list_sessions");
    assert((customerView.items as unknown[]).length === 1, "the App-signed customer sees only its Session");
    await expectReject("customer cannot list users", () => call("control-panel", "user.list", {}, customerToken));
    const retry = asRecord(await call("control-panel", "auth.app_login.exchange", { code: issued.code, nonce }, null), "exchange retry");
    assert(retry.status === "ready", "same nonce retries");
    const replay = asRecord(await call("control-panel", "auth.app_login.exchange", { code: issued.code, nonce: crypto.randomUUID() }, null), "exchange replay");
    assert(replay.status === "denied" && asRecord(replay.denial, "denial").detail === "nonce_mismatch", `replay refused: ${JSON.stringify(replay)}`);
    await call("control-panel", "apps.app_login.set", { app_instance_id: appInstance, enabled: false }, adminToken);
    await expectReject("issue while App login is off", () => call("control-panel", "user.app_login.issue", { external_account_id: "crm:off" }, appToken));
    console.log("  ✓ code issued once, exchanged once, replay and disabled switch refused");
  } else {
    console.log("4. App 代登录 skipped (set BUCKYOS_TEST_APP_TOKEN and BUCKYOS_TEST_APP_INSTANCE)");
  }
  console.log("DV passed");
} finally {
  if (groupDid) await asAdmin("group.delete", { group_did: groupDid }).catch((e) => console.log(`cleanup: ${e}`));
  await hookServer.shutdown();
}
