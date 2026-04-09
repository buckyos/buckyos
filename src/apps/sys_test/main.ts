/**
 * sys_test backend (Deno).
 *
 * Two responsibilities:
 *
 *   1) Serve the static web bundle for the in-page tester.
 *   2) Run the same selftest cases inside an AppService runtime, exposed via
 *      `POST /sdk/appservice/selftest`. The frontend calls this endpoint to
 *      execute the cases on the server side ("在后台服务中运行检测").
 *
 * Phase 1 (initBuckyOS as AppService) is done up front. If the required
 * environment (`app_instance_config` + the `<OWNER>_<APP>_TOKEN`) is missing
 * — for example when this binary is run standalone for development — the
 * static server still works and the selftest endpoints respond with a clear
 * "AppService not initialized" error so the frontend can render it.
 *
 * Mirrors the design of tests/app-service/systest/main.ts in
 * ../../../buckyos-websdk, which is the canonical reference for driving the
 * AppService runtime from a Deno process.
 */
import { serveDir } from "jsr:@std/http/file-server";

const APP_ID = "sys-test";
const TASK_STATUS_COMPLETED = "Completed"; // mirrors src/task_mgr_client.ts

type AppInstanceIdentity = {
  appId: string;
  ownerUserId: string;
};

type SelftestCaseResult = {
  name: string;
  ok: boolean;
  durationMs: number;
  error?: string;
  details?: Record<string, unknown>;
};

type GroupId = "system_config" | "app_settings" | "task_manager" | "verify_hub";

type SystemConfigClientLike = {
  get: (key: string) => Promise<{ value: string; version: number; is_changed: boolean }>;
  set: (key: string, value: string) => Promise<void>;
};

type TaskLike = { id: number; status: string };

type TaskManagerClientLike = {
  createTask: (params: {
    name: string;
    taskType: string;
    data: unknown;
    userId: string;
    appId: string;
  }) => Promise<TaskLike>;
  updateTaskProgress: (id: number, completedItems: number, totalItems: number) => Promise<void>;
  updateTaskStatus: (id: number, status: string) => Promise<void>;
  getTask: (id: number) => Promise<TaskLike>;
  listTasks: (params: { filter?: Record<string, unknown> }) => Promise<TaskLike[]>;
  deleteTask: (id: number) => Promise<void>;
};

type NodeSdkModule = {
  buckyos: {
    initBuckyOS: (appid: string, config: Record<string, unknown>) => Promise<void>;
    login: () => Promise<unknown>;
    logout: (cleanAccountInfo?: boolean) => void;
    getAccountInfo: () => Promise<
      {
        user_id?: string;
        user_name?: string;
        user_type?: string;
        session_token?: string | null;
      } | null
    >;
    getZoneHostName: () => string | null;
    getZoneServiceURL: (serviceName: string) => string;
    getAppSetting: (settingName?: string | null) => Promise<unknown>;
    setAppSetting: (settingName: string | null, settingValue: string) => Promise<void>;
    getSystemConfigClient: () => SystemConfigClientLike;
    getTaskManagerClient: () => TaskManagerClientLike;
  };
  RuntimeType: { AppService: string };
  parseSessionTokenClaims: (token: string | null | undefined) => Record<string, unknown> | null;
};

type BootstrapState =
  | { kind: "ready"; identity: AppInstanceIdentity; sdk: NodeSdkModule }
  | { kind: "missing-env"; reason: string }
  | { kind: "failed"; reason: string };

const port = Number.parseInt(Deno.env.get("PORT") ?? "3000", 10);
const sdkRoutePrefix = "/sdk/appservice";

function getEnv(name: string): string | null {
  const value = Deno.env.get(name);
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await Deno.stat(path);
    return true;
  } catch {
    return false;
  }
}

async function resolveStaticRoot(): Promise<string> {
  const candidates = [
    new URL("./web", import.meta.url).pathname,
    new URL("./dist/web", import.meta.url).pathname,
    new URL("./dist", import.meta.url).pathname,
  ];

  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }

  throw new Error(`failed to find sys_test static root, tried: ${candidates.join(", ")}`);
}

