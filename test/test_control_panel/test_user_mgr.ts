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

import { buckyos } from "buckyos";
import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import {
  fetchUserList,
  fetchUserDetail,
  createUser,
  updateUser,
  updateUserContact,
  fetchUserProfile,
  setUserProfile,
  setUserMsgTunnel,
  removeUserMsgTunnel,
  createUserInvite,
  fetchUserInvite,
  changeUserPassword,
  changeUserState,
  changeUserType,
  deleteUser,
  fetchAgentList,
  fetchAgentDetail,
  createAgent,
  updateAgent,
  deleteAgent,
  fetchAgentProfile,
  setAgentProfile,
  setAgentMsgTunnel,
  removeAgentMsgTunnel,
  type UserDetail,
  type UsersListResponse,
  type AgentsListResponse,
  type UserContactSettings,
  type UserType,
  type SimpleOkResponse,
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
    console.error(`  ❌ ${name} (${ms}ms): ${msg}`);
    return { name, ok: false, durationMs: ms, error: msg };
  }
}

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function requireRecord(
  value: unknown,
  label: string,
): Record<string, unknown> {
  const record = asRecord(value);
  assert(record !== null, `${label} should be an object`);
  return record!;
}

function localSystemContactFromDetail(
  detail: UserDetail,
): UserContactSettings | null {
  const localProfile = asRecord(detail.local_profile);
  const privateExtra = asRecord(localProfile?.private_extra);
  const systemContact = asRecord(privateExtra?.system_contact);
  return systemContact as UserContactSettings | null;
}

function visibleSystemContactFromDetail(
  detail: UserDetail,
): UserContactSettings | null {
  return detail.contact ?? localSystemContactFromDetail(detail);
}

// Derived from the DV zone config (src/kernel/scheduler/src/system_config_builder.rs)
const DV_DEFAULT_AGENT_ID = "jarvis";
// Fake sha256 hash for change_password — format doesn't matter to the backend,
// it only stores the string verbatim (no login happens after this point).
const FAKE_PW_HASH_B =
  "b2".padEnd(64, "0");

function getEnv(name: string, fallback: string): string {
  const val = Deno.env.get(name);
  return typeof val === "string" && val.trim().length > 0 ? val.trim() : fallback;
}

// Admin credentials used to obtain a sudo grant. The DV zone provisions the
// owner/admin account ("devtest") with the password "bucky2025"; the stored
// hash o8XyToejrbCYou84h/VkF4Tht0BeQQbuX3XKG+8+GQ4= equals
// hashPassword("devtest", "bucky2025") — see
// src/kernel/buckyos-api/src/test_config.rs (ADMIN_PASSWORD_HASH).
const DV_ADMIN_USER = getEnv("BUCKYOS_TEST_ADMIN_USER", "devtest");
const DV_ADMIN_PASSWORD = getEnv("BUCKYOS_TEST_ADMIN_PASSWORD", "bucky2025");
// appid must match the token appid the gateway accepts for the AppClient — the
// test runtime logs in as "buckycli" (see buckyos_client.ts).
const TEST_APP_ID = getEnv("BUCKYOS_TEST_APP_ID", "buckycli");

// ---------------------------------------------------------------------------
// Sudo / local-account auth helpers
//
// Sensitive writes under config/users/* (user.create) require an *elevated*
// sudo session token, not the regular admin login token. The flow mirrors the
// desktop sudo dialog (src/frame/desktop/src/components/sudo.tsx):
//   1. hashPassword(user, pw, nonce) -> verify-hub.sudo_by_password -> sudo token
//   2. call control-panel user.create with that sudo token as the session token
// ---------------------------------------------------------------------------

// The SDK's hashPassword typings are loose (nonce typed as `null`, return as a
// union). The runtime always takes an optional numeric nonce and returns the
// base64 string hash, so narrow it here.
const hashPassword = buckyos.hashPassword as unknown as (
  username: string,
  password: string,
  nonce?: number,
) => string;

// Typed view of the SDK rpc client used here.
//  - The managed client from buckyos.getServiceRpcClient supports a per-call
//    session-token override (3rd arg) — required to drive a single elevated
//    (sudo) request without polluting the AppClient runtime's token. (Earlier
//    websdk versions silently dropped this; the current one honors it.)
//  - For verify-hub login/sudo, which are unauthenticated, we use a raw client
//    so no caller token is attached at all.
type ManagedRpcClient = {
  call(
    method: string,
    params: Record<string, unknown>,
    options?: { sessionToken?: string },
  ): Promise<unknown>;
};
type RawRpcClient = {
  setSeq(seq: number): void;
  call(method: string, params: Record<string, unknown>): Promise<unknown>;
};
const KRpcClient = buckyos.kRPCClient as unknown as new (
  url: string,
  token?: string | null,
  seq?: number,
) => RawRpcClient;

/** Absolute gateway URL for a zone service (AppClient mode has no page origin). */
function serviceUrl(service: string): string {
  const cfg = buckyos.getBuckyOSConfig() as
    | { zoneHost?: string; defaultProtocol?: string }
    | null;
  const host = cfg?.zoneHost;
  if (!host) throw new Error("zoneHost is not configured on the runtime");
  const protocol = cfg?.defaultProtocol ?? "https://";
  return `${protocol}${host}/kapi/${service}`;
}

