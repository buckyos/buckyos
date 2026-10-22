/**
 * Hybrid SSO acceptance against a running Zone (notepads/hybrid-sso-implementation-todo.md
 * SSO-14). Raw kRPC so the `code` / `data` denial fields are read off the wire.
 *
 *   deno run --allow-net --allow-env --unsafely-ignore-certificate-errors test_hybrid_sso_dv.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_TEST_DID (the admin's DID for the
 * single-Zone DID login loop; skipped when unset), HYBRID_SSO_STEPS (comma list to run a subset).
 */

type Json = Record<string, unknown>;

const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const loopDid = Deno.env.get("BUCKYOS_TEST_DID")?.trim() || "";
const onlySteps = (Deno.env.get("HYBRID_SSO_STEPS") ?? "").split(",").map((s) => s.trim()).filter(Boolean);
const origin = `https://${zoneHost}`;
const runId = Date.now().toString(36);
let seq = Date.now();

class RpcFailure extends Error {
  constructor(message: string, readonly code?: string, readonly data?: Json) {
    super(message);
  }
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

async function sha256b64(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return btoa(String.fromCharCode(...new Uint8Array(digest)));
}

/** websdk `hashPassword`: base64 sha256(password + username + ".buckyos"), then with the nonce. */
async function passwordHash(username: string, password: string, nonce?: number): Promise<string> {
  const base = await sha256b64(`${password}${username}.buckyos`);
  return nonce === undefined ? base : await sha256b64(`${base}${nonce}`);
}

async function rpc(service: string, method: string, params: Json, token: string | null = null): Promise<Json> {
  seq = Math.max(Date.now(), seq + 1);
  const sys: unknown[] = token ? [seq, token] : [seq];
  const response = await fetch(`${origin}/kapi/${service}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ method, params, sys }),
  });
  if (!response.ok) throw new RpcFailure(`${service}.${method}: HTTP ${response.status}`);
  const body = await response.json() as Json;
  if (body.error) {
    throw new RpcFailure(`${service}.${method}: ${body.error}`, body.code as string | undefined, body.data as Json | undefined);
  }
  const result = (body.result ?? {}) as Json;
  // AI Workspace returns business failures as results: `{ok:false, error:{code, denial?}}`.
  if (result.ok === false && result.error && typeof result.error === "object") {
    const error = result.error as Json;
    const denial = error.denial as Json | undefined;
    throw new RpcFailure(`${service}.${method}: ${error.code}`, denial ? "access_denied" : String(error.code), denial);
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

async function login(username: string, password: string): Promise<{ token: string; identity: Json }> {
  const nonce = Date.now();
  const result = await rpc("control-panel", "auth.login", {
    username,
    password: await passwordHash(username, password, nonce),
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  });
  assert(typeof result.session_token === "string", `login(${username}) returned no token`);
  return { token: result.session_token as string, identity: result.identity as Json };
}

/** Follow a CP callback URL like a browser: returns the cookies it set. */
async function followCallback(url: string): Promise<Record<string, string>> {
  const response = await fetch(url, { redirect: "manual" });
  assert(response.status === 302, `callback ${url} → HTTP ${response.status}: ${await response.text()}`);
  const cookies: Record<string, string> = {};
  for (const line of response.headers.getSetCookie()) {
    const [pair] = line.split(";");
    const index = pair.indexOf("=");
    cookies[pair.slice(0, index)] = pair.slice(index + 1);
  }
  cookies.__location = response.headers.get("location") ?? "";
  return cookies;
}

async function ssoRefresh(refreshCookie: string): Promise<{ status: number; body: Json; refresh?: string }> {
  const response = await fetch(`${origin}/sso_refresh`, {
    method: "POST",
    headers: { Cookie: `buckyos_refresh_token=${refreshCookie}` },
  });
  const body = await response.json().catch(() => ({})) as Json;
  let refresh: string | undefined;
  for (const line of response.headers.getSetCookie()) {
    const [pair] = line.split(";");
    if (pair.startsWith("buckyos_refresh_token=")) refresh = pair.slice("buckyos_refresh_token=".length);
  }
  return { status: response.status, body, refresh };
}

const steps: Array<[string, () => Promise<void>]> = [];
const step = (name: string, run: () => Promise<void>) => steps.push([name, run]);
const state: Json = {};

const admin = await login(adminUser, adminPassword);
console.log(`admin ${adminUser} identity ${JSON.stringify(admin.identity)}`);

step("settings", async () => {
  const config = await rpc("control-panel", "auth.sso_config", {});
  console.log(`    sso_config ${JSON.stringify(config)}`);
  const settings = await rpc("control-panel", "sso.settings.get", {}, admin.token);
  state.settings = settings;
  assert(typeof settings.self_register_enabled === "boolean", "sso.settings.get shape");
});

step("support-group", async () => {
  const configuration = {
    profile: { name: `混合 SSO 验收 ${runId}` },
    membership: {
      guest_entry: {
        session_template: "tickets",
        max_open_per_guest: 2,
        auth_methods: { password: true, self_register: true, did_login: true },
      },
    },
    session_templates: { tickets: { membership: "explicit", rules: { allow_guests: true } } },
  };
  const created = await rpc("msg-center", "group.create", { idempotency_key: `hsso-${runId}`, configuration }, admin.token);
  state.groupDid = String(created.group_did);
  await rpc("msg-center", "group.create_session", {
    group_did: state.groupDid,
    session_id: "ticket-dv",
    membership: "explicit",
    rule_overrides: { allow_guests: true },
  }, admin.token);
  const described = await rpc("msg-center", "group.describe_entry", { group_did: state.groupDid });
  assert(described.state === "active", `group.describe_entry ${JSON.stringify(described)}`);
  console.log(`    group ${state.groupDid} with guest Session ticket-dv`);
});

step("exclusive-invite", async () => {
  const userId = `dvcust${runId}`;
  const created = await rpc("control-panel", "user.exclusive_invite.create", {
    new_user: { user_id: userId, show_name: "DV 客户" },
    channel: { kind: "email", target_hint: "c***@example.com" },
    resource: { kind: "group_session", group_did: state.groupDid, session_id: "ticket-dv", title: "DV 工单" },
  }, admin.token);
  const invite = created.invite as Json;
  const link = new URL(String(created.link));
  assert(invite.resource_state === "done", `resource_state ${invite.resource_state}`);
  assert(!("digest" in (invite.credential as Json)), "admin view leaks the credential digest");
  state.customer = userId;
  state.inviteId = invite.invite_id;
  state.secret = new URLSearchParams(link.hash.slice(1)).get("s");
  assert(link.pathname === `/sso/invite/${invite.invite_id}` && state.secret, `link ${link}`);
  console.log(`    invite ${state.inviteId} for ${userId}, link path ${link.pathname}`);

  const describe = await rpc("control-panel", "auth.invite.describe", { invite_id: state.inviteId, secret: state.secret });
  assert(describe.status === "active" && describe.user_id === userId, `describe ${JSON.stringify(describe)}`);
  const wrong = await rpc("control-panel", "auth.invite.describe", { invite_id: state.inviteId, secret: "wrong" });
  assert(wrong.status === "denied" && (wrong.denial as Json).detail === "link_replaced" && !("user_id" in wrong), `wrong secret ${JSON.stringify(wrong)}`);

  for (const attempt of [1, 2]) {
    const login = await rpc("control-panel", "auth.invite.login", { invite_id: state.inviteId, secret: state.secret });
    assert(login.status === "ready", `invite login #${attempt} ${JSON.stringify(login)}`);
    const cookies = await followCallback(String(login.callback_url));
    assert(cookies.buckyos_session_token && cookies.buckyos_refresh_token, "callback set both cookies");
    state.customerToken = cookies.buckyos_session_token;
    state.customerRefresh = cookies.buckyos_refresh_token;
    console.log(`    ✓ invite login #${attempt} (link reusable), back to ${cookies.__location}`);
  }
  const identity = await rpc("control-panel", "auth.current_identity", {}, state.customerToken as string);
  assert(identity.kind === "account" && identity.restricted === true && identity.auth_method === "exclusive_invite" && identity.password_set === false, `identity ${JSON.stringify(identity)}`);
  state.customerDid = identity.did;
});