function parseAppInstanceIdentity(appInstanceConfig: string): AppInstanceIdentity {
  const parsed = JSON.parse(appInstanceConfig) as {
    app_spec?: {
      user_id?: unknown;
      app_doc?: { name?: unknown };
    };
  };
  const appId = typeof parsed.app_spec?.app_doc?.name === "string"
    ? parsed.app_spec.app_doc.name.trim()
    : "";
  const ownerUserId = typeof parsed.app_spec?.user_id === "string"
    ? parsed.app_spec.user_id.trim()
    : "";
  if (!appId || !ownerUserId) {
    throw new Error(
      "app_instance_config is missing app_spec.user_id or app_spec.app_doc.name",
    );
  }
  return { appId, ownerUserId };
}

function getRustStyleAppServiceTokenEnvKey(identity: AppInstanceIdentity): string {
  return `${identity.ownerUserId}-${identity.appId}`
    .toUpperCase()
    .replaceAll("-", "_") + "_TOKEN";
}

async function resolveWebSdkRoot(): Promise<string> {
  const explicit = getEnv("BUCKYOS_WEBSDK_ROOT");
  const candidates = [
    explicit,
    new URL("./buckyos-websdk", import.meta.url).pathname,
    new URL("./dist/buckyos-websdk", import.meta.url).pathname,
    new URL("../../../../../buckyos-websdk", import.meta.url).pathname,
    "/Users/liuzhicong/project/buckyos-websdk",
  ].filter((value): value is string => typeof value === "string" && value.trim().length > 0);

  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }
  throw new Error(`failed to find buckyos-websdk root, tried: ${candidates.join(", ")}`);
}

async function loadSdkModule(): Promise<NodeSdkModule> {
  const sdkRoot = await resolveWebSdkRoot();
  const moduleUrl = new URL(`file://${sdkRoot}/dist/node.mjs`);
  return await import(moduleUrl.href) as NodeSdkModule;
}

