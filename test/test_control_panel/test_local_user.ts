import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;

type RpcClient = {
  call(method: string, params: JsonRecord): Promise<unknown>;
};

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
const restartEnabled = ["1", "true", "yes"].includes(
  (Deno.env.get("BUCKYOS_TEST_RESTART") || "").toLowerCase(),
);
const appId = "control-panel";
const appInstanceId = "control-panel@system";
let lastNonce = Date.now();

function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

function serviceUrl(service: string): string {
  return `https://${zoneHost}/kapi/${service}`;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

function asRecord(value: unknown, label: string): JsonRecord {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  return value as JsonRecord;
}

async function call(
  service: string,
  method: string,
  params: JsonRecord,
  token: string | null = null,
): Promise<unknown> {
  const rpc = new KRpcClient(serviceUrl(service), token, nextNonce());
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

async function login(username: string, password: string): Promise<JsonRecord> {
  const nonce = nextNonce();
  return asRecord(
    await call("control-panel", "auth.login", {
      username,
      password: hashPassword(username, password, nonce),
      appid: appId,
      login_nonce: nonce,
    }),
    "auth.login response",
  );
}

async function sudo(username: string, password: string): Promise<string> {
  const nonce = nextNonce();
  const response = asRecord(
    await call("verify-hub", "sudo_by_password", {
      username,
      password: hashPassword(username, password, nonce),
      appid: appId,
      app_instance_id: appInstanceId,
      aud: "system-config",
      login_nonce: nonce,
    }),
    "sudo_by_password response",
  );
  assert(typeof response.session_token === "string" && response.session_token, "sudo token is missing");
  return response.session_token;
}

async function controlPanel(
  token: string,
  method: string,
  params: JsonRecord = {},
): Promise<JsonRecord> {
  return asRecord(await call("control-panel", method, params, token), `${method} response`);
}

async function systemConfigGet(token: string, key: string): Promise<JsonRecord> {
  return asRecord(
    await call("system_config", "sys_config_get", { key }, token),
    `sys_config_get(${key}) response`,
  );
}

async function waitForUsersRole(token: string, userId: string): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const policy = await systemConfigGet(token, "system/rbac/policy");
    if (String(policy.value).includes(`g, ${userId}, users`)) return;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`scheduler RBAC does not contain users role for ${userId}`);
}

async function changeState(token: string, userId: string, state: string): Promise<void> {
  const result = await controlPanel(token, "user.change_state", { user_id: userId, state });
  assert(result.ok === true, `change state to ${state} failed`);
}

async function runUv(script: "stop.py" | "start.py"): Promise<void> {
  const command = new Deno.Command("uv", {
    args: ["run", script],
    cwd: new URL("../../src/", import.meta.url),
    stdout: "inherit",
    stderr: "inherit",
  });
  const result = await command.spawn().status;
  assert(result.success, `${script} failed with exit code ${result.code}`);
}

async function waitForLogin(username: string, password: string): Promise<JsonRecord> {
  let lastError: unknown;
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      return await login(username, password);
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
  }
  throw lastError ?? new Error("Zone did not become ready after restart");
}