step("restricted-gate", async () => {
  const token = state.customerToken as string;
  await expectDenied("task-manager list", "session_kind_not_allowed", () => rpc("task-manager", "list_tasks", {}, token));
  await expectDenied("msg-center owner sessions", "session_kind_not_allowed", () => rpc("msg-center", "msg.list_sessions", { owner: state.customerDid }, token));
  await expectDenied("control-panel user.list", "session_kind_not_allowed", () => rpc("control-panel", "user.list", {}, token));
  await expectDenied("control-panel other user", "session_kind_not_allowed", () => rpc("control-panel", "user.get", { user_id: adminUser }, token));
  const own = await rpc("control-panel", "user.get", {}, token);
  assert(own.user_id === state.customer || (own.settings as Json | undefined)?.user_id === state.customer || JSON.stringify(own).includes(String(state.customer)), "own user.get works");
  console.log("    ✓ own user.get allowed");
});

step("guest-session", async () => {
  const token = state.customerToken as string;
  const group = state.groupDid as string;
  await rpc("msg-center", "group.accept_session_invitation", { group_did: group, session_id: "ticket-dv", idempotency_key: `accept-${runId}` }, token);
  const sessions = await rpc("msg-center", "group.list_sessions", { group_did: group }, token);
  console.log(`    sessions visible to the guest: ${JSON.stringify((sessions.items as Json[] | undefined)?.map((s) => s.session_id) ?? sessions)}`);
  const msg = {
    from: state.customerDid,
    to: [group],
    kind: "group_msg",
    to_session: "ticket-dv",
    created_at_ms: Date.now(),
    content: { content: `客户留言 ${runId}`, content_type: "text/plain" },
  };
  const sent = await rpc("msg-center", "msg.post_send", { msg, idempotency_key: `post-${runId}` }, token);
  assert(sent.ok !== false, `post_send ${JSON.stringify(sent)}`);
  const asAdmin = await rpc("msg-center", "group.list_messages", { group_did: group, session_id: "ticket-dv", after_seq: 0, limit: 20, with_object: true }, admin.token);
  const asGuest = await rpc("msg-center", "group.list_messages", { group_did: group, session_id: "ticket-dv", after_seq: 0, limit: 20, with_object: true }, token);
  const found = (page: Json) => ((page.items as Json[]) ?? []).find((item) => JSON.stringify(item.msg ?? {}).includes(`客户留言 ${runId}`));
  assert(found(asAdmin), "admin sees the guest message");
  const own = found(asGuest);
  assert(own, "guest sees its own message");
  const msgId = String(own.obj_id);
  const sessionAddress = String((sessions.items as Json[] | undefined)?.find((s) => s.session_id === "ticket-dv")?.session ?? "");
  if (sessionAddress) {
    const objectUrl = `${origin}/kapi/msg-center/objects/${msgId}?context_path=${encodeURIComponent(`${sessionAddress}/${msgId}`)}`;
    const object = await fetch(objectUrl, { headers: { Authorization: `Bearer ${token}` } });
    assert(object.ok, `object read with context → HTTP ${object.status}`);
    await object.body?.cancel();
    const noContext = await fetch(`${origin}/kapi/msg-center/objects/${msgId}`, { headers: { Authorization: `Bearer ${token}` } });
    assert(!noContext.ok, `object read without context should fail, got ${noContext.status}`);
    await noContext.body?.cancel();
    console.log("    ✓ message object readable only with its Session context");
  } else {
    console.log("    (session address not in list_sessions result; object route check skipped)");
  }
});

