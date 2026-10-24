/**
 * sys-test official example, backend DV (notepads/self-host-group-extension-todo.md §11,
 * EXT-10 / EXT-11). Drives the installed `buckyos-systest` App the way its page does and
 * then plays the customer:
 *
 *   1. owner switches App login on; creates the support group (panel = the App's own page)
 *   2. the App backend issues a 48-hour customer link (`user.app_login.issue {kind:"link"}`)
 *   3. the customer signs in with the link twice (links are reusable), joins the Session,
 *      posts; the panel summary counts the message; the link is not an exchangeable code
 *   4. the customer sets a password: the link now asks for it, the password signs in
 *   5. owner switches App login off: no more links; switch back on; delete the group
 *
 * Run from the repository root against an activated single Zone with the App installed:
 *   deno run --config test/deno.json --allow-net --allow-env --unsafely-ignore-certificate-errors \
 *     test/test_sys_test/support_group_dv.ts
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER / _PASSWORD
 * (devtest / bucky2025), BUCKYOS_TEST_APP_HOST (buckyos-systest), BUCKYOS_TEST_APP_ID
 * (buckyos-systest.buckyos.bns.did), BUCKYOS_TEST_MEMBERS (alice,bob).
 */
import { asRecord, assert, call, items, login, zoneHost, type JsonRecord } from "../test_msg_center/group_ext_sample/common.ts";
import { buckyos } from "buckyos";

const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;
const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const appHost = Deno.env.get("BUCKYOS_TEST_APP_HOST")?.trim() || "buckyos-systest";
const appId = Deno.env.get("BUCKYOS_TEST_APP_ID")?.trim() || "buckyos-systest.buckyos.bns.did";
const members = (Deno.env.get("BUCKYOS_TEST_MEMBERS")?.trim() || "alice,bob").split(",").map((m) => m.trim()).filter(Boolean);
const appOrigin = `https://${appHost}.${zoneHost}`;
const appInstanceId = `${appId}@${adminUser}`;
const runId = Date.now().toString(36);
const SESSION_ID = "support";

const adminToken = await login(adminUser, adminPassword);
const asAdmin = (method: string, params: JsonRecord) => call("msg-center", method, params, adminToken);
const cp = (method: string, params: JsonRecord, token: string | null) => call("control-panel", method, params, token);

async function appApi(path: string, init: RequestInit): Promise<{ status: number; body: JsonRecord }> {
  const response = await fetch(`${appOrigin}${path}`, init);
  const text = await response.text();
  let body: JsonRecord = {};
  try {
    body = JSON.parse(text) as JsonRecord;
  } catch {
    body = { raw: text.slice(0, 200) };
  }
  return { status: response.status, body };
}