/** Request an elevated sudo session token from verify-hub via password. */
async function obtainSudoToken(
  username: string,
  password: string,
  appId: string,
): Promise<string> {
  const nonce = Date.now();
  const passwordHash = hashPassword(username, password, nonce);
  // verify-hub login/sudo endpoints are unauthenticated — use a raw client.
  const rpc = new KRpcClient(serviceUrl("verify-hub"), null, nonce);
  const res = (await rpc.call("sudo_by_password", {
    username,
    password: passwordHash,
    appid: appId,
    login_nonce: nonce,
  })) as { session_token?: unknown };
  const token = typeof res?.session_token === "string" ? res.session_token : "";
  if (!token) {
    throw new Error("sudo_by_password did not return a session_token");
  }
  return token;
}

/** Log in a local (password) account via verify-hub. Returns the account info. */
async function loginByPassword(
  username: string,
  password: string,
  appId: string,
): Promise<{
  session_token?: string;
  user_id?: string;
  user_info?: { user_id?: string; user_type?: string; show_name?: string };
}> {
  const nonce = Date.now();
  const passwordHash = hashPassword(username, password, nonce);
  const rpc = new KRpcClient(serviceUrl("verify-hub"), null, nonce);
  return (await rpc.call("login_by_password", {
    type: "password",
    username,
    password: passwordHash,
    appid: appId,
    login_nonce: nonce,
  })) as Awaited<ReturnType<typeof loginByPassword>>;
}

async function callControlPanelWithToken<T>(
  token: string,
  method: string,
  params: Record<string, unknown> = {},
): Promise<{ data: T | null; error: unknown }> {
  try {
    const rpc = buckyos.getServiceRpcClient(
      "control-panel",
    ) as unknown as ManagedRpcClient;
    const result = (await rpc.call(method, params, {
      sessionToken: token,
    })) as T;
    if (!result || typeof result !== "object") {
      throw new Error(`Invalid ${method} response`);
    }
    return { data: result, error: null };
  } catch (error) {
    return { data: null, error };
  }
}

/**
 * Call control-panel `user.create` with an explicit session token (the sudo
 * grant), using the managed client's per-call session-token override. This is
 * the pattern the desktop UI should use to perform a sudo'd operation without
 * mutating the AppClient runtime's own token.
 */
async function createUserWithToken(
  token: string,
  input: {
    userId: string;
    passwordHash: string;
    showName?: string;
    userType?: Exclude<UserType, "root">;
    allowPasswordChange?: boolean;
  },
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  const params: Record<string, unknown> = {
    user_id: input.userId,
    password_hash: input.passwordHash,
  };
  if (input.showName !== undefined) params.show_name = input.showName;
  if (input.userType !== undefined) params.user_type = input.userType;
  if (input.allowPasswordChange !== undefined) {
    params.allow_password_change = input.allowPasswordChange;
  }
  return callControlPanelWithToken<SimpleOkResponse>(
    token,
    "user.create",
    params,
  );
}

async function fetchUserDetailWithToken(
  token: string,
  options: { userId?: string } = {},
): Promise<{ data: UserDetail | null; error: unknown }> {
  const params: Record<string, unknown> = {};
  if (options.userId) params.user_id = options.userId;
  return callControlPanelWithToken<UserDetail>(token, "user.get", params);
}

async function updateUserWithToken(
  token: string,
  input: { userId?: string; showName?: string },
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  const params: Record<string, unknown> = {};
  if (input.userId) params.user_id = input.userId;
  if (input.showName !== undefined) params.show_name = input.showName;
  return callControlPanelWithToken<SimpleOkResponse>(
    token,
    "user.update",
    params,
  );
}

async function updateUserContactWithToken(
  token: string,
  input: {
    userId?: string;
    did?: string;
    note?: string;
    groups?: string[];
    tags?: string[];
  },
): Promise<{
  data: (SimpleOkResponse & { contact?: UserContactSettings }) | null;
  error: unknown;
}> {
  const params: Record<string, unknown> = {};
  if (input.userId) params.user_id = input.userId;
  if (input.did !== undefined) params.did = input.did;
  if (input.note !== undefined) params.note = input.note;
  if (input.groups !== undefined) params.groups = input.groups;
  if (input.tags !== undefined) params.tags = input.tags;
  return callControlPanelWithToken<
    SimpleOkResponse & { contact?: UserContactSettings }
  >(token, "user.update_contact", params);
}

async function fetchUserProfileWithToken(
  token: string,
  userId?: string,
): Promise<Awaited<ReturnType<typeof fetchUserProfile>>> {
  const params: Record<string, unknown> = {};
  if (userId) params.user_id = userId;
  return callControlPanelWithToken(token, "user.profile.get", params);
}