step("first-password", async () => {
  const token = state.customerToken as string;
  const user = state.customer as string;
  const result = await rpc("control-panel", "auth.invite.set_password", { new_password_hash: await passwordHash(user, "dv-password-123") }, token);
  assert(result.status === "ok", `set_password ${JSON.stringify(result)}`);
  const describe = await rpc("control-panel", "auth.invite.describe", { invite_id: state.inviteId, secret: state.secret });
  assert(describe.status === "denied" && (describe.denial as Json).detail === "password_required" && describe.user_id === user, `after password ${JSON.stringify(describe)}`);
  const relogin = await rpc("control-panel", "auth.invite.login", { invite_id: state.inviteId, secret: state.secret });
  assert(relogin.status === "denied", "the old link no longer signs in");
  const byPassword = await login(user, "dv-password-123");
  assert(byPassword.identity.kind === "account" && (byPassword.identity as Json).restricted === true, "password login keeps the Limited ceiling");
  const current = await rpc("control-panel", "auth.current_identity", {}, token);
  assert(current.password_set === true, "the current terminal stays signed in");
  await expectDenied("rotate after password", "credential_state", () => rpc("control-panel", "user.exclusive_invite.rotate", { invite_id: state.inviteId }, admin.token));
  console.log("    ✓ password set: link only an entry, current terminal kept");
});