async function bootstrapSdk(): Promise<BootstrapState> {
  const appInstanceConfig = getEnv("app_instance_config");
  if (!appInstanceConfig) {
    return {
      kind: "missing-env",
      reason: "missing app_instance_config; start sys_test through service_debug.tsx",
    };
  }

  let identity: AppInstanceIdentity;
  try {
    identity = parseAppInstanceIdentity(appInstanceConfig);
  } catch (error) {
    return {
      kind: "failed",
      reason: error instanceof Error ? error.message : String(error),
    };
  }

  const expectedTokenKey = getRustStyleAppServiceTokenEnvKey(identity);
  if (!getEnv(expectedTokenKey)) {
    return {
      kind: "missing-env",
      reason: `missing ${expectedTokenKey}; service_debug.tsx should inject it`,
    };
  }

  try {
    const sdk = await loadSdkModule();
    await sdk.buckyos.initBuckyOS("", {
      appId: "",
      ownerUserId: identity.ownerUserId,
      runtimeType: sdk.RuntimeType.AppService,
      zoneHost: getEnv("BUCKYOS_ZONE_HOST") ?? "",
      defaultProtocol: "https://",
    });
    await sdk.buckyos.login();
    return { kind: "ready", identity, sdk };
  } catch (error) {
    return {
      kind: "failed",
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}

function jsonResponse(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

async function readJsonBody(request: Request): Promise<Record<string, unknown>> {
  const text = (await request.text()).trim();
  if (!text) return {};
  const parsed = JSON.parse(text) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("request body must be a JSON object");
  }
  return parsed as Record<string, unknown>;
}

function isMissingSettingsError(error: unknown): boolean {
  return error instanceof Error && error.message.includes("system_config key not found");
}

function getSettingsPath(identity: AppInstanceIdentity): string {
  return `users/${identity.ownerUserId}/apps/${identity.appId}/settings`;
}

async function runSelftestCase(
  name: string,
  runCase: () => Promise<Record<string, unknown> | void>,
): Promise<SelftestCaseResult> {
  const startedAt = Date.now();
  try {
    const details = (await runCase()) ?? undefined;
    return {
      name,
      ok: true,
      durationMs: Date.now() - startedAt,
      details: details ?? undefined,
    };
  } catch (error) {
    return {
      name,
      ok: false,
      durationMs: Date.now() - startedAt,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

// Mirrors the cases in tests/helpers/service_client_suite.ts and the browser
// test_groups.ts, but runs them inside this AppService process so that the
// frontend can trigger the suite per-group with a single HTTP call.
function buildGroupRunners(
  state: Extract<BootstrapState, { kind: "ready" }>,
): Record<GroupId, () => Promise<SelftestCaseResult[]>> {
  const { sdk, identity } = state;

  const systemConfigGroup = async (): Promise<SelftestCaseResult[]> => {
    const results: SelftestCaseResult[] = [];

    results.push(
      await runSelftestCase("SystemConfigClient.get(boot/config)", async () => {
        const bootConfig = await sdk.buckyos.getSystemConfigClient().get("boot/config");
        const parsed = JSON.parse(bootConfig.value) as Record<string, unknown>;
        if (!parsed || typeof parsed !== "object") {
          throw new Error("boot/config did not decode into an object");
        }
        if (Object.keys(parsed).length === 0) {
          throw new Error("boot/config decoded into an empty object");
        }
        return { version: bootConfig.version, keys: Object.keys(parsed).length };
      }),
    );

    results.push(
      await runSelftestCase(
        "SystemConfigClient writes and reads back a namespaced key",
        async () => {
          const key = `users/${identity.ownerUserId}/apps/${identity.appId}/info`;
          const value = JSON.stringify({ ok: true, key, ts: Date.now() });
          await sdk.buckyos.getSystemConfigClient().set(key, value);
          const read = await sdk.buckyos.getSystemConfigClient().get(key);
          if (read.value !== value) {
            throw new Error(`value mismatch at ${key}`);
          }
          return { key };
        },
      ),
    );

    return results;
  };

  const appSettingsGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase(
        "getAppSetting/setAppSetting round trip on namespaced key",
        async () => {
          const settingPath = `test_settings.websdk_${Date.now()}`;
          try {
            await sdk.buckyos.setAppSetting(settingPath, '"roundtrip"');
          } catch (error) {
            if (!isMissingSettingsError(error)) throw error;
            // First-time settings write: synthesize the full settings tree at
            // the app-level key so subsequent setAppSetting calls succeed.
            const settingsPath = getSettingsPath(identity);
            const segments = settingPath.split(/[./]/).filter(Boolean);
            const rootSettings = segments.reduceRight<unknown>(
              (acc, segment) => ({ [segment]: acc }),
              "roundtrip",
            );
            await sdk.buckyos
              .getSystemConfigClient()
              .set(settingsPath, JSON.stringify(rootSettings));
          }
          const read = await sdk.buckyos.getAppSetting(settingPath);
          if (read !== "roundtrip") {
            throw new Error(`settings round trip mismatch, got ${JSON.stringify(read)}`);
          }
          return { settingPath };
        },
      ),
    ];
  };

  const taskManagerGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase(
        "TaskManagerClient creates/updates/queries/deletes a namespaced task",
        async () => {
          const client = sdk.buckyos.getTaskManagerClient();
          const name = `test-websdk-${Date.now()}`;
          const created = await client.createTask({
            name,
            taskType: "test",
            data: { createdBy: "sys-test-backend" },
            userId: identity.ownerUserId,
            appId: identity.appId,
          });
          try {
            await client.updateTaskProgress(created.id, 1, 2);
            await client.updateTaskStatus(created.id, TASK_STATUS_COMPLETED);
            const fetched = await client.getTask(created.id);
            if (fetched.status !== TASK_STATUS_COMPLETED) {
              throw new Error(
                `expected task ${created.id} to be Completed, got ${fetched.status}`,
              );
            }
            const filtered = await client.listTasks({
              filter: { root_id: String(created.id) },
            });
            if (!filtered.some((task) => task.id === created.id)) {
              throw new Error(`task ${created.id} missing from filtered list`);
            }
            return { taskId: created.id };
          } finally {
            try {
              await client.deleteTask(created.id);
            } catch {
              // best-effort cleanup, ignore
            }
          }
        },
      ),
    ];
  };

  const verifyHubGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase("getAccountInfo + parseSessionTokenClaims", async () => {
        const accountInfo = await sdk.buckyos.getAccountInfo();
        if (!accountInfo) {
          throw new Error("AppService is not logged in");
        }
        const claims = sdk.parseSessionTokenClaims(accountInfo.session_token ?? null);
        if (!claims) {
          throw new Error("failed to parse session token claims");
        }
        return {
          userId: accountInfo.user_id ?? null,
          userType: accountInfo.user_type ?? null,
          appId: claims.appid ?? null,
          exp: claims.exp ?? null,
        };
      }),
    ];
  };

  return {
    system_config: systemConfigGroup,
    app_settings: appSettingsGroup,
    task_manager: taskManagerGroup,
    verify_hub: verifyHubGroup,
  };
}