async function setUserProfileWithToken(
  token: string,
  input: Parameters<typeof setUserProfile>[0],
): Promise<Awaited<ReturnType<typeof setUserProfile>>> {
  const params: Record<string, unknown> = {};
  if (input.userId) params.user_id = input.userId;
  if (input.profile !== undefined) params.profile = input.profile;
  if (input.name !== undefined) params.name = input.name;
  if (input.displayName !== undefined) params.display_name = input.displayName;
  if (input.avatar !== undefined) params.avatar = input.avatar;
  if (input.avatarUrl !== undefined) params.avatar = input.avatarUrl;
  if (input.headline !== undefined) params.headline = input.headline;
  if (input.title !== undefined) params.title = input.title;
  if (input.bio !== undefined) params.bio = input.bio;
  if (input.location !== undefined) params.location = input.location;
  if (input.organization !== undefined) params.organization = input.organization;
  if (input.birthday !== undefined) params.birthday = input.birthday;
  if (input.bkgImage !== undefined) params.bkg_image = input.bkgImage;
  if (input.tags !== undefined) params.tags = input.tags;
  if (input.website !== undefined) params.website = input.website;
  if (input.email !== undefined) params.email = input.email;
  if (input.phone !== undefined) params.phone = input.phone;
  if (input.privateExtra !== undefined) params.private_extra = input.privateExtra;
  if (input.extra !== undefined) params.extra = input.extra;
  return callControlPanelWithToken(token, "user.profile.set", params);
}

async function setUserMsgTunnelWithToken(
  token: string,
  input: Parameters<typeof setUserMsgTunnel>[0],
): Promise<Awaited<ReturnType<typeof setUserMsgTunnel>>> {
  const params: Record<string, unknown> = {
    platform: input.platform,
    account_id: input.accountId,
  };
  if (input.userId) params.user_id = input.userId;
  if (input.displayId !== undefined) params.display_id = input.displayId;
  if (input.tunnelId !== undefined) params.tunnel_id = input.tunnelId;
  if (input.status !== undefined) params.status = input.status;
  if (input.lastSyncAt !== undefined) params.last_sync_at = input.lastSyncAt;
  if (input.meta !== undefined) params.meta = input.meta;
  return callControlPanelWithToken(token, "user.set_msg_tunnel", params);
}

async function removeUserMsgTunnelWithToken(
  token: string,
  input: Parameters<typeof removeUserMsgTunnel>[0],
): Promise<Awaited<ReturnType<typeof removeUserMsgTunnel>>> {
  const params: Record<string, unknown> = { platform: input.platform };
  if (input.userId) params.user_id = input.userId;
  return callControlPanelWithToken(token, "user.remove_msg_tunnel", params);
}

async function changeUserPasswordWithToken(
  token: string,
  input: { userId?: string; newPasswordHash: string },
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  const params: Record<string, unknown> = {
    new_password_hash: input.newPasswordHash,
  };
  if (input.userId) params.user_id = input.userId;
  return callControlPanelWithToken<SimpleOkResponse>(
    token,
    "user.change_password",
    params,
  );
}

async function changeUserStateWithToken(
  token: string,
  input: Parameters<typeof changeUserState>[0],
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  return callControlPanelWithToken<SimpleOkResponse>(
    token,
    "user.change_state",
    {
      user_id: input.userId,
      state: input.state,
    },
  );
}

async function changeUserTypeWithToken(
  token: string,
  input: Parameters<typeof changeUserType>[0],
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  return callControlPanelWithToken<SimpleOkResponse>(
    token,
    "user.change_type",
    {
      user_id: input.userId,
      user_type: input.userType,
    },
  );
}

async function deleteUserWithToken(
  token: string,
  userId: string,
): Promise<{ data: SimpleOkResponse | null; error: unknown }> {
  return callControlPanelWithToken<SimpleOkResponse>(token, "user.delete", {
    user_id: userId,
  });
}

function createUserInviteWithToken(
  token: string,
  input: Parameters<typeof createUserInvite>[0],
): ReturnType<typeof createUserInvite> {
  const params: Record<string, unknown> = {};
  if (input.inviteId !== undefined) params.invite_id = input.inviteId;
  if (input.userId !== undefined) params.user_id = input.userId;
  if (input.targetDid !== undefined) params.target_did = input.targetDid;
  if (input.showName !== undefined) params.show_name = input.showName;
  if (input.userType !== undefined) params.user_type = input.userType;
  if (input.expiresAt !== undefined) params.expires_at = input.expiresAt;
  if (input.ttlSecs !== undefined) params.ttl_secs = input.ttlSecs;
  if (input.groups !== undefined) params.groups = input.groups;
  if (input.appIds !== undefined) params.app_ids = input.appIds;
  return callControlPanelWithToken(token, "user.invite.create", params);
}

function createAgentWithToken(
  token: string,
  input: Parameters<typeof createAgent>[0],
): ReturnType<typeof createAgent> {
  const params: Record<string, unknown> = { agent_id: input.agentId };
  if (input.displayName !== undefined) params.display_name = input.displayName;
  if (input.ownerUserId !== undefined) params.owner_user_id = input.ownerUserId;
  if (input.agentDid !== undefined) params.agent_did = input.agentDid;
  if (input.description !== undefined) params.description = input.description;
  if (input.profile !== undefined) params.profile = input.profile;
  if (input.settings !== undefined) params.settings = input.settings;
  return callControlPanelWithToken(token, "agent.create", params);
}