step("revoke", async () => {
  const user = `dvrev${runId}`;
  const created = await rpc("control-panel", "user.exclusive_invite.create", {
    new_user: { user_id: user },
    channel: { kind: "sms", target_hint: "138****0000" },
  }, admin.token);
  const inviteId = String((created.invite as Json).invite_id);
  const secret = new URLSearchParams(new URL(String(created.link)).hash.slice(1)).get("s") ?? "";
  const login = await rpc("control-panel", "auth.invite.login", { invite_id: inviteId, secret });
  const cookies = await followCallback(String(login.callback_url));
  const rotated = await rpc("control-panel", "user.exclusive_invite.rotate", { invite_id: inviteId }, admin.token);
  const newSecret = new URLSearchParams(new URL(String(rotated.link)).hash.slice(1)).get("s") ?? "";
  const old = await rpc("control-panel", "auth.invite.describe", { invite_id: inviteId, secret });
  assert(old.status === "denied" && (old.denial as Json).detail === "link_replaced", `old link after rotate ${JSON.stringify(old)}`);
  const resend = await rpc("control-panel", "auth.invite.request_resend", { invite_id: inviteId });
  console.log(`    resend on a fresh credential → ${resend.status}`);
  await rpc("control-panel", "user.exclusive_invite.revoke", { invite_id: inviteId }, admin.token);
  const revoked = await rpc("control-panel", "auth.invite.describe", { invite_id: inviteId, secret: newSecret });
  assert(revoked.status === "denied" && (revoked.denial as Json).reason === "source_revoked", `revoked ${JSON.stringify(revoked)}`);
  const refresh = await ssoRefresh(cookies.buckyos_refresh_token);
  assert(refresh.status !== 200, `refresh of a revoked invite family must fail, got ${refresh.status}`);
  console.log("    ✓ rotate replaces the link, revoke ends the invite's families");
});

