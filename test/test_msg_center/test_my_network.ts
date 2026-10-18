/**
 * My Network relationship RPCs against a running zone (notepads/my-network-upgrade-todo.md §6.9):
 * lookup, user tags and their write rules, Zone relations that cannot be removed, a friend
 * request to an unreachable DID, and the HomeStation `homestation.following` projection.
 * All users of one Zone are friends already, so a request round trip between two Zones is
 * covered by `cargo test -p msg_center test_contact_network` and not here.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_my_network.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_TEST_MEMBER_USER (bob),
 * BUCKYOS_TEST_MEMBER_PASSWORD (bucky2025).
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
const unreachableDid = "did:web:nobody.invalid";
const userTag = `DV 验收 ${Date.now()}`;
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

async function expectReject(label: string, expected: string, operation: () => Promise<unknown>): Promise<void> {
  try {
    await operation();
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    assert(reason.includes(expected), `${label}: expected "${expected}", got "${reason}"`);
    console.log(`  ✓ ${label}: ${reason}`);
    return;
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

async function waitFor<T>(label: string, probe: () => Promise<T | undefined>, attempts = 20, delayMs = 500): Promise<T> {
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
const homestation = (method: string, params: JsonRecord) => call("homestation", method, params, adminToken);
const contactOf = async (did: string) =>
  await asAdmin("contact.get_contact", { did, contact_mgr_owner: adminDid }) as JsonRecord | null;

let pendingRequest: string | undefined;
let followedSource: string | undefined;
try {
  console.log("1. lookup finds Zone users by name and by DID");
  const byName = asRecord(await asAdmin("contact.lookup", { input: memberUser }), "lookup(name)");
  const found = (byName.candidates as JsonRecord[]).find((c) => c.did === memberDid);
  assert(found, `${memberUser} is a candidate`);
  assert(asRecord(found.relation, "relation").friend_state === "friend", "Zone users are friends");
  assert(asRecord(found.reachability, "reachability").state === "local", "a Zone user is local");
  console.log(`  ✓ ${memberUser}: section=${found.section} friend`);
  const byDid = asRecord(await asAdmin("contact.lookup", { input: memberDid }), "lookup(did)");
  assert((byDid.candidates as JsonRecord[])[0]?.did === memberDid, "the DID itself comes first");
  const bad = asRecord(await asAdmin("contact.lookup", { input: "did:web:g.example?nothing=1" }), "lookup(bad)");
  assert(asRecord(bad.error, "error").code === "invalid-input", "a broken link is invalid-input");
  console.log("  ✓ DID lookup and invalid-input");

  console.log("2. user tags: the owner writes them, reserved tags stay closed");
  const tagged = asRecord(await asAdmin("contact.update_tags", { did: memberDid, add: [userTag] }), "update_tags");
  assert((tagged.tags as string[]).includes(userTag), "tag added");
  const byTag = await asAdmin("contact.list_contacts", { query: { tag: userTag }, contact_mgr_owner: adminDid }) as JsonRecord[];
  assert(byTag.length === 1 && byTag[0].did === memberDid, "list_contacts filters by tag");
  await expectReject("user writes an app tag", "tag-not-writable", () =>
    asAdmin("contact.update_tags", { did: memberDid, add: ["homestation.following"] }));
  await expectReject("user writes a system tag", "tag-not-writable", () =>
    asAdmin("contact.update_tags", { did: memberDid, remove: ["zone_user"] }));
  await expectReject("member writes the admin's store", "", () =>
    asMember("contact.update_tags", { did: memberDid, add: [userTag], contact_mgr_owner: adminDid }));
  await asAdmin("contact.update_tags", { did: memberDid, remove: [userTag] });
  console.log("  ✓ tag removed again");

  console.log("3. Zone relations are friends and cannot be removed");
  const already = asRecord(await asAdmin("contact.send_friend_request", { did: memberDid }), "send(friend)");
  assert(already.friend_state === "friend" && !already.request, "no request between Zone users");
  await expectReject("remove_friend", "system-relation", () => asAdmin("contact.remove_friend", { did: memberDid }));
  await expectReject("remove_contact", "system-relation", () => asAdmin("contact.remove_contact", { did: memberDid }));

  console.log("4. a request to an unreachable DID reports why");
  const sent = asRecord(await asAdmin("contact.send_friend_request", { did: unreachableDid, message: "DV" }), "send(unreachable)");
  const request = asRecord(sent.request, "request");
  pendingRequest = String(request.request_id);
  assert(sent.friend_state === "outgoing_pending", "the owner waits for confirmation");
  const delivery = await waitFor("delivery outcome", async () => {
    const list = asRecord(await asAdmin("contact.list_friend_requests", { direction: "outgoing" }), "list");
    const mine = (list.items as JsonRecord[]).find((r) => r.request_id === pendingRequest);
    const state = mine?.delivery ? asRecord(mine.delivery, "delivery") : undefined;
    return state?.state === "failed" ? state : undefined;
  });
  console.log(`  ✓ delivery failed: ${delivery.reason}`);
  const cancelled = asRecord(await asAdmin("contact.cancel_friend_request", { request_id: pendingRequest }), "cancel");
  assert(asRecord(cancelled.request, "request").state === "cancelled", "request cancelled");
  pendingRequest = undefined;
  await asAdmin("contact.remove_contact", { did: unreachableDid });
  assert(await contactOf(unreachableDid) === null, "the contact record is gone");
  console.log("  ✓ cancelled and removed");

  console.log("5. HomeStation mirrors an active follow as homestation.following");
  const sources = asRecord(await homestation("sources.list", {}), "sources.list").sources as JsonRecord[];
  const existing = sources.find((s) => s.did === memberDid);
  const active = existing && (existing.basis as string[]).includes("active");
  if (!active) {
    const resolution = await homestation("sources.resolve", { kind: "follow", text: memberDid });
    followedSource = String(await homestation("sources.follow", { resolution }));
  }
  await waitFor("following tag", async () => {
    const tags = ((await contactOf(memberDid))?.tags as string[] | undefined) ?? [];
    return tags.includes("homestation.following") ? true : undefined;
  });
  console.log("  ✓ tagged");
  if (followedSource) {
    const list = asRecord(await homestation("sources.list", {}), "sources.list").sources as JsonRecord[];
    const id = String(list.find((s) => s.did === memberDid)?.id);
    await homestation("sources.unfollow", { sourceId: id });
    followedSource = undefined;
    await waitFor("following tag removed", async () => {
      const tags = ((await contactOf(memberDid))?.tags as string[] | undefined) ?? [];
      return tags.includes("homestation.following") ? undefined : true;
    });
    console.log("  ✓ unfollow removed the tag");
  }
  console.log("My Network DV acceptance passed");
} finally {
  if (pendingRequest) {
    await asAdmin("contact.cancel_friend_request", { request_id: pendingRequest }).catch(() => undefined);
  }
  await asAdmin("contact.update_tags", { did: memberDid, remove: [userTag] }).catch(() => undefined);
}