function deleteAgentWithToken(
  token: string,
  agentId: string,
): ReturnType<typeof deleteAgent> {
  return callControlPanelWithToken(token, "agent.delete", {
    agent_id: agentId,
  });
}

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
        typeof data!.state === "string",
        "state should be string",
      );
      assert(
        typeof data!.res_pool_id === "string",
        "res_pool_id should be string",
      );
      requireRecord(data!.profile, "profile");
      if (data!.local_profile !== undefined && data!.local_profile !== null) {
        requireRecord(data!.local_profile, "local_profile");
      }
      // Calling yourself should include contact
      const contact = visibleSystemContactFromDetail(data!);
      if (contact !== null) {
        assert(
          typeof contact === "object" && contact !== null,
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

  // The new account is a *local* (password) account — created with a username
  // and a password hash, logged in via verify-hub.login_by_password. It is not
  // an account backed by an independent BNS identity.
  //
  // IMPORTANT: the stored password hash must be hashPassword(user, pw) with NO
  // nonce (the "org" hash). verify-hub re-salts it with the per-login nonce
  // (src/kernel/verify_hub/src/main.rs::validate_password_login), so a fake
  // hash here would store fine but make login impossible.
  const NEW_USER_PASSWORD = "DvTest-Pass-2025";
  const newUserPasswordHash = hashPassword(testUserId, NEW_USER_PASSWORD);

  let sudoToken: string | null = null;
  let newUserToken: string | null = null;
  let newUserSudoToken: string | null = null;
  const requireSudoToken = (): string => {
    assert(sudoToken !== null, "sudoToken should have been granted");
    return sudoToken!;
  };
  const requireNewUserToken = (): string => {
    assert(newUserToken !== null, "newUserToken should be populated");
    return newUserToken!;
  };
  const requireNewUserSudoToken = (): string => {
    assert(newUserSudoToken !== null, "newUserSudoToken should be populated");
    return newUserSudoToken!;
  };

  results.push(
    await runCase("sudo: admin obtains an elevated sudo token", async () => {
      sudoToken = await obtainSudoToken(
        DV_ADMIN_USER,
        DV_ADMIN_PASSWORD,
        TEST_APP_ID,
      );
      assert(
        typeof sudoToken === "string" && sudoToken.length > 0,
        "sudo token should be a non-empty string",
      );
    }),
  );

  results.push(
    await runCase("user.create is rejected without sudo", async () => {
      // Uses the regular (non-elevated) admin session token. Sensitive writes
      // under config/users/* require sudo, so this must fail. A distinct
      // user_id keeps this isolated from the real create below.
      const { data, error } = await createUser({
        userId: `${testUserId}nosudo`,
        passwordHash: newUserPasswordHash,
        showName: "DV Test User (no sudo)",
        userType: "user",
      });
      assert(
        error !== null || data === null,
        "create without sudo must be rejected",
      );
    }),
  );

  results.push(
    await runCase("user.create with sudo creates a new local user", async () => {
      assert(sudoToken !== null, "sudoToken should have been granted");
      const { data, error } = await createUserWithToken(sudoToken!, {
        userId: testUserId,
        passwordHash: newUserPasswordHash,
        showName: `DV Test User ${testUserId}`,
        userType: "user",
      });
      assert(!error, `createUser (sudo) should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.user_id === testUserId,
        `user_id should be '${testUserId}', got '${data!.user_id}'`,
      );
    }),
  );

  results.push(
    await runCase(
      "new local user can log in with username + password",
      async () => {
        const res = await loginByPassword(
          testUserId,
          NEW_USER_PASSWORD,
          TEST_APP_ID,
        );
        const sessionToken = res.session_token;
        if (typeof sessionToken !== "string" || sessionToken.length === 0) {
          throw new Error("login should return a non-empty session_token");
        }
        newUserToken = sessionToken;
        const loggedInUserId = res.user_info?.user_id ?? res.user_id;
        assert(
          loggedInUserId === testUserId,
          `logged-in user_id should be '${testUserId}', got '${loggedInUserId}'`,
        );
      },
    ),
  );

  results.push(
    await runCase("new local user can obtain a sudo token", async () => {
      newUserSudoToken = await obtainSudoToken(
        testUserId,
        NEW_USER_PASSWORD,
        TEST_APP_ID,
      );
      assert(
        typeof newUserSudoToken === "string" && newUserSudoToken.length > 0,
        "new user sudo token should be a non-empty string",
      );
    }),
  );

  results.push(
    await runCase("user.create rejects duplicate user_id (with sudo)", async () => {
      assert(sudoToken !== null, "sudoToken should have been granted");
      const { data, error } = await createUserWithToken(sudoToken!, {
        userId: testUserId,
        passwordHash: newUserPasswordHash,
      });
      assert(
        error !== null || data === null,
        "duplicate create should fail",
      );
    }),
  );

  results.push(
    await runCase("user.create rejects reserved username (with sudo)", async () => {
      assert(sudoToken !== null, "sudoToken should have been granted");
      const { data, error } = await createUserWithToken(sudoToken!, {
        userId: "root",
        passwordHash: newUserPasswordHash,
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

  results.push(
    await runCase("new user can read their own detail", async () => {
      const { data, error } = await fetchUserDetailWithToken(
        requireNewUserToken(),
      );
      assert(!error, `fetchUserDetail (as new user) should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.user_id === testUserId,
        `user_id mismatch: '${data!.user_id}' vs '${testUserId}'`,
      );
      const profile = requireRecord(data!.profile, "profile");
      const localProfile = requireRecord(data!.local_profile, "local_profile");
      assert(
        profile.display_name === `DV Test User ${testUserId}`,
        `profile.display_name mismatch: '${String(profile.display_name)}'`,
      );
      assert(
        localProfile.display_name === `DV Test User ${testUserId}`,
        `local_profile.display_name mismatch: '${String(localProfile.display_name)}'`,
      );
      assert(data!.state === "active", `state should be 'active'`);
      const systemContact = localSystemContactFromDetail(data!);
      assert(
        systemContact !== null,
        "local_profile.private_extra.system_contact should exist",
      );
      assert(
        Array.isArray(systemContact!.groups) &&
          systemContact!.groups.includes("users"),
        "system_contact.groups should include default 'users'",
      );
    }),
  );

  results.push(
    await runCase("new user can update their profile display_name", async () => {
      const newShowName = `Renamed ${testUserId}`;
      const { data, error } = await updateUserWithToken(requireNewUserToken(), {
        showName: newShowName,
      });
      assert(!error, `updateUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed, error: refreshErr } =
        await fetchUserDetailWithToken(requireNewUserToken());
      assert(!refreshErr, `fetchUserDetail should not error: ${refreshErr}`);
      assert(refreshed !== null, "refreshed detail should not be null");
      const profile = requireRecord(refreshed!.profile, "profile");
      const localProfile = requireRecord(
        refreshed!.local_profile,
        "local_profile",
      );
      assert(
        profile.display_name === newShowName,
        `profile.display_name should be '${newShowName}', got '${String(profile.display_name)}'`,
      );
      assert(
        localProfile.display_name === newShowName,
        `local_profile.display_name should be '${newShowName}', got '${String(localProfile.display_name)}'`,
      );

      const { data: listed, error: listErr } = await fetchUserList();
      assert(!listErr, `fetchUserList should not error: ${listErr}`);
      assert(listed !== null, "listed users should not be null");
      const found = listed!.users.find((u) => u.user_id === testUserId);
      assert(!!found, `new user '${testUserId}' should appear in user.list`);
      assert(
        found!.show_name === newShowName,
        `user.list show_name should be '${newShowName}', got '${found!.show_name}'`,
      );
    }),
  );

  results.push(
    await runCase("new user can write contact settings", async () => {
      const { data, error } = await updateUserContactWithToken(
        requireNewUserToken(),
        {
          did: `did:bns:${testUserId}`,
          note: "created by test_user_mgr",
          groups: ["dv-test"],
          tags: ["automation", "temporary"],
        },
      );
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

  let contactAfterUpdate: UserContactSettings | null = null;

  results.push(
    await runCase("new user can read back contact update", async () => {
      const { data, error } = await fetchUserDetailWithToken(
        requireNewUserToken(),
      );
      assert(!error, `fetchUserDetail should not error: ${error}`);
      assert(data !== null, "data should not be null");
      // Caller is the user, so contact must be included.
      assert(!!data!.contact, "contact should be visible to self caller");
      const localSystemContact = localSystemContactFromDetail(data!);
      assert(
        localSystemContact !== null,
        "local_profile.private_extra.system_contact should persist",
      );
      assert(
        data!.contact!.did === `did:bns:${testUserId}`,
        "contact.did should persist",
      );
      assert(
        localSystemContact!.did === data!.contact!.did,
        "compat contact should match local_profile.private_extra.system_contact",
      );
      assert(
        Array.isArray(localSystemContact!.groups) &&
          localSystemContact!.groups.includes("users") &&
          localSystemContact!.groups.includes("dv-test"),
        "system_contact.groups should include default and updated groups",
      );
      contactAfterUpdate = localSystemContact;
    }),
  );

  // -----------------------------------------------------------------------
  // user.profile.get / user.profile.set
  // -----------------------------------------------------------------------
  console.log(`\n[user.profile.get / user.profile.set: ${testUserId}]`);

  results.push(
    await runCase("new user can write local profile fields", async () => {
      assert(
        contactAfterUpdate !== null,
        "contactAfterUpdate should be populated before profile.set",
      );
      const { data, error } = await setUserProfileWithToken(
        requireNewUserToken(),
        {
          displayName: `DV ${testUserId}`,
          headline: "DV profile headline",
          title: "QA Bot",
          bio: "created by test_user_mgr",
          location: "DV Zone",
          organization: "BuckyOS DV",
          birthday: "2025-01-01",
          bkgImage: "https://buckyos.io/dv-bg.png",
          tags: ["dv-profile", "automation"],
          website: "https://buckyos.io",
          email: "dv-user@example.com",
          phone: "+15550101000",
          privateExtra: {
            system_contact: contactAfterUpdate,
            dv_test_marker: {
              source: "test_user_mgr",
              user_id: testUserId,
            },
          },
        },
      );
      assert(!error, `setUserProfile should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      const localProfile = requireRecord(data!.profile, "set response profile");
      assert(
        localProfile.display_name === `DV ${testUserId}`,
        "set response profile.display_name should reflect update",
      );
      const privateExtra = requireRecord(
        localProfile.private_extra,
        "set response profile.private_extra",
      );
      assert(
        asRecord(privateExtra.system_contact) !== null,
        "set response profile.private_extra.system_contact should be preserved",
      );
    }),
  );

  results.push(
    await runCase("new user can read the written profile", async () => {
      const { data, error } = await fetchUserProfileWithToken(
        requireNewUserToken(),
      );
      assert(!error, `fetchUserProfile should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.user_id === testUserId,
        `user_id should be '${testUserId}', got '${data!.user_id}'`,
      );
      const profile = (data!.profile ?? {}) as Record<string, unknown>;
      const localProfile = requireRecord(data!.local_profile, "local_profile");
      // The merged profile (local + DID) must reflect the local write.
      assert(
        profile.display_name === `DV ${testUserId}`,
        `profile.display_name should round-trip, got '${String(profile.display_name)}'`,
      );
      assert(
        profile.headline === "DV profile headline",
        `profile.headline should round-trip, got '${String(profile.headline)}'`,
      );
      assert(
        profile.title === "QA Bot",
        `profile.title should be 'QA Bot', got '${String(profile.title)}'`,
      );
      assert(
        profile.bio === "created by test_user_mgr",
        `profile.bio should round-trip, got '${String(profile.bio)}'`,
      );
      assert(
        profile.location === "DV Zone",
        `profile.location should round-trip, got '${String(profile.location)}'`,
      );
      assert(
        profile.organization === "BuckyOS DV",
        `profile.organization should round-trip, got '${String(profile.organization)}'`,
      );
      assert(
        profile.birthday === "2025-01-01",
        `profile.birthday should round-trip, got '${String(profile.birthday)}'`,
      );
      assert(
        profile.bkg_image === "https://buckyos.io/dv-bg.png",
        `profile.bkg_image should round-trip, got '${String(profile.bkg_image)}'`,
      );
      assert(
        Array.isArray(profile.tags) &&
          profile.tags.includes("dv-profile") &&
          profile.tags.includes("automation"),
        "profile.tags should round-trip",
      );
      const links = requireRecord(profile.links, "profile.links");
      const website = requireRecord(links.website, "profile.links.website");
      assert(
        website.url === "https://buckyos.io",
        `profile.links.website.url should round-trip, got '${String(website.url)}'`,
      );
      assert(
        profile.email === "dv-user@example.com",
        `profile.email should round-trip, got '${String(profile.email)}'`,
      );
      assert(
        profile.phone === "+15550101000",
        `profile.phone should round-trip, got '${String(profile.phone)}'`,
      );
      const privateExtra = requireRecord(
        localProfile.private_extra,
        "local_profile.private_extra",
      );
      const systemContact = requireRecord(
        privateExtra.system_contact,
        "local_profile.private_extra.system_contact",
      ) as UserContactSettings;
      assert(
        systemContact.did === `did:bns:${testUserId}`,
        "system_contact.did should survive user.profile.set",
      );
      const marker = requireRecord(
        privateExtra.dv_test_marker,
        "local_profile.private_extra.dv_test_marker",
      );
      assert(
        marker.source === "test_user_mgr" && marker.user_id === testUserId,
        "private_extra.dv_test_marker should round-trip",
      );
      assert(
        asRecord(profile.private_extra) === null,
        "merged public profile should not expose private_extra",
      );
      assert(
        data!.did_profile === null || asRecord(data!.did_profile) !== null,
        "did_profile should be null or an object",
      );
    }),
  );

  // -----------------------------------------------------------------------
  // user.set_msg_tunnel / user.remove_msg_tunnel
  // -----------------------------------------------------------------------
  console.log(
    `\n[user.set_msg_tunnel / user.remove_msg_tunnel: ${testUserId}]`,
  );

  const userPlatform = `dvtest_user_${Date.now()}`;

  results.push(
    await runCase("new user can add a message tunnel binding", async () => {
      const { data, error } = await setUserMsgTunnelWithToken(
        requireNewUserToken(),
        {
          platform: userPlatform,
          accountId: "dv-user-account",
          displayId: "DV User Display",
          tunnelId: "dv-user-tunnel",
          meta: { source: "test_user_mgr" },
        },
      );
      assert(!error, `setUserMsgTunnel should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.platform === userPlatform,
        "platform should round-trip",
      );
      assert(
        typeof data!.total_bindings === "number" && data!.total_bindings! >= 1,
        "total_bindings should be >= 1",
      );
    }),
  );

  results.push(
    await runCase(
      "user.set_msg_tunnel binding is visible via user.get contact",
      async () => {
        const { data, error } = await fetchUserDetailWithToken(
          requireNewUserToken(),
        );
        assert(!error, `fetchUserDetail should not error: ${error}`);
        assert(data !== null, "data should not be null");
        const localSystemContact = localSystemContactFromDetail(data!);
        assert(
          localSystemContact !== null,
          "local_profile.private_extra.system_contact should be present",
        );
        const contact = visibleSystemContactFromDetail(data!);
        assert(contact !== null, "contact should be visible to self caller");
        const bindings = localSystemContact!.bindings ?? [];
        const found = bindings.find((b) => b.platform === userPlatform);
        assert(
          !!found,
          `binding for '${userPlatform}' should appear in system_contact.bindings`,
        );
        assert(
          found!.account_id === "dv-user-account",
          "binding account_id should round-trip",
        );
        assert(
          contact!.bindings?.some((b) => b.platform === userPlatform) === true,
          "compat contact.bindings should include the same binding",
        );
      },
    ),
  );

  results.push(
    await runCase("new user can remove the message tunnel binding", async () => {
      const { data, error } = await removeUserMsgTunnelWithToken(
        requireNewUserToken(),
        {
          platform: userPlatform,
        },
      );
      assert(!error, `removeUserMsgTunnel should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.platform === userPlatform,
        "platform should round-trip",
      );

      const { data: refreshed, error: refreshErr } =
        await fetchUserDetailWithToken(requireNewUserToken());
      assert(!refreshErr, `fetchUserDetail should not error: ${refreshErr}`);
      assert(refreshed !== null, "refreshed detail should not be null");
      const bindings =
        localSystemContactFromDetail(refreshed!)?.bindings ?? [];
      assert(
        !bindings.some((b) => b.platform === userPlatform),
        "removed binding should no longer exist in system_contact.bindings",
      );
    }),
  );

  results.push(
    await runCase("new user can update their password hash", async () => {
      const { data, error } = await changeUserPasswordWithToken(
        requireNewUserSudoToken(),
        {
          newPasswordHash: FAKE_PW_HASH_B,
        },
      );
      assert(!error, `changeUserPassword should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
    }),
  );

  results.push(
    await runCase("new user password change rejects empty hash", async () => {
      const { data, error } = await changeUserPasswordWithToken(
        requireNewUserSudoToken(),
        {
          newPasswordHash: "",
        },
      );
      assert(
        error !== null || data === null,
        "empty password hash should be rejected",
      );
    }),
  );

  results.push(
    await runCase("user.change_state sets suspended:reason", async () => {
      const { data, error } = await changeUserStateWithToken(
        requireSudoToken(),
        {
          userId: testUserId,
          state: "suspended:dv-test",
        },
      );
      assert(!error, `changeUserState should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetailWithToken(
        requireSudoToken(),
        { userId: testUserId },
      );
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.state === "suspended:dv-test",
        `state should be 'suspended:dv-test', got '${refreshed!.state}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_state can re-activate the user", async () => {
      const { data, error } = await changeUserStateWithToken(
        requireSudoToken(),
        {
          userId: testUserId,
          state: "active",
        },
      );
      assert(!error, `changeUserState(active) should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetailWithToken(
        requireNewUserToken(),
      );
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.state === "active",
        `state should be 'active', got '${refreshed!.state}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_type promotes/demotes the user", async () => {
      const { data, error } = await changeUserTypeWithToken(
        requireSudoToken(),
        {
          userId: testUserId,
          userType: "limited",
        },
      );
      assert(!error, `changeUserType should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetailWithToken(
        requireSudoToken(),
        { userId: testUserId },
      );
      assert(refreshed !== null, "refreshed detail should not be null");
      assert(
        refreshed!.user_type === "limited",
        `user_type should be 'limited', got '${refreshed!.user_type}'`,
      );
    }),
  );

  results.push(
    await runCase("user.change_type rejects promotion to root", async () => {
      const { data, error } = await changeUserTypeWithToken(
        requireSudoToken(),
        {
          userId: testUserId,
          // Bypass the compile-time exclusion for negative testing
          userType: "root" as unknown as "user",
        },
      );
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
      const { data, error } = await deleteUserWithToken(
        requireSudoToken(),
        "root",
      );
      assert(
        error !== null || data === null,
        "deleting root must be rejected",
      );
    }),
  );

  results.push(
    await runCase("user.delete rejects deleting self", async () => {
      const { data, error } = await deleteUserWithToken(
        requireSudoToken(),
        callerUserId,
      );
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
        const { data, error } = await changeUserStateWithToken(
          requireSudoToken(),
          {
            userId: "root",
            state: "suspended:should-fail",
          },
        );
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
      const { data, error } = await deleteUserWithToken(
        requireSudoToken(),
        testUserId,
      );
      assert(!error, `deleteUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");

      const { data: refreshed } = await fetchUserDetailWithToken(
        requireSudoToken(),
        { userId: testUserId },
      );
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
  // user.invite.create / user.invite.get
  // invite.accept is intentionally not exercised here: it requires a valid
  // owner_config DID document that can only be produced by a real client
  // keypair, which is out of scope for this RPC-surface smoke test.
  // -----------------------------------------------------------------------
  console.log("\n[user.invite.create / user.invite.get]");

  const inviteTargetUserId = `dvinvite${Date.now()}`;
  let createdInviteId: string | null = null;

  results.push(
    await runCase("user.invite.create generates a pending invite", async () => {
      const { data, error } = await createUserInviteWithToken(
        requireSudoToken(),
        {
          userId: inviteTargetUserId,
          showName: `Invited ${inviteTargetUserId}`,
          userType: "user",
          ttlSecs: 3600,
          groups: ["dv-test"],
        },
      );
      assert(!error, `createUserInvite should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(!!data!.invite, "response should carry an invite record");
      assert(
        typeof data!.invite.invite_id === "string" &&
          data!.invite.invite_id.length > 0,
        "invite_id should be a non-empty string",
      );
      createdInviteId = data!.invite.invite_id;
      assert(
        data!.invite.state === "pending",
        `invite state should be 'pending', got '${data!.invite.state}'`,
      );
      assert(
        data!.invite.target_user_id === inviteTargetUserId,
        `invite target_user_id should be '${inviteTargetUserId}'`,
      );
      assert(
        typeof data!.invite_url === "string" && data!.invite_url!.length > 0,
        "response should include an invite_url",
      );
    }),
  );

  results.push(
    await runCase("user.invite.get reads the invite back (public)", async () => {
      assert(createdInviteId !== null, "createdInviteId should be populated");
      const { data, error } = await fetchUserInvite(createdInviteId!);
      assert(!error, `fetchUserInvite should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.invite.invite_id === createdInviteId,
        "invite_id should round-trip",
      );
      assert(
        data!.invite.state === "pending",
        `invite state should be 'pending', got '${data!.invite.state}'`,
      );
      // Public invite view must carry zone routing info for the accept page.
      assert(
        typeof data!.zone_host === "string" && data!.zone_host!.length > 0,
        "invite response should include zone_host",
      );
    }),
  );

  results.push(
    await runCase("user.invite.get fails for unknown invite_id", async () => {
      const { data, error } = await fetchUserInvite("nonexistent_invite_xyz");
      assert(
        error !== null || data === null,
        "unknown invite_id should fail",
      );
    }),
  );

  // Clean up the pending user that invite.create pre-provisioned.
  results.push(
    await runCase("cleanup: soft-delete the invited pending user", async () => {
      const { data, error } = await deleteUserWithToken(
        requireSudoToken(),
        inviteTargetUserId,
      );
      assert(!error, `deleteUser should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
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
      `agent.get handles ${DV_DEFAULT_AGENT_ID} global doc availability`,
      async () => {
        const { data, error } = await fetchAgentDetail(DV_DEFAULT_AGENT_ID);
        if (error) {
          assert(
            String(error).includes("not found"),
            `fetchAgentDetail should either return the global doc or a not found error: ${error}`,
          );
          return;
        }
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
  // Agent write cycle: create → get → update → profile → delete
  // Agent identity management is Admin-only (Agents are a Zone-level resource).
  // -----------------------------------------------------------------------
  const testAgentId = `dvagent${Date.now()}`;
  console.log(`\n[agent write cycle: ${testAgentId}]`);

  results.push(
    await runCase("agent.create creates a new agent identity", async () => {
      const { data, error } = await createAgentWithToken(
        requireSudoToken(),
        {
          agentId: testAgentId,
          displayName: `DV Agent ${testAgentId}`,
          ownerUserId: callerUserId,
          description: "created by test_user_mgr",
        },
      );
      assert(!error, `createAgent should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        data!.agent_id === testAgentId,
        `agent_id should be '${testAgentId}', got '${data!.agent_id}'`,
      );
    }),
  );

  results.push(
    await runCase("agent.create rejects duplicate agent_id", async () => {
      const { data, error } = await createAgentWithToken(
        requireSudoToken(),
        { agentId: testAgentId },
      );
      assert(
        error !== null || data === null,
        "duplicate agent create should fail",
      );
    }),
  );

  results.push(
    await runCase("agent.list now contains the new agent", async () => {
      const { data, error } = await fetchAgentList();
      assert(!error, `fetchAgentList should not error: ${error}`);
      assert(data !== null, "data should not be null");
      const found = data!.agents.find((a) => a.agent_id === testAgentId);
      assert(!!found, `new agent '${testAgentId}' should appear in agent.list`);
    }),
  );

  results.push(
    await runCase("agent.get returns the new agent's detail", async () => {
      const { data, error } = await fetchAgentDetail(testAgentId);
      assert(!error, `fetchAgentDetail should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.agent_id === testAgentId,
        `agent_id should be '${testAgentId}'`,
      );
    }),
  );

  results.push(
    await runCase("agent.update changes display_name", async () => {
      const { data, error } = await updateAgent({
        agentId: testAgentId,
        displayName: `Renamed ${testAgentId}`,
      });
      assert(!error, `updateAgent should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
    }),
  );

  results.push(
    await runCase("agent.profile.set then agent.profile.get round-trips", async () => {
      const { data: setData, error: setErr } = await setAgentProfile({
        agentId: testAgentId,
        profile: { title: "DV Test Agent", bio: "round-trip check" },
      });
      assert(!setErr, `setAgentProfile should not error: ${setErr}`);
      assert(setData !== null, "set data should not be null");
      assert(setData!.ok === true, "ok should be true");

      const { data, error } = await fetchAgentProfile(testAgentId);
      assert(!error, `fetchAgentProfile should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.agent_id === testAgentId,
        "profile agent_id should round-trip",
      );
      const profile = (data!.profile ?? {}) as Record<string, unknown>;
      assert(
        profile.title === "DV Test Agent",
        `profile.title should round-trip, got '${String(profile.title)}'`,
      );
    }),
  );

  results.push(
    await runCase("agent.delete soft-deletes the test agent", async () => {
      const { data, error } = await deleteAgentWithToken(
        requireSudoToken(),
        testAgentId,
      );
      assert(!error, `deleteAgent should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(data!.ok === true, "ok should be true");
      assert(
        (data as Record<string, unknown>).state === "deleted",
        "delete response should report state 'deleted'",
      );

      // Soft-delete keeps the doc; agent.get still resolves but settings.state
      // is now 'deleted'.
      const { data: after, error: afterErr } = await fetchAgentDetail(
        testAgentId,
      );
      assert(!afterErr, `fetchAgentDetail should not error: ${afterErr}`);
      assert(after !== null, "soft-deleted agent should still be retrievable");
      const settings = (after!.settings ?? {}) as Record<string, unknown>;
      assert(
        settings.state === "deleted",
        `settings.state should be 'deleted', got '${String(settings.state)}'`,
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