step("aiws-entry", async () => {
  const ws = await rpc("aiworkspace", "ws.create", { title: `DJ 电台 ${runId}` }, admin.token);
  const workspaceId = String(ws.workspace_id ?? ws.id);
  const entry = await rpc("aiworkspace", "ws.create_entry", {
    workspace_id: workspaceId,
    capabilities: ["read", "comment"],
    methods: { guest: true, password: true, self_register: true, did_login: true },
  }, admin.token);
  const entryId = String((entry.entry as Json).entry_id);
  const descriptor = await rpc("control-panel", "auth.entry.describe", { service: "aiworkspace", entry_id: entryId });
  assert(descriptor.state === "active", `entry describe ${JSON.stringify(descriptor)}`);
  const methods = descriptor.methods as Json;
  assert(methods.guest && methods.self_register, `methods ${JSON.stringify(methods)}`);

  const base32 = "abcdefghijklmnopqrstuvwxyz234567";
  const guestToken = `gt_${Array.from(crypto.getRandomValues(new Uint8Array(26)), (b) => base32[b % 32]).join("")}`;
  const asGuest = await rpc("aiworkspace", "ws.get_info", { workspace_id: workspaceId }, guestToken);
  assert(asGuest.principal && String(asGuest.principal).startsWith("guest:"), `guest ws.get_info ${JSON.stringify(asGuest)}`);
  console.log(`    ✓ anonymous guest reads through the guest entry as ${asGuest.principal}`);

  const user = `dvreg${runId}`;
  const registered = await rpc("control-panel", "auth.register", {
    service: "aiworkspace",
    entry_id: entryId,
    user_id: user,
    show_name: "DV 注册访客",
    password_hash: await passwordHash(user, "dv-register-123"),
  });
  assert(registered.status === "created", `register ${JSON.stringify(registered)}`);
  const visitor = await login(user, "dv-register-123");
  assert((visitor.identity as Json).restricted === true, "self-registered accounts are Limited");
  await rpc("aiworkspace", "ws.create", { title: "x" }, visitor.token).then(() => {
    throw new Error("a Limited account created a workspace");
  }, (error) => console.log(`    ✓ Limited cannot create a workspace (${error instanceof RpcFailure ? `${error.code}/${error.data?.reason ?? "-"}` : error})`));
  const joined = await rpc("aiworkspace", "ws.join_entry", { entry_id: entryId }, visitor.token);
  assert(joined.joined !== undefined || joined.ok, `join ${JSON.stringify(joined)}`);
  const info = await rpc("aiworkspace", "ws.get_info", { workspace_id: workspaceId }, visitor.token);
  console.log(`    ✓ registered visitor joined and reads ${info.workspace_id ?? workspaceId}`);

  const settings = state.settings as Json | undefined;
  await rpc("control-panel", "sso.settings.set", { self_register_enabled: false }, admin.token);
  try {
    const closed = await rpc("control-panel", "auth.entry.describe", { service: "aiworkspace", entry_id: entryId });
    assert((closed.zone_disabled as string[] | undefined)?.includes("self_register"), `zone_disabled ${JSON.stringify(closed)}`);
    const refused = await rpc("control-panel", "auth.register", {
      service: "aiworkspace",
      entry_id: entryId,
      user_id: `${user}x`,
      password_hash: await passwordHash(`${user}x`, "dv-register-123"),
    });
    assert(refused.status === "denied" && (refused.denial as Json).reason === "zone_method_disabled", `register when closed ${JSON.stringify(refused)}`);
    console.log("    ✓ Zone switch closes self-registration");
  } finally {
    await rpc("control-panel", "sso.settings.set", { self_register_enabled: settings?.self_register_enabled ?? true }, admin.token);
  }
});

step("did-loop", async () => {
  if (!loopDid) {
    console.log("    (BUCKYOS_TEST_DID unset; skipped)");
    return;
  }
  const begin = await rpc("control-panel", "auth.did.begin", { did: loopDid, redirect_url: `${origin}/` });
  console.log(`    begin → ${JSON.stringify(begin).slice(0, 200)}`);
  assert(begin.status === "redirect", "did.begin");
  const federate = new URL(String(begin.url));
  const params = Object.fromEntries(federate.searchParams.entries());
  if (federate.host !== zoneHost) {
    console.log(`    canonical host ${federate.host} is another Zone; single-Zone loop skipped`);
    return;
  }
  const consent = await rpc("control-panel", "auth.federation.describe", params, admin.token);
  assert(consent.status === "consent", `consent ${JSON.stringify(consent)}`);
  const approved = await rpc("control-panel", "auth.federation.approve", { ...params, remember: true }, admin.token);
  assert(approved.status === "redirect", `approve ${JSON.stringify(approved)}`);
  const cookies = await followCallback(String(approved.url));
  assert(cookies.buckyos_session_token, `did callback set no session, location ${cookies.__location}`);
  const identity = await rpc("control-panel", "auth.current_identity", {}, cookies.buckyos_session_token);
  assert(identity.auth_method === "did_federation", `identity ${JSON.stringify(identity)}`);
  console.log(`    ✓ DID login loop → ${identity.kind} ${identity.user_id ?? identity.did}`);
});

let failed = 0;
for (const [name, run] of steps) {
  if (onlySteps.length && !onlySteps.includes(name)) continue;
  console.log(`- ${name}`);
  try {
    await run();
  } catch (error) {
    failed += 1;
    console.log(`  ✗ ${name}: ${error instanceof RpcFailure ? `${error.message} code=${error.code} data=${JSON.stringify(error.data)}` : error}`);
  }
}
console.log(failed ? `${failed} step(s) failed` : "all steps passed");
Deno.exit(failed ? 1 : 0);