const bootstrapState = await bootstrapSdk();
const staticRoot = await resolveStaticRoot();
const groupRunners = bootstrapState.kind === "ready"
  ? buildGroupRunners(bootstrapState)
  : null;

if (bootstrapState.kind === "ready") {
  console.log(
    `[sys_test] AppService initialized as ${bootstrapState.identity.ownerUserId}/${bootstrapState.identity.appId}`,
  );
} else {
  console.warn(
    `[sys_test] AppService NOT initialized (${bootstrapState.kind}): ${bootstrapState.reason}`,
  );
  console.warn(
    "[sys_test] static page will still work; /sdk/appservice/* endpoints will return an error",
  );
}

console.log(`[sys_test] serving ${staticRoot} on http://0.0.0.0:${port}`);
console.log(`[sys_test] sdk routes mounted at ${sdkRoutePrefix}`);

function appServiceUnavailableResponse(): Response {
  const reason = bootstrapState.kind === "ready" ? "unknown" : bootstrapState.reason;
  return jsonResponse(
    {
      ok: false,
      error: `AppService not initialized: ${reason}`,
      hint:
        "start sys_test through buckyos node-daemon, or via tests/scripts/debug_systest.sh-style harness, so that app_instance_config and the <OWNER>_<APP>_TOKEN env are present",
    },
    503,
  );
}

function summarizeHeaders(req: Request): Record<string, string> {
  const interesting = [
    "host",
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "x-forwarded-uri",
    "x-real-ip",
    "user-agent",
    "content-type",
    "content-length",
    "origin",
    "referer",
  ];
  const out: Record<string, string> = {};
  for (const name of interesting) {
    const value = req.headers.get(name);
    if (value !== null) out[name] = value;
  }
  return out;
}

let requestSeq = 0;