async function main(): Promise<void> {
  const suffix = `${Date.now()}${crypto.getRandomValues(new Uint16Array(1))[0]}`;
  const userId = `dvlocal${suffix}`.slice(0, 48).toLowerCase();
  const submittedUserId = `  ${userId.toUpperCase()}  `;
  const passwordBytes = crypto.getRandomValues(new Uint8Array(18));
  const localPassword = `Dv-${Array.from(passwordBytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}!`;
  const displayName = `DV Local ${suffix.slice(-6)}`;
  const passwordHash = hashPassword(userId, localPassword);
  let adminLogin = await login(adminUser, adminPassword);
  let adminToken = String(adminLogin.session_token || "");
  let adminSudo = await sudo(adminUser, adminPassword);
  let localLogin: JsonRecord | null = null;
  let deleted = false;

  assert(adminToken, "admin auth.login did not return a session token");
  console.log(`[local-user-dv] zone=${zoneHost} user=${userId}`);

  try {
    await expectReject("regular admin token cannot create", () =>
      controlPanel(adminToken, "user.create", {
        user_id: `${userId}nosudo`,
        password_hash: passwordHash,
        user_type: "user",
      })
    );
    await expectReject("illegal password hash is rejected", () =>
      controlPanel(adminSudo, "user.create", {
        user_id: `${userId}badhash`,
        password_hash: "not-a-sha256-digest",
        user_type: "user",
      })
    );
    await expectReject("unsupported user type is rejected", () =>
      controlPanel(adminSudo, "user.create", {
        user_id: `${userId}admin`,
        password_hash: passwordHash,
        user_type: "admin",
      })
    );

    const collisionUserId = `${userId}collision`.slice(0, 60);
    await call("system_config", "sys_config_create", {
      key: `security/${collisionUserId}/key`,
      value: "collision-sentinel",
    }, adminSudo);
    try {
      await expectReject("transaction conflict rejects the whole create", () =>
        controlPanel(adminSudo, "user.create", {
          user_id: collisionUserId,
          password_hash: passwordHash,
          user_type: "user",
        })
      );
      for (const suffix of ["settings", "doc", "profile"]) {
        await expectReject(`transaction left no users/${collisionUserId}/${suffix}`, () =>
          systemConfigGet(adminSudo, `users/${collisionUserId}/${suffix}`)
        );
      }
    } finally {
      await call("system_config", "sys_config_delete", {
        key: `security/${collisionUserId}/key`,
      }, adminSudo);
    }

    const created = await controlPanel(adminSudo, "user.create", {
      user_id: submittedUserId,
      show_name: displayName,
      password_hash: passwordHash,
      user_type: "user",
      allow_password_change: true,
    });
    assert(created.ok === true && created.created === true, "user.create did not confirm commit");
    assert(created.user_id === userId, "user.create did not trim and lowercase user_id");
    assert(typeof created.rbac_refreshed === "boolean", "user.create omitted rbac_refreshed");
    console.log(`  ✓ created; rbac_refreshed=${created.rbac_refreshed}`);
    await waitForUsersRole(adminSudo, userId);
    console.log("  ✓ scheduler RBAC contains the users role");

    await expectReject("duplicate username is rejected", () =>
      controlPanel(adminSudo, "user.create", {
        user_id: userId,
        password_hash: passwordHash,
        user_type: "user",
      })
    );
    await expectReject("wrong password is rejected", () => login(userId, `${localPassword}-wrong`));

    localLogin = await login(userId, localPassword);
    const localToken = String(localLogin.session_token || "");
    const refreshToken = String(localLogin.refresh_token || "");
    const userInfo = asRecord(localLogin.user_info, "login user_info");
    assert(localToken && refreshToken, "local auth.login did not return both tokens");
    assert(userInfo.user_id === userId && userInfo.user_type === "user", "local login identity is wrong");
    console.log("  ✓ local auth.login returned session and refresh tokens");

    const self = await controlPanel(localToken, "user.get");
    assert(self.user_id === userId, "user.get did not default to self");
    assert(self.state === "active" && self.is_local === true, "self detail is not active/local");
    await controlPanel(localToken, "apps.list");
    console.log("  ✓ ordinary local user can query its authorized apps");
    await expectReject("local user cannot read another user", () =>
      controlPanel(localToken, "user.get", { user_id: adminUser })
    );
    await expectReject("local user cannot create another user", () =>
      controlPanel(localToken, "user.create", {
        user_id: `${userId}child`,
        password_hash: passwordHash,
        user_type: "user",
      })
    );

    const listed = await controlPanel(adminToken, "user.list");
    const users = Array.isArray(listed.users) ? listed.users.map((item) => asRecord(item, "listed user")) : [];
    const listedUser = users.find((item) => item.user_id === userId);
    assert(listedUser?.show_name === displayName, "user.list did not return backend display name");
    assert(listedUser?.is_local === true, "user.list did not expose local-account source");
    assert(listedUser?.allow_password_change === true, "user.list did not expose password policy");

    const key = await systemConfigGet(adminSudo, `security/${userId}/key`);
    assert(typeof key.value === "string" && key.value.length > 0, "private key was not stored under security/");
    await expectReject("ordinary admin cannot read private key", () =>
      systemConfigGet(adminToken, `security/${userId}/key`)
    );
    await expectReject("ordinary user cannot read private key", () =>
      systemConfigGet(localToken, `security/${userId}/key`)
    );

    for (const state of ["pending", "suspended:dv-test", "banned:dv-test"]) {
      await changeState(adminSudo, userId, state);
      await expectReject(`${state}: login rejected`, () => login(userId, localPassword));
      await expectReject(`${state}: sudo rejected`, () => sudo(userId, localPassword));
      await expectReject(`${state}: refresh rejected`, () =>
        call("verify-hub", "refresh_token", { refresh_token: refreshToken })
      );
    }

    await changeState(adminSudo, userId, "active");
    localLogin = await login(userId, localPassword);
    assert(localLogin.session_token, "reactivated user could not log in");

    if (restartEnabled) {
      console.log("  … restarting BuckyOS for persistence verification (without --all)");
      await runUv("stop.py");
      await runUv("start.py");
      localLogin = await waitForLogin(userId, localPassword);
      adminLogin = await waitForLogin(adminUser, adminPassword);
      adminToken = String(adminLogin.session_token || "");
      adminSudo = await sudo(adminUser, adminPassword);
      const persisted = await controlPanel(adminSudo, "user.get", { user_id: userId });
      assert(persisted.user_id === userId && persisted.state === "active", "user did not survive restart");
      console.log("  ✓ user login and API records survived stop/start");
    } else {
      console.log("  ↷ persistence restart skipped; set BUCKYOS_TEST_RESTART=1 to enable it");
    }

    const activeRefresh = String(localLogin.refresh_token || "");
    const removed = await controlPanel(adminSudo, "user.delete", { user_id: userId });
    assert(removed.ok === true, "user.delete failed");
    deleted = true;
    await expectReject("deleted: login rejected", () => login(userId, localPassword));
    await expectReject("deleted: sudo rejected", () => sudo(userId, localPassword));
    await expectReject("deleted: refresh rejected", () =>
      call("verify-hub", "refresh_token", { refresh_token: activeRefresh })
    );

    const retained = await controlPanel(adminSudo, "user.get", { user_id: userId });
    assert(retained.state === "deleted", "soft-deleted record was not retained");
    const storedSettings = await systemConfigGet(adminSudo, `users/${userId}/settings`);
    assert(String(storedSettings.value).includes("deleted"), "system-config settings record was not retained");
    console.log(`  ✓ cleanup soft-deleted ${userId}; retained users/${userId}/settings for audit`);
  } finally {
    if (!deleted) {
      try {
        const cleanup = await controlPanel(adminSudo, "user.delete", { user_id: userId });
        console.log(`  ↷ cleanup user.delete ok=${cleanup.ok}`);
      } catch (error) {
        console.error(`  ! cleanup failed for ${userId}: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
  }

  console.log("[local-user-dv] PASS");
}

await main();