async function customerLink(groupDid: string, external: string, showName: string) {
  return await appApi("/sdk/appservice/support/customer_link", {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${adminToken}` },
    body: JSON.stringify({ group_did: groupDid, session_id: SESSION_ID, external_account_id: external, show_name: showName, title: `客服工单 ${runId}` }),
  });
}

async function followCallback(url: string): Promise<string> {
  const response = await fetch(url, { redirect: "manual" });
  const cookie = response.headers.getSetCookie().map((c) => c.split(";")[0]).find((c) => c.startsWith("buckyos_session_token="));
  assert(cookie, `callback set the session cookie (status ${response.status})`);
  return cookie.split("=").slice(1).join("=");
}

async function signInWithLink(link: string): Promise<{ token: string; inviteId: string; secret: string }> {
  const url = new URL(link);
  const inviteId = url.pathname.split("/").pop()!;
  const secret = new URLSearchParams(url.hash.slice(1)).get("s")!;
  const result = asRecord(await cp("auth.invite.login", { invite_id: inviteId, secret }, null), "auth.invite.login");
  assert(result.status === "ready" && typeof result.callback_url === "string", `invite login: ${JSON.stringify(result)}`);
  return { token: await followCallback(String(result.callback_url)), inviteId, secret };
}

const zoneUsers = (asRecord(await cp("user.list", {}, adminToken), "user.list").users as JsonRecord[]) ?? [];
const didOf = (userId: string) => {
  const did = zoneUsers.find((u) => u.user_id === userId && typeof u.did === "string")?.did;
  assert(typeof did === "string", `user.list has a DID for ${userId}`);
  return did;
};

let groupDid = "";
let switchWasOn = false;
try {
  console.log("1. owner: App login on, support group with the App's panel");
  const before = asRecord(await cp("apps.app_login.get", { app_instance_id: appInstanceId }, adminToken), "apps.app_login.get");
  switchWasOn = asRecord(before.settings, "settings").enabled === true;
  await cp("apps.app_login.set", { app_instance_id: appInstanceId, enabled: true }, adminToken);
  const health = await appApi("/sdk/appservice/healthz", {});
  assert(health.body.ok === true, `sys_test backend is ready: ${JSON.stringify(health.body)}`);
  const panelUrl = `${appOrigin}/support/panel`;
  const panel = await fetch(panelUrl);
  assert(panel.ok && (await panel.text()).includes("客服工单"), `panel page served at ${panelUrl} (${panel.status})`);
  const name = `客服群（sys-test DV ${runId}）`;
  const created = asRecord(await asAdmin("group.create", {
    idempotency_key: `systest-support-${runId}`,
    profile: { name },
    configuration: {
      required_features: ["sessions", "session_guests", "panels"],
      display: { announcement: `${name}：客服与客户在 Session「${SESSION_ID}」里交流。` },
      extensions: { panels: [{ id: "tickets", title: "客服工单", entry_url: panelUrl, scope: "group" }] },
      session_templates: { support: { membership: { roles: ["owner", "admin"] }, rules: { allow_guests: true, post: "all_participants", panels: ["tickets"] } } },
    },
    sessions: [{ session_id: SESSION_ID, template: "support", title: "客服工单" }],
    invitations: members.filter((m) => m !== adminUser).map((m) => ({ member_did: didOf(m) })),
  }), "group.create");
  groupDid = String(created.group_did);
  const sessions = items(await asAdmin("group.list_sessions", { group_did: groupDid }), "list_sessions");
  const support = sessions.find((s) => s.session_id === SESSION_ID)!;
  assert((support.panels as JsonRecord[]).some((p) => p.entry_url === panelUrl), "the Session carries the App panel");
  console.log(`  ✓ ${groupDid}; members invited: ${members.join(", ")}`);

  console.log("2. App backend issues a 48-hour customer link");
  const forbidden = await appApi("/sdk/appservice/support/customer_link", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  assert(forbidden.status === 401, `no page session → 401 (got ${forbidden.status})`);
  const external = `dv:${runId}`;
  const issued = await customerLink(groupDid, external, "DV 客户");
  assert(issued.status === 200 && issued.body.ok === true && typeof issued.body.link === "string", `customer_link: ${JSON.stringify(issued.body)}`);
  const link = String(issued.body.link);
  assert(link.startsWith(`https://${zoneHost}/sso/invite/`) && link.includes("#s="), `link is a Zone-root exclusive invite: ${link.split("#")[0]}`);
  const ttl = Number(issued.body.expires_at) - Math.floor(Date.now() / 1000);
  assert(Math.abs(ttl - 48 * 3600) < 120, `link lives 48 hours (ttl ${ttl}s)`);
  assert(issued.body.new_account === true, "first sight of this external account creates the Limited account");
  const customer = String(issued.body.user_id);
  const again = await customerLink(groupDid, external, "DV 客户");
  assert(again.body.user_id === customer && again.body.new_account === false, "the same external account reuses the Limited account");
  console.log(`  ✓ link for ${customer} (ttl ${Math.round(ttl / 3600)}h), reissue reuses the account`);

  console.log("3. customer: sign in with the link (twice), join, post; summary counts; not a code");
  const first = await signInWithLink(link);
  const second = await signInWithLink(link);
  assert(first.token !== second.token, "a link signs in more than once");
  const identity = asRecord(await cp("auth.current_identity", {}, first.token), "customer identity");
  assert(identity.auth_method === "exclusive_invite" && identity.restricted === true && identity.user_id === customer, `identity: ${JSON.stringify(identity)}`);
  await call("msg-center", "group.accept_session_invitation", { group_did: groupDid, session_id: SESSION_ID }, first.token);
  const view = asRecord(await call("msg-center", "group.list_sessions", { group_did: groupDid }, first.token), "customer list_sessions");
  const seen = view.items as JsonRecord[];
  assert(seen.length === 1 && seen[0].session_id === SESSION_ID && (seen[0].panels as JsonRecord[]).length === 1, "the customer sees only its Session, with the panel");
  const customerDid = String(identity.did);
  const posted = asRecord(await call("msg-center", "msg.post_send", {
    idempotency_key: crypto.randomUUID(),
    msg: { from: customerDid, to: [groupDid], kind: "group_msg", to_session: SESSION_ID, created_at_ms: Date.now(), nonce: Math.floor(Math.random() * 1e9), content: { format: "text/plain", content: `客户留言 ${runId}` } },
  }, first.token), "msg.post_send");
  assert(posted.ok === true, `customer posts: ${JSON.stringify(posted)}`);
  const summary = await appApi(`/sdk/appservice/support/summary?${new URLSearchParams({ group_did: groupDid, session_id: SESSION_ID })}`, {});
  assert(summary.body.ok === true && Number(summary.body.count) >= 1 && Number(summary.body.participants) >= 1, `panel summary: ${JSON.stringify(summary.body)}`);
  assert(JSON.stringify(summary.body).includes(`客户留言 ${runId}`) === false, "the summary carries no message text");
  const exchange = asRecord(await cp("auth.app_login.exchange", { code: `${first.inviteId}.${first.secret}`, nonce: crypto.randomUUID() }, null), "exchange");
  assert(exchange.status === "denied" && asRecord(exchange.denial, "denial").detail === "credential_kind", `a link is not a code: ${JSON.stringify(exchange)}`);
  console.log("  ✓ reusable link, Session + panel visible, post counted without text, exchange refuses the link");

  console.log("4. customer sets a password: the link asks for it, the password signs in");
  const password = `dv-pass-${runId}`;
  const set = asRecord(await cp("auth.invite.set_password", { new_password_hash: hashPassword(customer, password) }, first.token), "set_password");
  assert(set.status === "ok", `set_password: ${JSON.stringify(set)}`);
  const relogin = asRecord(await cp("auth.invite.login", { invite_id: first.inviteId, secret: first.secret }, null), "relogin");
  assert(relogin.status === "denied" && asRecord(relogin.denial, "denial").detail === "password_required", `after password: ${JSON.stringify(relogin)}`);
  const byPassword = await login(customer, password);
  const afterView = asRecord(await call("msg-center", "group.list_sessions", { group_did: groupDid }, byPassword), "password session list_sessions");
  assert((afterView.items as JsonRecord[]).length === 1, "the password session reaches the same Session");
  const consumedReissue = await customerLink(groupDid, external, "DV 客户");
  assert(consumedReissue.status === 200, "the App may still issue a new link record; it is refused at login while the password is set");
  const reissued = await cp("auth.invite.login", { invite_id: new URL(String(consumedReissue.body.link)).pathname.split("/").pop()!, secret: new URLSearchParams(new URL(String(consumedReissue.body.link)).hash.slice(1)).get("s")! }, null);
  assert(asRecord(reissued, "reissued").status === "denied", "a fresh link for an account with a password does not sign in");
  console.log("  ✓ password_required on the link, password login works");

  console.log("5. owner switches App login off: no links; back on");
  await cp("apps.app_login.set", { app_instance_id: appInstanceId, enabled: false }, adminToken);
  const off = await customerLink(groupDid, `dv-off:${runId}`, "关掉后");
  const denial = asRecord(off.body.data ?? {}, "denial");
  assert(off.status === 502 && off.body.code === "access_denied" && denial.reason === "zone_method_disabled" && denial.detail === "app_login_disabled", `issue while off: ${JSON.stringify(off.body)}`);
  await cp("apps.app_login.set", { app_instance_id: appInstanceId, enabled: true }, adminToken);
  console.log("  ✓ switch governs link issuance");
  console.log("\nsupport_group DV passed");
} finally {
  if (groupDid) await asAdmin("group.delete", { group_did: groupDid }).catch((e) => console.log(`cleanup: ${e}`));
  if (!switchWasOn) await cp("apps.app_login.set", { app_instance_id: appInstanceId, enabled: false }, adminToken).catch(() => {});
}