Deno.serve({
  port,
  hostname: "0.0.0.0",
  onListen: ({ hostname, port }) => {
    console.log(`[sys_test] listening on http://${hostname}:${port}`);
    console.log(`[sys_test] mounted routes:`);
    console.log(`  GET  ${sdkRoutePrefix}/healthz`);
    console.log(`  GET  ${sdkRoutePrefix}/runtime`);
    console.log(`  POST ${sdkRoutePrefix}/selftest             (run all groups)`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/system_config`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/app_settings`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/task_manager`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/verify_hub`);
    console.log(`  GET  *                                       (static dist/)`);
  },
}, async (req: Request) => {
  const reqId = ++requestSeq;
  const startedAt = Date.now();
  let url: URL;
  try {
    url = new URL(req.url);
  } catch (error) {
    console.warn(
      `[sys_test][req#${reqId}] failed to parse req.url=${JSON.stringify(req.url)}: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
    return jsonResponse({ ok: false, error: "invalid request URL" }, 400);
  }

  console.log(
    `[sys_test][req#${reqId}] -> ${req.method} ${url.pathname}${url.search} headers=${
      JSON.stringify(summarizeHeaders(req))
    }`,
  );

  const log = (status: number, route: string) => {
    console.log(
      `[sys_test][req#${reqId}] <- ${status} ${route} (${Date.now() - startedAt}ms)`,
    );
  };
  const tap = (route: string, response: Response): Response => {
    log(response.status, route);
    return response;
  };

  try {

    if (req.method === "GET" && url.pathname === `${sdkRoutePrefix}/healthz`) {
      return tap("healthz", jsonResponse({
        ok: bootstrapState.kind === "ready",
        appId: APP_ID,
        bootstrap: bootstrapState.kind,
      }));
    }

    if (req.method === "GET" && url.pathname === `${sdkRoutePrefix}/runtime`) {
      if (bootstrapState.kind !== "ready") {
        return tap("runtime[unavail]", appServiceUnavailableResponse());
      }
      const { sdk, identity } = bootstrapState;
      const accountInfo = await sdk.buckyos.getAccountInfo();
      return tap("runtime", jsonResponse({
        ok: true,
        mode: "app-service",
        appId: identity.appId,
        ownerUserId: identity.ownerUserId,
        zoneHost: sdk.buckyos.getZoneHostName(),
        hostGateway: getEnv("BUCKYOS_HOST_GATEWAY"),
        expectedTokenEnvKey: getRustStyleAppServiceTokenEnvKey(identity),
        serviceUrls: {
          verifyHub: sdk.buckyos.getZoneServiceURL("verify-hub"),
          taskManager: sdk.buckyos.getZoneServiceURL("task-manager"),
          systemConfig: sdk.buckyos.getZoneServiceURL("system-config"),
        },
        accountInfo: accountInfo
          ? {
            userId: accountInfo.user_id ?? null,
            userType: accountInfo.user_type ?? null,
          }
          : null,
        tokenClaims: sdk.parseSessionTokenClaims(accountInfo?.session_token ?? null),
      }));
    }

    // Per-group selftest endpoint, e.g.
    //   POST /sdk/appservice/selftest/system_config
    //   POST /sdk/appservice/selftest/app_settings
    //   POST /sdk/appservice/selftest/task_manager
    //   POST /sdk/appservice/selftest/verify_hub
    //
    // Each test group on the frontend gets its own URL so the routing in
    // cyfs-gateway / static servers in front of this process can express
    // per-endpoint policies, and so logs are easy to grep per group.
    if (
      req.method === "POST" &&
      url.pathname.startsWith(`${sdkRoutePrefix}/selftest/`)
    ) {
      if (!groupRunners || bootstrapState.kind !== "ready") {
        return tap("selftest[unavail]", appServiceUnavailableResponse());
      }
      const groupId = url.pathname.slice(`${sdkRoutePrefix}/selftest/`.length) as GroupId;
      const runner = groupRunners[groupId];
      if (!runner) {
        return tap(
          `selftest/${groupId}[unknown]`,
          jsonResponse(
            {
              ok: false,
              group: groupId,
              error: `no such group: ${groupId}`,
              availableGroups: Object.keys(groupRunners),
            },
            404,
          ),
        );
      }
      const results = await runner();
      const ok = results.every((result) => result.ok);
      return tap(
        `selftest/${groupId}`,
        jsonResponse(
          {
            ok,
            group: groupId,
            appId: bootstrapState.identity.appId,
            ownerUserId: bootstrapState.identity.ownerUserId,
            results,
          },
          ok ? 200 : 500,
        ),
      );
    }

    // Convenience endpoint that runs every group at once. The body is
    // optional and ignored — kept around so that the systest jest harness
    // (tests/app-service/integration/app_service_test.ts) and any
    // command-line callers can still trigger the full sweep with one call.
    if (req.method === "POST" && url.pathname === `${sdkRoutePrefix}/selftest`) {
      if (!groupRunners || bootstrapState.kind !== "ready") {
        return tap("selftest[unavail]", appServiceUnavailableResponse());
      }
      const results: SelftestCaseResult[] = [];
      for (const groupId of Object.keys(groupRunners) as GroupId[]) {
        const groupResults = await groupRunners[groupId]();
        results.push(...groupResults);
      }
      const ok = results.every((result) => result.ok);
      return tap(
        "selftest[all]",
        jsonResponse(
          {
            ok,
            group: "all",
            appId: bootstrapState.identity.appId,
            ownerUserId: bootstrapState.identity.ownerUserId,
            results,
          },
          ok ? 200 : 500,
        ),
      );
    }

    const staticResponse = await serveDir(req, {
      fsRoot: staticRoot,
      quiet: true,
    });
    return tap("static", staticResponse);
  } catch (error) {
    console.error(
      `[sys_test][req#${reqId}] !! handler threw: ${
        error instanceof Error ? (error.stack ?? error.message) : String(error)
      }`,
    );
    return tap(
      "error",
      jsonResponse(
        { ok: false, error: error instanceof Error ? error.message : String(error) },
        500,
      ),
    );
  }
});
