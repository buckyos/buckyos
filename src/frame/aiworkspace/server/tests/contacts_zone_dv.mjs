// Run on an activated dev Zone with both updated services; allow 21 minutes for the scheduled refresh.
// deno run --allow-net --allow-env src/frame/aiworkspace/server/tests/contacts_zone_dv.mjs
// This creates two temporary contact records and one shared workspace, then removes them on exit.
import { createHash, randomUUID } from "node:crypto";
import assert from "node:assert/strict";

const base = Deno.env.get("AIWS_ZONE_BASE") ?? "http://127.0.0.1:3180/kapi";
const requester = Deno.env.get("BUCKYOS_TEST_ADMIN_USER") ?? "devtest";
const member = Deno.env.get("BUCKYOS_TEST_MEMBER_USER") ?? "bob";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const runId = randomUUID();
let nonce = Date.now();
function next() {
  nonce = Math.max(Date.now(), nonce + 1);
  return nonce;
}

async function rpc(service, method, params, token) {
  const response = await fetch(base + "/" + service, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      method,
      params,
      sys: token ? [next(), token] : [next()],
    }),
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) {
    throw new Error(method + " returned HTTP " + response.status);
  }
  const { result, error } = await response.json();
  if (error || result?.ok === false) {
    throw new Error(method + " " + JSON.stringify(error ?? result.error));
  }
  return result;
}

async function login(account, password) {
  const hash = (value) => createHash("sha256").update(value).digest("base64");
  const n = next();
  const session = await rpc("control-panel", "auth.login", {
    username: account,
    password: hash(hash(password + account + ".buckyos") + n),
    login_nonce: n,
    appid: "control-panel",
    target: { kind: "system", service_id: "control-panel" },
  });
  const token = session.session_token;
  const ctx = await rpc("control-panel", "owner.context", {}, token);
  assert.equal(ctx.owner_user_id, account);
  assert.ok(ctx.owner_did);
  return { account, token, did: ctx.owner_did };
}

const ownerPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD") ??
  "bucky2025";
const memberPassword = Deno.env.get("BUCKYOS_TEST_MEMBER_PASSWORD") ??
  "buckyos2026";
const alice = await login(requester, ownerPassword);
const bob = await login(member, memberPassword);
assert.notEqual(alice.did, bob.did);
const contacts = (user, method, params) =>
  rpc(
    "msg-center",
    method,
    { contact_mgr_owner: user.did, ...params },
    user.token,
  );
const work = (user, method, params) =>
  rpc("aiworkspace", method, params, user.token);
const ws = await work(alice, "ws.create", { title: "Contacts DV " + runId });
const bound = (user, method, params) =>
  work(user, method, { workspace_id: ws.workspace_id, ...params });
const refs = ["a", "b"].map((prefix) =>
  "did:web:" + prefix + "-" + runId + ".example.invalid"
);
const [aliceRef, bobRef] = refs;
const cleanup = [() => bound(alice, "ws.delete", {})];

async function waitRun(user, runId) {
  for (let attempt = 0; attempt < 100; attempt++) {
    const run = await bound(user, "proc.get", { run_id: runId });
    if (run.state === "succeeded") return;
    if (["failed", "cancelled"].includes(run.state)) {
      throw new Error(
        JSON.stringify(run.result?.error ?? run.error ?? run.state),
      );
    }
    await sleep(500);
  }
  throw new Error("refresh did not settle");
}

const record = async (user, did) => {
  const table = await bound(user, "doc.query", {
    sql: "SELECT _record FROM t WHERE did = ?",
    tables: { t: "friends" },
    params: [did],
  });
  return table.rows.map(([row]) => JSON.parse(row));
};

try {
  for (const [index, user] of [alice, bob].entries()) {
    const did = refs[index];
    assert.equal(await contacts(user, "contact.get_contact", { did }), null);
    await contacts(user, "contact.update_tags", { did, add: ["aiws-dv"] });
    cleanup.push(() => contacts(user, "contact.remove_contact", { did }));
    await contacts(user, "contact.update_contact", {
      did,
      patch: { name: "Before sync", note: "DV private field" },
    });
  }
  await bound(alice, "acl.set", {
    changes: [{ subject: bob.account, resource: "/", allow: ["read", "run"] }],
  });
  const first = await bound(alice, "source.bind_contacts", {
    epoch: ws.epoch,
    entity_id: "friends",
    create: { parent_id: "data", order_key: "a", name: "Contacts DV" },
    idempotency_key: runId,
  });
  await waitRun(alice, first.run_id);
  const meta = await bound(alice, "doc.read", {
    entity_id: "friends",
    as: "meta",
  });
  const interval = meta.data.source.refresh.interval_s;
  assert.equal(interval, 900);
  const [entry] = await record(bob, aliceRef);
  assert.deepEqual(Object.keys(entry.values).sort(), [
    "contact_ref",
    "did",
    "friend_state",
    "name",
    "tags",
  ]);
  assert.equal(entry.values.name, "Before sync");
  assert.equal((await record(bob, bobRef)).length, 0);
  const manual = await bound(bob, "proc.start", {
    program: "source.refresh@1",
    params: { entity_id: "friends" },
    idempotency_key: runId + "-run",
  });
  await waitRun(bob, manual.run_id);
  assert.equal((await record(bob, aliceRef))[0].record_id, entry.record_id);
  assert.equal((await record(bob, bobRef)).length, 0);
  console.log(
    "Owner binding, collaborator run, projection and shared SQL verified.",
  );
  await contacts(alice, "contact.update_contact", {
    did: aliceRef,
    patch: { name: "After background sync" },
  });
  console.log(
    "Upstream temporary contact renamed. Waiting for the 900-second service timer without a workspace client.",
  );
  await sleep(interval * 1000);
  const [ownerSession, memberSession] = await Promise.all([
    login(requester, ownerPassword),
    login(member, memberPassword),
  ]);
  assert.equal(ownerSession.did, alice.did);
  assert.equal(memberSession.did, bob.did);
  alice.token = ownerSession.token;
  bob.token = memberSession.token;
  const deadline = Date.now() + 330_000;
  while (true) {
    const [row] = await record(bob, aliceRef);
    if (row?.values.name === "After background sync") {
      assert.equal(row.record_id, entry.record_id);
      break;
    }
    if (Date.now() >= deadline) {
      throw new Error("scheduled refresh missed the rename");
    }
    await sleep(5000);
  }
  console.log(
    "Scheduled refresh picked up the new name with the original row ID.",
  );
} finally {
  const errors = [];
  for (const remove of cleanup.reverse()) {
    try {
      await remove();
    } catch (error) {
      errors.push(error.message);
    }
  }
  if (errors.length) {
    throw new Error(
      "Contacts DV cleanup failed for " + ws.workspace_id + ": " +
        errors.join(", "),
    );
  }
}
