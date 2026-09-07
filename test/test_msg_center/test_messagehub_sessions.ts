/**
 * MessageHub real-RPC acceptance (UI_DATAMODEL §9.4 subset) against a running
 * zone: owner-scoped session lifecycle, activity ordering, empty-session
 * registration, PostSendResult, owner UI state, viewer → owner authorization
 * and the object access route.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_messagehub_sessions.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER
 * (devtest), BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_TEST_AGENT_DID
 * (did:web:jarvis.<zone>).
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
const agentDid = Deno.env.get("BUCKYOS_TEST_AGENT_DID")?.trim() || `did:web:jarvis.${zoneHost}`;
const selfDid = `did:bns:${adminUser}`;
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

async function expectReject(label: string, operation: () => Promise<unknown>): Promise<void> {
  try {
    await operation();
  } catch (error) {
    console.log(`  ✓ ${label}: ${error instanceof Error ? error.message : String(error)}`);
    return;
  }
  throw new Error(`${label} should have been rejected`);
}

async function login(): Promise<string> {
  const nonce = nextNonce();
  const result = asRecord(
    await call("control-panel", "auth.login", {
      username: adminUser,
      password: hashPassword(adminUser, adminPassword, nonce),
      appid: "control-panel",
      target: { kind: "system", service_id: "control-panel" },
      login_nonce: nonce,
    }, null),
    "auth.login response",
  );
  assert(typeof result.session_token === "string" && result.session_token, "login returned no token");
  return result.session_token;
}

const token = await login();
const msg = (method: string, params: JsonRecord, t: string | null = token) => call("msg-center", method, params, t);
const created: string[] = [];

try {
  console.log("1. self session list (activity order)");
  const mine = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }), "list_sessions");
  console.log(`  ✓ ${((mine.items as unknown[]) ?? []).length} active session(s) for ${selfDid}`);

  console.log("2. observe the zone agent (read only)");
  const observed = asRecord(await msg("msg.list_sessions", { owner: agentDid, order_by: "activity", lifecycle: "all", with_object: true }), "list_sessions(agent)");
  const observedItems = (observed.items as JsonRecord[]) ?? [];
  console.log(`  ✓ ${observedItems.length} session(s) observed for ${agentDid}`);
  for (const item of observedItems) {
    assert(typeof item.last_activity_ms === "number", "summary carries last_activity_ms");
    assert(typeof item.request_count === "number", "summary carries request_count");
    assert(item.lifecycle === "active" || item.lifecycle === "archived", "summary carries lifecycle");
    if (item.last_record) {
      const record = asRecord(asRecord(item.last_record, "last_record").record, "record");
      assert(record.owner === agentDid, "record owner is the observed agent");
    }
  }
  if (observedItems.length > 0) {
    const sessionId = String(observedItems[0].session_id);
    const timeline = asRecord(await msg("msg.list_session", { owner: agentDid, session_id: sessionId, limit: 5, with_object: true }), "list_session(agent)");
    const items = (timeline.items as JsonRecord[]) ?? [];
    console.log(`  ✓ agent timeline ${sessionId}: ${items.length} item(s), box kinds ${[...new Set(items.map((i) => i.box_kind))].join(",")}`);
    await expectReject("archive as agent", () => msg("msg.archive_session", { owner: agentDid, session_id: sessionId }));
    await expectReject("owner ui state write as agent", () => msg("ui_session.update_state", { owner: agentDid, session_id: sessionId, key: "ui.title", value: "x" }));
    const inbound = items.find((i) => i.direction === "in");
    if (inbound) {
      await expectReject("mark agent record read", () => msg("msg.update_record_state", { record_id: inbound.record_id, new_state: "READ" }));
    }
  }
  await expectReject("read another user's sessions", () => msg("msg.list_sessions", { owner: "did:bns:root" }));
  await expectReject("forged token", () => msg("msg.list_sessions", { owner: selfDid }, "eyJhbGciOiJFZERTQSJ9.e30.invalid"));

  console.log("3. register empty sessions with the agent");
  const first = asRecord(await msg("msg.create_session", { owner: selfDid, peer_did: agentDid, title: "RPC 验收", binding: { kind: "native", targetDid: agentDid } }), "create_session");
  const second = asRecord(await msg("msg.create_session", { owner: selfDid, peer_did: agentDid, title: "RPC 验收" }), "create_session");
  created.push(String(first.session_id), String(second.session_id));
  assert(first.session_id !== second.session_id, "same title yields independent sessions");
  assert(first.registered === true && first.lifecycle === "active", "registered active session");
  let page = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }), "list_sessions");
  let summary = ((page.items as JsonRecord[]) ?? []).find((i) => i.session_id === first.session_id);
  assert(summary, "empty session is listed");
  assert(summary.last_record === undefined && summary.unread_count === 0 && summary.last_activity_ms === first.created_at_ms, "empty session has no record and activity = created");
  const empty = asRecord(await msg("msg.list_session", { owner: selfDid, session_id: String(first.session_id), with_object: true }), "list_session");
  assert(((empty.items as unknown[]) ?? []).length === 0, "empty history");
  console.log(`  ✓ registered ${first.session_id} and ${second.session_id}`);

  console.log("4. post_send results");
  const bad = asRecord(await msg("msg.post_send", {
    msg: { from: selfDid, to: ["did:msgtunnel:1.user.no-such-tunnel"], kind: "chat", created_at_ms: Date.now(), content: { format: "text/plain", content: "unroutable" } },
    idempotency_key: `mh-bad-${Date.now()}`,
  }), "post_send(bad)");
  assert(bad.ok === false && typeof bad.reason === "string", "unroutable target returns ok:false with reason");
  console.log(`  ✓ ok:false reason: ${bad.reason}`);
  const key = `mh-ok-${Date.now()}`;
  const outgoing = { from: selfDid, to: [agentDid], kind: "chat", thread: { topic: String(first.session_id) }, created_at_ms: Date.now(), content: { format: "text/plain", content: "MessageHub 集成验收测试消息（可忽略）" } };
  const sent = asRecord(await msg("msg.post_send", { msg: outgoing, idempotency_key: key }), "post_send");
  assert(sent.ok === true && typeof sent.msg_id === "string" && Array.isArray(sent.deliveries), "post_send ok with deliveries");
  const replay = asRecord(await msg("msg.post_send", { msg: outgoing, idempotency_key: key }), "post_send(replay)");
  assert(replay.msg_id === sent.msg_id, "same idempotency key returns the same message");
  const timeline = asRecord(await msg("msg.list_session", { owner: selfDid, session_id: String(first.session_id), with_object: true }), "list_session");
  const outItems = (timeline.items as JsonRecord[]) ?? [];
  assert(outItems.length === 1 && outItems[0].direction === "out" && outItems[0].box_kind === "SENT", "first message joined the registered session once");
  assert(typeof asRecord(outItems[0].delivery, "delivery").overall === "string", "outbound item carries delivery view");
  page = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }), "list_sessions");
  summary = ((page.items as JsonRecord[]) ?? []).find((i) => i.session_id === first.session_id);
  assert(summary && summary.last_activity_ms === outgoing.created_at_ms, "activity advanced to the message time");
  console.log(`  ✓ sent ${sent.msg_id}, delivery ${asRecord(outItems[0].delivery, "delivery").overall}`);

  console.log("5. owner ui state");
  await msg("ui_session.update_state", { owner: selfDid, session_id: String(first.session_id), key: "ui.title", value: "我的标题" });
  const mineState = asRecord(await msg("ui_session.get_state", { owner: selfDid, session_id: String(first.session_id), key: "ui.title" }), "get_state");
  assert(mineState.value === "我的标题", "owner state round trip");
  const legacy = await msg("ui_session.get_state", { session_id: String(first.session_id), key: "ui.title" });
  assert(legacy === null || legacy === undefined, "legacy session-only KV untouched");
  console.log("  ✓ owner-scoped ui state isolated from legacy KV");

  console.log("6. lifecycle");
  const archived = asRecord(await msg("msg.archive_session", { owner: selfDid, session_id: String(first.session_id) }), "archive");
  assert(archived.lifecycle === "archived", "archived");
  page = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity" }), "list active");
  assert(!((page.items as JsonRecord[]) ?? []).some((i) => i.session_id === first.session_id), "archived session left the active list");
  page = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", lifecycle: "archived" }), "list archived");
  assert(((page.items as JsonRecord[]) ?? []).some((i) => i.session_id === first.session_id && i.lifecycle === "archived"), "archived list contains it");
  const restored = asRecord(await msg("msg.restore_session", { owner: selfDid, session_id: String(first.session_id) }), "restore");
  assert(restored.lifecycle === "active", "restored");
  const deleted = asRecord(await msg("msg.delete_session", { owner: selfDid, session_id: String(first.session_id) }), "delete");
  assert(typeof deleted.delete_watermark_sort_key === "number" && deleted.registered === false, "delete sets watermark and drops registration");
  const afterDelete = asRecord(await msg("msg.list_session", { owner: selfDid, session_id: String(first.session_id), with_object: true }), "list_session after delete");
  assert(((afterDelete.items as unknown[]) ?? []).length === 0, "history hidden after delete");
  page = asRecord(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", lifecycle: "all" }), "list all");
  assert(!((page.items as JsonRecord[]) ?? []).some((i) => i.session_id === first.session_id), "deleted session is not listed");
  const stateAfter = await msg("msg.get_session_state", { owner: selfDid, session_id: String(first.session_id) });
  assert(asRecord(stateAfter, "state").delete_watermark_sort_key === deleted.delete_watermark_sort_key, "watermark persisted");
  const uiAfter = await msg("ui_session.get_state", { owner: selfDid, session_id: String(first.session_id), key: "ui.title" });
  assert(uiAfter === null || uiAfter === undefined, "owner ui state cleared by delete");
  const replayed = asRecord(await msg("msg.post_send", { msg: outgoing, idempotency_key: key }), "post_send(replay after delete)");
  assert(replayed.msg_id === sent.msg_id, "idempotent replay still returns the old result");
  const afterReplay = asRecord(await msg("msg.list_session", { owner: selfDid, session_id: String(first.session_id), with_object: true }), "list_session after replay");
  assert(((afterReplay.items as unknown[]) ?? []).length === 0, "replayed old message stays hidden");
  console.log("  ✓ archive → restore → delete → replay verified");

  console.log("7. object access route");
  const objectUrl = `https://${zoneHost}/kapi/msg-center/objects/${encodeURIComponent(String(sent.msg_id))}`;
  const unauthenticated = await fetch(objectUrl);
  assert(unauthenticated.status === 401, `object route without token is 401 (got ${unauthenticated.status})`);
  const authenticated = await fetch(objectUrl, { headers: { authorization: `Bearer ${token}` } });
  assert(authenticated.status === 200, `object route with token is 200 (got ${authenticated.status})`);
  const objectJson = asRecord(await authenticated.json(), "object json");
  assert(objectJson.from === selfDid, "object json is the sent message");
  const content = await fetch(`${objectUrl}/content?token=${encodeURIComponent(token)}`);
  assert(content.status === 415, `message object has no content (got ${content.status})`);
  console.log("  ✓ objects route: 401 without token, 200 JSON with token, 415 content for non-file");

  console.log("ALL PASSED");
} finally {
  for (const sessionId of created) {
    try {
      await msg("msg.delete_session", { owner: selfDid, session_id: sessionId });
    } catch (error) {
      console.log(`cleanup ${sessionId} failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}
