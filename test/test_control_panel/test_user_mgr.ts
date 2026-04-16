/**
 * test_user_mgr.ts — DV 测试用例，覆盖 control_panel 的 user_mgr.rs
 *
 * 测试 RPC 方法：
 *   user.*  —— user.list / user.get / user.create / user.update /
 *              user.update_contact / user.change_password /
 *              user.change_state / user.change_type / user.delete
 *   agent.* —— agent.list / agent.get / agent.set_msg_tunnel /
 *              agent.remove_msg_tunnel
 *
 * 通过 deno 直接运行：
 *   deno run --allow-net --allow-read --allow-env \
 *     --unsafely-ignore-certificate-errors test_user_mgr.ts
 */

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import {
  fetchUserList,
  fetchUserDetail,
  createUser,
  updateUser,
  updateUserContact,
  changeUserPassword,
  changeUserState,
  changeUserType,
  deleteUser,
  fetchAgentList,
  fetchAgentDetail,
  setAgentMsgTunnel,
  removeAgentMsgTunnel,
  type UserDetail,
  type UsersListResponse,
  type AgentsListResponse,
} from "../../src/frame/desktop/src/api/user_mgr.ts";

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

type TestResult = {
  name: string;
  ok: boolean;
  durationMs: number;
  error?: string;
};

async function runCase(
  name: string,
  fn: () => Promise<void>,
): Promise<TestResult> {
  const start = Date.now();
  try {
    await fn();
    const ms = Date.now() - start;
    console.log(`  ✓ ${name} (${ms}ms)`);
    return { name, ok: true, durationMs: ms };
  } catch (err) {
    const ms = Date.now() - start;
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`  ✗ ${name} (${ms}ms): ${msg}`);
    return { name, ok: false, durationMs: ms, error: msg };
  }
}

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

// Derived from the DV zone config (src/kernel/scheduler/src/system_config_builder.rs)
const DV_DEFAULT_AGENT_ID = "jarvis";
// Fake sha256 hashes for the test user — format doesn't matter to the backend,
// it only stores the string verbatim.
const FAKE_PW_HASH_A =
  "a1".padEnd(64, "0");
