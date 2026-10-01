/**
 * Self-host Group v2 real-RPC acceptance against a running zone (TODO §3):
 * create a group as the admin and invite a second zone user, accept the
 * invitation from the member (stranger invitations land in the member's
 * REQUEST_BOX and need an explicit `group.accept_invitation`), exchange
 * messages in the default and a named session, then remove / leave / delete.
 * A plain (non-admin) member also creates and deletes a group of their own,
 * which exercises the `obj://msg-center/group` `create` RBAC grant.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_messagehub_groups.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER
 * (devtest), BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_TEST_MEMBER_USER
 * (bob), BUCKYOS_TEST_MEMBER_PASSWORD (bucky2025).
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
const memberUser = Deno.env.get("BUCKYOS_TEST_MEMBER_USER")?.trim() || "bob";
const memberPassword = Deno.env.get("BUCKYOS_TEST_MEMBER_PASSWORD")?.trim() || "bucky2025";
const adminDid = `did:bns:${adminUser}`;
const memberDid = `did:bns:${memberUser}`;
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

function items(value: unknown, label: string): JsonRecord[] {
  return (asRecord(value, label).items as JsonRecord[]) ?? [];
}

async function call(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  const rpc = new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce());
  return await rpc.call(method, params);
}

async function expectReject(label: string, operation: () => Promise<unknown>): Promise<string> {
  try {
    await operation();
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    console.log(`  ✓ ${label}: ${reason}`);
    return reason;
  }
  throw new Error(`${label} should have been rejected`);
}

async function login(username: string, password: string): Promise<string> {
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

/** Poll until `probe` returns a value; group projections are written in the same transaction, so this is short. */
async function waitFor<T>(label: string, probe: () => Promise<T | undefined>, attempts = 10, delayMs = 500): Promise<T> {
  for (let i = 0; i < attempts; i++) {
    const found = await probe();
    if (found !== undefined) return found;
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  throw new Error(`${label}: not observed after ${attempts} attempts`);
}

const adminToken = await login(adminUser, adminPassword);
const memberToken = await login(memberUser, memberPassword);
const asAdmin = (method: string, params: JsonRecord) => call("msg-center", method, params, adminToken);
const asMember = (method: string, params: JsonRecord) => call("msg-center", method, params, memberToken);
const groups: Array<{ did: string; token: string }> = [];

/** Invitation notices (`buckyos.group_invitation`) delivered to `owner` from `from`, newest first. */
async function invitationNotices(token: string, owner: string, from: string, groupDid: string): Promise<JsonRecord[]> {
  const sessionId = `dm:${from}`;
  const page = asRecord(await call("msg-center", "msg.list_session", { owner, session_id: sessionId, limit: 50, descending: true, with_object: true }, token), `list_session(${sessionId})`);
  const out: JsonRecord[] = [];
  for (const item of (page.items as JsonRecord[]) ?? []) {
    const msg = item.msg as JsonRecord | undefined;
    const machine = ((msg?.content as JsonRecord | undefined)?.machine as JsonRecord | undefined);
    if (machine?.intent !== "buckyos.group_invitation") continue;
    const data = asRecord(machine.data, "machine.data");
    if (data.group_did === groupDid) out.push(data);
  }
  return out;
}

try {
  console.log("1. admin creates a group and invites the member");
  const created = asRecord(await asAdmin("group.create", {
    idempotency_key: `group-acceptance-${Date.now()}`,
    profile: { name: `RPC 验收群 ${new Date().toISOString()}` },
    invitations: [{ member_did: memberDid }],
  }), "group.create");
  const groupDid = String(created.group_did);
  groups.push({ did: groupDid, token: adminToken });
  console.log(`  ✓ created ${groupDid}`);
  const mine = items(await asAdmin("group.list_by_member", {}), "list_by_member");
  assert(mine.some((g) => asRecord(g.doc, "doc").id === groupDid), "creator sees the group in list_by_member");
  const members = items(await asAdmin("group.list_members", { group_did: groupDid }), "list_members");
  const invited = members.find((m) => m.member_did === memberDid);
  assert(invited && ["invited", "active"].includes(String(invited.state)), `member is invited or auto-joined (got ${invited?.state})`);
  assert(invited.invited_by === adminDid, "invitation records the inviter");
  await expectReject("forged actor_did", () => asAdmin("group.list_members", { group_did: groupDid, actor_did: memberDid }));

  console.log("2. member sees the invitation card and accepts it");
  let memberState = String(invited.state);
  if (memberState === "invited") {
    const notice = await waitFor("invitation notice in the member's dm", async () => {
      const notices = await invitationNotices(memberToken, memberDid, adminDid, groupDid);
      return notices.find((n) => n.action === "invite");
    });
    const data = asRecord(notice.data, "invite data");
    assert(typeof data.invite_id === "string", "invite carries invite_id");
    assert(data.state === "invited", "stranger invitation waits for the member");
    const access = asRecord(await asMember("group.check_access", { group_did: groupDid, action: "session.post" }), "check_access");
    assert(access.allowed === false, "not a member before accepting");
    await expectReject("stale invitation id", () => asMember("group.accept_invitation", { group_did: groupDid, invitation_id: "stale" }));
    const accepted = asRecord(await asMember("group.accept_invitation", { group_did: groupDid, invitation_id: data.invite_id }), "accept_invitation");
    memberState = String(accepted.state);
    assert(memberState === "active", `admin invitation activates on accept (got ${memberState})`);
    await expectReject("accepting twice", () => asMember("group.accept_invitation", { group_did: groupDid, invitation_id: data.invite_id }));
  } else {
    console.log("  ✓ member was auto-accepted (inviter is a friend)");
  }
  const access = asRecord(await asMember("group.check_access", { group_did: groupDid, action: "session.post" }), "check_access");
  assert(access.allowed === true, "member may post after joining");
  const after = items(await asAdmin("group.list_members", { group_did: groupDid }), "list_members");
  assert(after.filter((m) => m.state === "active").length === 2, "two active members");
  console.log("  ✓ member active");

  console.log("3. messages in the default session");
  const text = `群验收消息 ${Date.now()}`;
  const sent = asRecord(await asAdmin("msg.post_send", {
    msg: { from: adminDid, to: [groupDid], kind: "group_msg", created_at_ms: Date.now(), nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER), content: { format: "text/plain", content: text } },
    idempotency_key: `group-msg-${Date.now()}`,
  }), "post_send(group)");
  assert(sent.ok === true, `group message accepted (${sent.reason ?? ""})`);
  const seen = await waitFor("member sees the group message", async () => {
    const page = items(await asMember("msg.list_session", { owner: memberDid, session_id: groupDid, limit: 20, with_object: true }), "member group timeline");
    return page.find((i) => (asRecord(i.msg ?? {}, "msg").content as JsonRecord | undefined)?.content === text);
  });
  assert(seen.direction === "in" && seen.box_kind === "INBOX", "member got an INBOX projection");
  const own = items(await asAdmin("msg.list_session", { owner: adminDid, session_id: groupDid, limit: 20, with_object: true }), "admin group timeline");
  const copies = own.filter((i) => (asRecord(i.msg ?? {}, "msg").content as JsonRecord | undefined)?.content === text);
  assert(copies.length === 1 && copies[0].direction === "out", "sender only holds the SENT record, no INBOX copy");
  const reply = asRecord(await asMember("msg.post_send", {
    msg: { from: memberDid, to: [groupDid], kind: "group_msg", created_at_ms: Date.now(), nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER), content: { format: "text/plain", content: `回复 ${text}` } },
    idempotency_key: `group-reply-${Date.now()}`,
  }), "post_send(reply)");
  assert(reply.ok === true, `member reply accepted (${reply.reason ?? ""})`);
  console.log("  ✓ both directions delivered");

  console.log("4. named session");
  const topic = asRecord(await asAdmin("group.create_session", { group_did: groupDid, title: "验收话题", idempotency_key: `topic-${Date.now()}` }), "create_session");
  const sid = String(topic.session_id);
  const sessions = items(await asMember("group.list_sessions", { group_did: groupDid }), "list_sessions");
  assert(sessions.some((s) => s.session_id === sid && (s.shared_state as JsonRecord).title === "验收话题"), "member lists the named session with its title");
  const topicMsg = asRecord(await asMember("msg.post_send", {
    msg: { from: memberDid, to: [groupDid], kind: "group_msg", to_session: sid, created_at_ms: Date.now(), nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER), content: { format: "text/plain", content: "话题消息" } },
    idempotency_key: `topic-msg-${Date.now()}`,
  }), "post_send(topic)");
  assert(topicMsg.ok === true, `named session message accepted (${topicMsg.reason ?? ""})`);
  const unknown = asRecord(await asMember("msg.post_send", {
    msg: { from: memberDid, to: [groupDid], kind: "group_msg", to_session: "no-such-session", created_at_ms: Date.now(), content: { format: "text/plain", content: "x" } },
    idempotency_key: `topic-bad-${Date.now()}`,
  }), "post_send(unknown session)");
  assert(unknown.ok === false, "unknown session is rejected");
  console.log(`  ✓ ${String(topic.session)} works, unknown session rejected: ${unknown.reason}`);

  console.log("5. remove, leave, delete");
  await asAdmin("group.remove_member", { group_did: groupDid, member_did: memberDid });
  const removed = asRecord(await asMember("group.check_access", { group_did: groupDid, action: "session.post" }), "check_access(removed)");
  assert(removed.allowed === false, "removed member cannot post");
  const reinvite = asRecord(await asAdmin("group.invite_member", { group_did: groupDid, member_did: memberDid }), "invite_member(again)");
  if (reinvite.state === "invited") {
    await asMember("group.accept_invitation", { group_did: groupDid, invitation_id: reinvite.invite_id });
  }
  await asMember("group.leave", { group_did: groupDid });
  await expectReject("owner must transfer before leaving", () => asAdmin("group.leave", { group_did: groupDid }));
  await asAdmin("group.delete", { group_did: groupDid, idempotency_key: `delete-${Date.now()}` });
  groups.pop();
  await expectReject("deleted group is gone", () => asAdmin("group.get_doc", { group_did: groupDid }));
  console.log("  ✓ lifecycle verified");

  console.log("6. a plain zone user may create a group (RBAC users create)");
  const own2 = asRecord(await asMember("group.create", {
    idempotency_key: `member-group-${Date.now()}`,
    profile: { name: `成员自建群 ${Date.now()}` },
  }), "group.create(member)");
  groups.push({ did: String(own2.group_did), token: memberToken });
  await asMember("group.delete", { group_did: String(own2.group_did), idempotency_key: `member-delete-${Date.now()}` });
  groups.pop();
  console.log(`  ✓ ${memberDid} created and deleted ${own2.group_did}`);

  console.log("ALL PASSED");
} finally {
  for (const group of groups) {
    try {
      await call("msg-center", "group.delete", { group_did: group.did, idempotency_key: `cleanup-${Date.now()}` }, group.token);
    } catch (error) {
      console.log(`cleanup ${group.did} failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}