const FAKE_PW_HASH_B =
  "b2".padEnd(64, "0");

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  console.log("=== test_user_mgr: user_mgr.rs DV tests ===\n");

  // 1. Initialize as AppClient (must run as an admin account, e.g. devtest)
  const { userId: callerUserId, ownerUserId, zoneHost } =
    await initTestRuntime();
  console.log(
    `zone: ${zoneHost}, caller: ${callerUserId}, owner: ${ownerUserId}\n`,
  );

  const results: TestResult[] = [];

  // Unique username for the write-cycle tests. Keeps reruns isolated even
  // though user.delete is a soft-delete (state="deleted") in this backend.
  const testUserId = `dvtest${Date.now()}`;

  // -----------------------------------------------------------------------
  // Read-only: user.list
  // -----------------------------------------------------------------------
  console.log("[user.list]");

  let userListAtStart: UsersListResponse | null = null;

  results.push(
    await runCase("user.list returns valid response", async () => {
      const { data, error } = await fetchUserList();
      assert(!error, `fetchUserList should not error: ${error}`);
      assert(data !== null, "data should not be null");
      userListAtStart = data!;
      assert(typeof data!.total === "number", "total should be number");
      assert(Array.isArray(data!.users), "users should be array");
      assert(
        data!.total === data!.users.length,
        `total (${data!.total}) should match users.length (${data!.users.length})`,
      );
    }),
  );

  results.push(
    await runCase("user.list contains root-tier account", async () => {
      assert(userListAtStart !== null, "userListAtStart should be populated");
      // DV always provisions at least a Root account on first boot.
      assert(
        userListAtStart!.users.length >= 1,
        `expected >= 1 default user, got ${userListAtStart!.users.length}`,
      );
      const roots = userListAtStart!.users.filter(
        (u) => u.user_type === "root",
      );
      assert(roots.length >= 1, "should have at least one root account");
    }),
  );

  results.push(
    await runCase("user.list entries have required fields", async () => {
      assert(userListAtStart !== null, "userListAtStart should be populated");
      for (const u of userListAtStart!.users) {
        assert(
          typeof u.user_id === "string" && u.user_id.length > 0,
          "user_id should be non-empty string",
        );
        assert(
          typeof u.show_name === "string",
          `user '${u.user_id}' show_name should be string`,
        );
        assert(
          typeof u.user_type === "string",
          `user '${u.user_id}' user_type should be string`,
        );
        assert(
          typeof u.state === "string",
          `user '${u.user_id}' state should be string`,
        );
      }
    }),
  );

  // -----------------------------------------------------------------------
  // Read-only: user.get
  // -----------------------------------------------------------------------
  console.log("\n[user.get]");

  results.push(
    await runCase("user.get (no user_id) returns caller detail", async () => {
      const { data, error } = await fetchUserDetail();
      assert(!error, `fetchUserDetail should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        typeof data!.user_id === "string",
        "user_id should be string",
      );
      assert(
        typeof data!.show_name === "string",
        "show_name should be string",
      );
      assert(
        typeof data!.state === "string",
        "state should be string",
      );
      assert(
        typeof data!.res_pool_id === "string",
        "res_pool_id should be string",
      );
      // Calling yourself should include contact
      if (data!.contact !== undefined) {
        assert(
          typeof data!.contact === "object" && data!.contact !== null,
          "contact should be object when returned",
        );
      }
    }),
  );

  results.push(
    await runCase(
      "user.get with explicit user_id returns detail",
      async () => {
        const { data, error } = await fetchUserDetail({ userId: callerUserId });
        assert(!error, `fetchUserDetail('${callerUserId}') should not error: ${error}`);
        assert(data !== null, "data should not be null");
        // Password must never be returned
        assert(
          !("password" in (data as unknown as Record<string, unknown>)),
          "user.get response must not expose password field",
        );
      },
    ),
  );

  results.push(
    await runCase("user.get for non-existent user returns error", async () => {
      const { data, error } = await fetchUserDetail({
        userId: "nonexistent_user_xyz_12345",
      });
      assert(
        error !== null || data === null,
        "request should fail for non-existent user",
      );
    }),
  );

  // -----------------------------------------------------------------------
  // Write cycle: create → update → contact → password → state → type → delete
  // -----------------------------------------------------------------------
  console.log(`\n[user write cycle: ${testUserId}]`);

  results.push(
    await runCase("user.create creates a new user", async () => {
      const { data, error } = await createUser({
        userId: testUserId,
        passwordHash: FAKE_PW_HASH_A,
        showName: `DV Test User ${testUserId}`,
        userType: "user",
      });
      assert(!error, `createUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.user_id === testUserId,
        `user_id should be '${testUserId}', got '${data!.user_id}'`,
      );
    }),
  );

  results.push(
    await runCase("user.create rejects duplicate user_id", async () => {
      const { data, error } = await createUser({
        userId: testUserId,
        passwordHash: FAKE_PW_HASH_A,
      });
      assert(
        error !== null || data === null,
        "duplicate create should fail",
      );
    }),
  );

  results.push(
    await runCase("user.create rejects reserved username", async () => {
      const { data, error } = await createUser({
        userId: "root",
        passwordHash: FAKE_PW_HASH_A,
      });
      assert(
        error !== null || data === null,
        "create with reserved name 'root' should fail",
      );
    }),
  );

  results.push(
    await runCase("user.list now contains the new user", async () => {
      const { data, error } = await fetchUserList();
      assert(!error, `fetchUserList should not error: ${error}`);
      assert(data !== null, "data should not be null");
      const found = data!.users.find((u) => u.user_id === testUserId);
      assert(!!found, `new user '${testUserId}' should appear in user.list`);
      assert(
        found!.user_type === "user",
        `new user type should be 'user', got '${found!.user_type}'`,
      );
      assert(
        found!.state === "active",
        `new user state should be 'active', got '${found!.state}'`,
      );
    }),
  );

  let detailAfterCreate: UserDetail | null = null;
  results.push(
    await runCase("user.get returns the new user's detail", async () => {
      const { data, error } = await fetchUserDetail({ userId: testUserId });
      assert(!error, `fetchUserDetail('${testUserId}') should not error: ${error}`);
      assert(data !== null, "data should not be null");
      detailAfterCreate = data!;
      assert(
        data!.user_id === testUserId,
        `user_id mismatch: '${data!.user_id}' vs '${testUserId}'`,
      );
      assert(
        data!.show_name === `DV Test User ${testUserId}`,
        `show_name mismatch: '${data!.show_name}'`,
      );
      assert(data!.state === "active", `state should be 'active'`);
    }),
  );

  results.push(
    await runCase("user.update changes show_name", async () => {
      const newShowName = `Renamed ${testUserId}`;
      const { data, error } = await updateUser({
        userId: testUserId,
        showName: newShowName,
      });
      assert(!error, `updateUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed, error: refreshErr } = await fetchUserDetail({
        userId: testUserId,
      });
      assert(!refreshErr, `fetchUserDetail should not error: ${refreshErr}`);
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.show_name === newShowName,
        `show_name should be '${newShowName}', got '${refreshed!.show_name}'`,
      );
    }),
  );

  results.push(
    await runCase("user.update_contact writes contact settings", async () => {
      const { data, error } = await updateUserContact({
        userId: testUserId,
        did: `did:bns:${testUserId}`,
        note: "created by test_user_mgr",
        groups: ["dv-test"],
        tags: ["automation", "temporary"],
      });
      assert(!error, `updateUserContact should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(!!data!.contact, "response should echo contact");
      assert(
        data!.contact!.did === `did:bns:${testUserId}`,
        `contact.did mismatch: '${data!.contact!.did}'`,
      );
      assert(
        data!.contact!.note === "created by test_user_mgr",
        `contact.note mismatch`,
      );
      assert(
        Array.isArray(data!.contact!.groups) &&
          data!.contact!.groups!.includes("dv-test"),
        "contact.groups should contain 'dv-test'",
      );
      assert(
        Array.isArray(data!.contact!.tags) &&
          data!.contact!.tags!.length === 2,
        "contact.tags should have 2 entries",
      );
    }),
  );

  results.push(
    await runCase("user.get reflects contact update (self or admin)", async () => {
      const { data, error } = await fetchUserDetail({ userId: testUserId });
      assert(!error, `fetchUserDetail should not error: ${error}`);
      assert(data !== null, "data should not be null");
      // Caller is admin, so contact must be included.
      assert(!!data!.contact, "contact should be visible to admin caller");
      assert(
        data!.contact!.did === `did:bns:${testUserId}`,
        "contact.did should persist",
      );
    }),
  );

  results.push(
    await runCase("user.change_password updates password hash", async () => {
      const { data, error } = await changeUserPassword({
        userId: testUserId,
        newPasswordHash: FAKE_PW_HASH_B,
      });
      assert(!error, `changeUserPassword should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
    }),
  );

  results.push(
    await runCase("user.change_password rejects empty hash", async () => {
      const { data, error } = await changeUserPassword({
        userId: testUserId,
        newPasswordHash: "",
      });
      assert(
        error !== null || data === null,
        "empty password hash should be rejected",
      );
    }),
  );

  results.push(
    await runCase("user.change_state sets suspended:reason", async () => {
      const { data, error } = await changeUserState({
        userId: testUserId,
        state: "suspended:dv-test",
      });
      assert(!error, `changeUserState should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetail({ userId: testUserId });
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.state === "suspended:dv-test",
        `state should be 'suspended:dv-test', got '${refreshed!.state}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_state can re-activate the user", async () => {
      const { data, error } = await changeUserState({
        userId: testUserId,
        state: "active",
      });
      assert(!error, `changeUserState(active) should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetail({ userId: testUserId });
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.state === "active",
        `state should be 'active', got '${refreshed!.state}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_type promotes/demotes the user", async () => {
      const { data, error } = await changeUserType({
        userId: testUserId,
        userType: "limited",
      });
      assert(!error, `changeUserType should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetail({ userId: testUserId });
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.user_type === "limited",
        `user_type should be 'limited', got '${refreshed!.user_type}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_type rejects promotion to root", async () => {
      const { data, error } = await changeUserType({
        userId: testUserId,
        // Bypass the compile-time exclusion for negative testing
        userType: "root" as unknown as "user",
      });
      assert(
        error !== null || data === null,
        "promotion to root must be rejected",
      );
    }),
  );

  // -----------------------------------------------------------------------
  // Protection tests — root must stay untouched
  // -----------------------------------------------------------------------
  console.log("\n[protection]");

  results.push(
    await runCase("user.delete rejects deleting 'root'", async () => {
      const { data, error } = await deleteUser("root");
      assert(
        error !== null || data === null,
        "deleting root must be rejected",
      );
    }),
  );

  results.push(
    await runCase("user.delete rejects deleting self", async () => {
      const { data, error } = await deleteUser(callerUserId);
      assert(
        error !== null || data === null,
        "deleting self must be rejected",
      );
    }),
  );

  results.push(
    await runCase(
      "user.change_state rejects suspending root",
      async () => {
        const { data, error } = await changeUserState({
          userId: "root",
          state: "suspended:should-fail",
        });
        assert(
          error !== null || data === null,
          "suspending root must be rejected",
        );
      },
    ),
  );

  // -----------------------------------------------------------------------
  // Soft-delete — do this last so nothing else depends on the user
  // -----------------------------------------------------------------------
  console.log("\n[user.delete]");

  results.push(
    await runCase("user.delete soft-deletes the test user", async () => {
      const { data, error } = await deleteUser(testUserId);
      assert(!error, `deleteUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetail({ userId: testUserId });
      assert(
        refreshed !== null,
        "soft-deleted user should still be retrievable",
      );
      assert(
        refreshed!.state === "deleted",
        `state should be 'deleted', got '${refreshed!.state}'`,
      );
    }),
  );

  // -----------------------------------------------------------------------
  // Read-only: agent.list / agent.get
  // -----------------------------------------------------------------------
  console.log("\n[agent.list / agent.get]");

  let agentList: AgentsListResponse | null = null;

  results.push(
    await runCase("agent.list returns valid response", async () => {
      const { data, error } = await fetchAgentList();
      assert(!error, `fetchAgentList should not error: ${error}`);
      assert(data !== null, "data should not be null");
      agentList = data!;
      assert(typeof data!.total === "number", "total should be number");
      assert(Array.isArray(data!.agents), "agents should be array");
      assert(
        data!.total === data!.agents.length,
        `total (${data!.total}) should match agents.length (${data!.agents.length})`,
      );
    }),
  );

  results.push(
    await runCase(
      `agent.list includes DV default agent (${DV_DEFAULT_AGENT_ID})`,
      async () => {
        assert(agentList !== null, "agentList should be populated");
        const found = agentList!.agents.find(
          (a) => a.agent_id === DV_DEFAULT_AGENT_ID,
        );
        assert(
          !!found,
          `DV default agent '${DV_DEFAULT_AGENT_ID}' should be in agent.list`,
        );
      },
    ),
  );

  results.push(
    await runCase(
      `agent.get returns doc for ${DV_DEFAULT_AGENT_ID}`,
      async () => {
        const { data, error } = await fetchAgentDetail(DV_DEFAULT_AGENT_ID);
        assert(!error, `fetchAgentDetail should not error: ${error}`);
        assert(data !== null, "data should not be null");
        assert(
          data!.agent_id === DV_DEFAULT_AGENT_ID,
          `agent_id should be '${DV_DEFAULT_AGENT_ID}'`,
        );
        // jarvis has an `id` (DID) field in its doc
        assert(
          typeof (data as Record<string, unknown>).id === "string",
          "agent doc should carry an 'id' (DID) field",
        );
        // settings is optional but, when present, must be an object
        if (data!.settings !== undefined) {
          assert(
            typeof data!.settings === "object" && data!.settings !== null,
            "settings should be object when present",
          );
        }
      },
    ),
  );

  results.push(
    await runCase("agent.get for non-existent agent returns error", async () => {
      const { data, error } = await fetchAgentDetail("nonexistent_agent_xyz");
      assert(
        error !== null || data === null,
        "request should fail for non-existent agent",
      );
    }),
  );

  // -----------------------------------------------------------------------
  // agent.set_msg_tunnel / agent.remove_msg_tunnel
  // Use a throwaway platform name unique to this run so we don't clobber
  // any real-world binding.
  // -----------------------------------------------------------------------
  console.log("\n[agent.set_msg_tunnel / agent.remove_msg_tunnel]");

  const testPlatform = `dvtest_${Date.now()}`;

  results.push(
    await runCase("agent.set_msg_tunnel adds a new binding", async () => {
      const { data, error } = await setAgentMsgTunnel({
        agentId: DV_DEFAULT_AGENT_ID,
        platform: testPlatform,
        accountId: "dv-test-account",
        displayId: "DV Test Display",
        tunnelId: "dv-test-tunnel",
        meta: { source: "test_user_mgr" },
      });
      assert(!error, `setAgentMsgTunnel should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.agent_id === DV_DEFAULT_AGENT_ID,
        "agent_id should round-trip",
      );
      assert(
        data!.platform === testPlatform,
        "platform should round-trip",
      );
      assert(
        typeof data!.total_bindings === "number" && data!.total_bindings! >= 1,
        "total_bindings should be >= 1",
      );
    }),
  );

  results.push(
    await runCase("agent.set_msg_tunnel is idempotent per platform", async () => {
      // Re-setting the same platform should replace, not duplicate.
      const { data, error } = await setAgentMsgTunnel({
        agentId: DV_DEFAULT_AGENT_ID,
        platform: testPlatform,
        accountId: "dv-test-account-v2",
      });
      assert(!error, `setAgentMsgTunnel (update) should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
    }),
  );

  results.push(
    await runCase("agent.remove_msg_tunnel removes the binding", async () => {
      const { data, error } = await removeAgentMsgTunnel({
        agentId: DV_DEFAULT_AGENT_ID,
        platform: testPlatform,
      });
      assert(!error, `removeAgentMsgTunnel should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.platform === testPlatform,
        "platform should round-trip",
      );
    }),
  );

  results.push(
    await runCase(
      "agent.remove_msg_tunnel fails for already-removed platform",
      async () => {
        const { data, error } = await removeAgentMsgTunnel({
          agentId: DV_DEFAULT_AGENT_ID,
          platform: testPlatform,
        });
        assert(
          error !== null || data === null,
          "removing an already-removed platform should fail",
        );
      },
    ),
  );

  // -----------------------------------------------------------------------
  // Summary
  // -----------------------------------------------------------------------
  console.log("\n=== Summary ===");
  const passed = results.filter((r) => r.ok).length;
  const failed = results.filter((r) => !r.ok).length;
  const totalMs = results.reduce((sum, r) => sum + r.durationMs, 0);
  console.log(
    `${passed} passed, ${failed} failed, ${results.length} total (${totalMs}ms)`,
  );

  if (failed > 0) {
    console.log("\nFailed tests:");
    for (const r of results.filter((r) => !r.ok)) {
      console.log(`  - ${r.name}: ${r.error}`);
    }
    Deno.exit(1);
  }

  console.log("\nAll tests passed!");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  Deno.exit(2);
});
