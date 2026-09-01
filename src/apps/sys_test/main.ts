/**
 * sys_test backend (Deno).
 *
 * Two responsibilities:
 *
 *   1) Serve the static web bundle for the in-page tester.
 *   2) Run the same selftest cases inside an AppService runtime, exposed via
 *      `POST /sdk/appservice/selftest`. The frontend calls this endpoint to
 *      execute the cases on the server side ("Run tests on backend service").
 *
 * Phase 1 (initBuckyOS as AppService) is done up front. If the required
 * environment (`app_instance_config` + fixed `BUCKYOS_APP_*` variables) is missing
 * — for example when this binary is run standalone for development — the
 * static server still works and the selftest endpoints respond with a clear
 * "AppService not initialized" error so the frontend can render it.
 *
 * Mirrors the design of tests/app-service/systest/main.ts in
 * ../../../buckyos-websdk, which is the canonical reference for driving the
 * AppService runtime from a Deno process.
 */
import { serveDir } from "jsr:@std/http/file-server";

type NodeSdkModule = typeof import("@sys-test/websdk-node-types");
type NdmModule = NodeSdkModule["ndm"];
type QueryObjectByIdResponse = Awaited<
  ReturnType<NdmModule["queryObjectById"]>
>;
type QueryChunkStateResponse = Awaited<
  ReturnType<NdmModule["queryChunkState"]>
>;

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

type GroupId =
  | "runtime"
  | "system_config"
  | "app_settings"
  | "task_manager"
  | "verify_hub"
  | "kevent"
  | "service_clients"
  | "sdk_utilities"
  | "ndm_proxy";

type ContentAvailabilityState =
  | {
    kind: "chunk";
    state: QueryChunkStateResponse | { state: "error"; error: string };
  }
  | {
    kind: "object";
    state: QueryObjectByIdResponse | { state: "error"; error: string };
  };

type BootstrapState =
  | { kind: "ready"; identity: AppInstanceIdentity; sdk: NodeSdkModule }
  | { kind: "missing-env"; reason: string }
  | { kind: "failed"; reason: string };

const port = Number.parseInt(Deno.env.get("PORT") ?? "3000", 10);
const sdkRoutePrefix = "/sdk/appservice";
const bootstrapRetryDelaysMs = [250, 500, 1_000, 2_000, 5_000, 10_000, 30_000];

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
    new URL("./web/dist", import.meta.url).pathname,
    new URL("./web", import.meta.url).pathname,
    new URL("./dist/web", import.meta.url).pathname,
    new URL("./dist", import.meta.url).pathname,
  ];

  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }

  throw new Error(
    `failed to find sys_test static root, tried: ${candidates.join(", ")}`,
  );
}

function parseAppInstanceIdentity(
  appInstanceConfig: string,
): AppInstanceIdentity {
  const appId = getEnv("BUCKYOS_APP_ID") ?? "";
  const ownerUserId = getEnv("BUCKYOS_OWNER_USER_ID") ?? "";
  const appInstanceId = getEnv("BUCKYOS_APP_INSTANCE_ID") ?? "";
  if (!appId || !ownerUserId || appInstanceId !== `${appId}@${ownerUserId}`) {
    throw new Error(
      "fixed BuckyOS app identity environment is missing or inconsistent",
    );
  }
  const parsed = JSON.parse(appInstanceConfig) as {
    node_execution_spec?: {
      app_instance_id?: unknown;
    };
  };
  const configuredInstanceId =
    typeof parsed.node_execution_spec?.app_instance_id === "string"
      ? parsed.node_execution_spec.app_instance_id.trim()
      : "";
  if (configuredInstanceId !== appInstanceId) {
    throw new Error(
      "app_instance_config AppInstanceId does not match the fixed environment",
    );
  }
  return { appId, ownerUserId };
}

async function resolveWebSdkRoot(): Promise<string> {
  const explicit = getEnv("BUCKYOS_WEBSDK_ROOT");
  const candidates = [
    explicit,
    new URL("./node_modules/buckyos", import.meta.url).pathname,
    new URL("./dist/node_modules/buckyos", import.meta.url).pathname,
  ].filter((value): value is string =>
    typeof value === "string" && value.trim().length > 0
  );

  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }
  throw new Error(
    `failed to find buckyos-websdk root, tried: ${candidates.join(", ")}`,
  );
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
      reason:
        "missing app_instance_config; start sys_test through service_debug.tsx",
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

  const appToken = getEnv("BUCKYOS_APP_TOKEN");
  if (!appToken) {
    return {
      kind: "missing-env",
      reason: "missing BUCKYOS_APP_TOKEN; service_debug.tsx should inject it",
    };
  }

  try {
    const sdk = await loadSdkModule();
    await sdk.buckyos.initBuckyOS(identity.appId, {
      appId: identity.appId,
      ownerUserId: identity.ownerUserId,
      runtimeType: sdk.RuntimeType.AppService,
      zoneHost: getEnv("BUCKYOS_ZONE_HOST") ?? "",
      defaultProtocol: "https://",
      sessionToken: appToken,
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

function isRetryableBootstrapFailure(state: BootstrapState): boolean {
  return state.kind === "failed" &&
    (state.reason.includes("RPC call failed: fetch failed") ||
      /RPC call error: 50[234]/.test(state.reason));
}

function delay(durationMs: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

function jsonResponse(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

async function readJsonBody(
  request: Request,
): Promise<Record<string, unknown>> {
  const text = (await request.text()).trim();
  if (!text) return {};
  const parsed = JSON.parse(text) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("request body must be a JSON object");
  }
  return parsed as Record<string, unknown>;
}

function isMissingSettingsError(error: unknown): boolean {
  return error instanceof Error &&
    error.message.includes("system_config key not found");
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

function getKEventBaseUrl(sdk: NodeSdkModule): string {
  const baseUrl = sdk.buckyos.getZoneServiceURL("kevent");
  return baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
}

function getKEventRequestUrl(
  sdk: NodeSdkModule,
  path: "publish" | "stream",
): string {
  return new URL(path, getKEventBaseUrl(sdk)).toString();
}

async function readJsonResponse(
  response: Response,
): Promise<Record<string, unknown>> {
  const text = await response.text();
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch {
    throw new Error(
      `non-json response (${response.status}): ${text.slice(0, 200)}`,
    );
  }
}

async function publishKEvent(
  sdk: NodeSdkModule,
  eventid: string,
  data: Record<string, unknown>,
): Promise<void> {
  const response = await fetch(getKEventRequestUrl(sdk, "publish"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ eventid, data }),
  });
  const payload = await readJsonResponse(response);
  if (!response.ok || payload.status !== "ok") {
    throw new Error(
      String(
        payload.error ?? `kevent publish failed with status ${response.status}`,
      ),
    );
  }
}

// Mirrors the cases in tests/helpers/service_client_suite.ts and the browser
// test_groups.ts, but runs them inside this AppService process so that the
// frontend can trigger the suite per-group with a single HTTP call.
function buildGroupRunners(
  state: Extract<BootstrapState, { kind: "ready" }>,
): Record<GroupId, () => Promise<SelftestCaseResult[]>> {
  const { sdk, identity } = state;

  const runtimeGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase(
        "Runtime identity and service URL resolution",
        async () => {
          const actualAppId = sdk.buckyos.getAppId();
          const zoneHost = sdk.buckyos.getZoneHostName();
          if (actualAppId !== identity.appId) {
            throw new Error(
              `expected appId ${identity.appId}, got ${actualAppId ?? "null"}`,
            );
          }
          if (!zoneHost) {
            throw new Error("getZoneHostName() returned an empty value");
          }
          const services = [
            "system-config",
            "task-manager",
            "workflow",
            "aicc",
            "kmsg",
            "msg-center",
            "repo-service",
          ];
          const serviceUrls = Object.fromEntries(
            services.map((name) => [
              name,
              sdk.buckyos.getZoneServiceURL(name),
            ]),
          );
          return {
            runtimeType: sdk.buckyos.getRuntimeType(),
            appId: actualAppId,
            zoneHost,
            serviceUrls,
          };
        },
      ),
    ];
  };

  const systemConfigGroup = async (): Promise<SelftestCaseResult[]> => {
    const results: SelftestCaseResult[] = [];

    results.push(
      await runSelftestCase("SystemConfigClient.get(boot/config)", async () => {
        const bootConfig = await sdk.buckyos.getSystemConfigClient().get(
          "boot/config",
        );
        const parsed = JSON.parse(bootConfig.value) as Record<string, unknown>;
        if (!parsed || typeof parsed !== "object") {
          throw new Error("boot/config did not decode into an object");
        }
        if (Object.keys(parsed).length === 0) {
          throw new Error("boot/config decoded into an empty object");
        }
        return {
          version: bootConfig.version,
          keys: Object.keys(parsed).length,
        };
      }),
    );

    results.push(
      await runSelftestCase(
        "SystemConfigClient writes and reads back a namespaced key",
        async () => {
          const key =
            `users/${identity.ownerUserId}/apps/${identity.appId}/info`;
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
            throw new Error(
              `settings round trip mismatch, got ${JSON.stringify(read)}`,
            );
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
            schema_id: "raw/v1",
            input: { createdBy: "sys-test-backend" },
            executor: { kind: "SelfApp" },
            idempotency_key: `sys-test-${crypto.randomUUID()}`,
          });
          const taskId = created.task_id;
          try {
            await client.runnerStart(taskId);
            await client.runnerProgress(taskId, { completed: 1, total: 2 });
            await client.runnerComplete(taskId, { ok: true });
            const fetched = await client.getTask(taskId);
            if (
              fetched.phase !== "Terminal" || fetched.outcome !== "Succeeded"
            ) {
              throw new Error(
                `expected task ${taskId} to succeed, got ${fetched.phase}/${fetched.outcome}`,
              );
            }
            const page = await client.listTasks({ root_id: created.root_id });
            if (!page.tasks.some((task) => task.task_id === taskId)) {
              throw new Error(`task ${taskId} missing from filtered list`);
            }
            return { taskId };
          } finally {
            try {
              const latest = await client.getTask(taskId);
              if (
                latest.phase === "Terminal" && latest.archived_at === undefined
              ) {
                await client.archiveTask({
                  task_id: taskId,
                  expected_revision: latest.revision,
                });
              }
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
      await runSelftestCase(
        "getAccountInfo + parseSessionTokenClaims",
        async () => {
          const accountInfo = await sdk.buckyos.getAccountInfo();
          if (!accountInfo) {
            throw new Error("AppService is not logged in");
          }
          const claims = sdk.parseSessionTokenClaims(
            accountInfo.session_token ?? null,
          );
          if (!claims) {
            throw new Error("failed to parse session token claims");
          }
          return {
            userId: accountInfo.user_id ?? null,
            userType: accountInfo.user_type ?? null,
            appId: claims.appid ?? null,
            exp: claims.exp ?? null,
          };
        },
      ),
    ];
  };

  const keventGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase(
        "KEvent stream/publish round trip on a unique eventid",
        async () => {
          const eventid =
            `/users/${identity.ownerUserId}/apps/${identity.appId}/kevent/sys_test_${Date.now()}_${
              crypto.randomUUID().replaceAll("-", "").slice(0, 8)
            }`;
          const marker = `app_service_${Date.now()}`;
          const reader = await sdk.buckyos.createEventReader(eventid, {
            keepaliveMs: 1_000,
          });
          try {
            await publishKEvent(sdk, eventid, {
              marker,
              origin: "sys_test_app_service",
              userId: identity.ownerUserId,
              appId: identity.appId,
            });

            const event = await reader.pullEvent(5_000);
            if (!event) {
              throw new Error("timed out waiting for the published kevent");
            }
            const eventData = event.data && typeof event.data === "object"
              ? event.data as Record<string, unknown>
              : {};
            if (event.eventid !== eventid) {
              throw new Error(`received mismatched eventid: ${event.eventid}`);
            }
            if (eventData.marker !== marker) {
              throw new Error(
                `received mismatched marker: ${JSON.stringify(eventData)}`,
              );
            }

            return {
              eventid,
              sourceNode: event.source_node,
              sourcePid: event.source_pid,
              ingressNode: event.ingress_node ?? null,
              timestamp: event.timestamp,
            };
          } finally {
            await reader.close();
          }
        },
      ),
    ];
  };

  const serviceClientsGroup = async (): Promise<SelftestCaseResult[]> => {
    const owner = sdk.bns.didBnsFromName(identity.ownerUserId);
    return [
      await runSelftestCase("WorkflowClient.listDefinitions", async () => {
        const definitions = await sdk.buckyos
          .getWorkflowClient()
          .listDefinitions({
            owner: {
              user_id: identity.ownerUserId,
              app_id: identity.appId,
            },
          });
        return { definitions: definitions.length };
      }),
      await runSelftestCase("AiccClient.queryQuota", async () => {
        const { quota } = await sdk.buckyos.getAiccClient().queryQuota({});
        if (typeof quota.state !== "string" || quota.state.length === 0) {
          throw new Error("quota.query returned an invalid state");
        }
        return { quota };
      }),
      await runSelftestCase("MsgCenterClient.peekBox", async () => {
        const records = await sdk.buckyos.getMsgCenterClient().peekBox({
          owner,
          box_kind: "INBOX",
          limit: 1,
          with_object: false,
        });
        return { owner, records: records.length };
      }),
      await runSelftestCase("RepoClient.stat", async () => {
        const stat = await sdk.buckyos.getRepoClient().stat();
        return { stat };
      }),
      await runSelftestCase(
        "MsgQueueClient create/post/stat/delete lifecycle",
        async () => {
          const client = sdk.buckyos.getMsgQueueClient();
          const queueName = [
            "sys-test",
            String(Date.now()),
            crypto.randomUUID().replaceAll("-", "").slice(0, 8),
          ].join("-");
          const queueUrn = await client.createQueue(
            queueName,
            identity.appId,
            owner,
          );
          try {
            const msgIndex = await client.postMessage(queueUrn, {
              index: 0,
              created_at: Date.now(),
              payload: Array.from(new TextEncoder().encode("sys_test")),
              headers: { source: "sys_test_app_service" },
            });
            const stats = await client.getQueueStats(queueUrn);
            if (stats.message_count < 1) {
              throw new Error(
                `expected at least one queued message, got ${stats.message_count}`,
              );
            }
            return { queueUrn, msgIndex, stats };
          } finally {
            await client.deleteQueue(queueUrn);
          }
        },
      ),
    ];
  };

  const sdkUtilitiesGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase("BNS name/DID canonical round trip", async () => {
        const canonicalName = sdk.bns.canonicalBnsName(identity.ownerUserId);
        const did = sdk.bns.didBnsFromName(canonicalName);
        const roundTripName = sdk.bns.nameFromDidBns(did);
        if (roundTripName !== canonicalName) {
          throw new Error(`BNS round trip mismatch: ${roundTripName}`);
        }
        return { canonicalName, did };
      }),
      await runSelftestCase("SN URL and region normalization", async () => {
        const authUrl = sdk.sn.normalizeSnUrl("https://sn.example", "auth");
        const region = sdk.sn.normalizeSnRegionIdHint("  US__West / 2  ");
        if (
          authUrl !== "https://sn.example/kapi/sn/auth" ||
          region !== "us-west-2"
        ) {
          throw new Error(
            `unexpected SN normalization: ${authUrl}, ${region}`,
          );
        }
        return { authUrl, region };
      }),
    ];
  };

  const ndmProxyGroup = async (): Promise<SelftestCaseResult[]> => {
    return [
      await runSelftestCase("ndm_proxy.outboxCount", async () => {
        const result = await sdk.ndm_proxy.outboxCount();
        if (!Number.isInteger(result.count) || result.count < 0) {
          throw new Error(`invalid outbox count: ${result.count}`);
        }
        return { count: result.count };
      }),
    ];
  };

  return {
    runtime: runtimeGroup,
    system_config: systemConfigGroup,
    app_settings: appSettingsGroup,
    task_manager: taskManagerGroup,
    verify_hub: verifyHubGroup,
    kevent: keventGroup,
    service_clients: serviceClientsGroup,
    sdk_utilities: sdkUtilitiesGroup,
    ndm_proxy: ndmProxyGroup,
  };
}

let bootstrapState = await bootstrapSdk();
const staticRoot = await resolveStaticRoot();
let groupRunners = bootstrapState.kind === "ready"
  ? buildGroupRunners(bootstrapState)
  : null;

let bootstrapRetryAttempt = 0;

async function retryBootstrapUntilReady(): Promise<void> {
  if (!isRetryableBootstrapFailure(bootstrapState)) {
    return;
  }

  const delayMs = bootstrapRetryDelaysMs[
    Math.min(bootstrapRetryAttempt, bootstrapRetryDelaysMs.length - 1)
  ];
  bootstrapRetryAttempt += 1;
  console.warn(
    `[sys_test] AppService initialization retry ${bootstrapRetryAttempt} in ${delayMs}ms`,
  );
  await delay(delayMs);

  const nextState = await bootstrapSdk();
  bootstrapState = nextState;
  if (nextState.kind === "ready") {
    groupRunners = buildGroupRunners(nextState);
    console.log(
      `[sys_test] AppService initialized as ${nextState.identity.ownerUserId}/${nextState.identity.appId} after ${bootstrapRetryAttempt} retries`,
    );
    return;
  }

  console.warn(
    `[sys_test] AppService initialization retry ${bootstrapRetryAttempt} failed (${nextState.kind}): ${nextState.reason}`,
  );
  void retryBootstrapUntilReady();
}

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
  void retryBootstrapUntilReady();
}

console.log(`[sys_test] serving ${staticRoot} on http://0.0.0.0:${port}`);
// Log static root contents for debugging
try {
  const entries: string[] = [];
  for await (const entry of Deno.readDir(staticRoot)) {
    entries.push(`${entry.isDirectory ? "d" : "f"} ${entry.name}`);
  }
  console.log(`[sys_test] static root contents: ${entries.join(", ")}`);
} catch (e) {
  console.warn(
    `[sys_test] failed to list static root: ${
      e instanceof Error ? e.message : String(e)
    }`,
  );
}
console.log(`[sys_test] sdk routes mounted at ${sdkRoutePrefix}`);

function appServiceUnavailableResponse(): Response {
  const reason = bootstrapState.kind === "ready"
    ? "unknown"
    : bootstrapState.reason;
  return jsonResponse(
    {
      ok: false,
      error: `AppService not initialized: ${reason}`,
      hint:
        "start sys_test through buckyos node-daemon, or via tests/scripts/debug_systest.sh-style harness, so that app_instance_config and fixed BUCKYOS_APP_* variables are present",
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

function isChunkListContentId(contentId: string): boolean {
  return contentId.startsWith("clist:") || contentId.startsWith("chunklist:");
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) &&
    value.every((item) => typeof item === "string" && item.length > 0);
}

function extractChunkIdsFromChunkListObjectData(
  objData: string,
): string[] | null {
  try {
    const parsed = JSON.parse(objData) as unknown;
    return isStringArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

async function queryContentAvailabilityState(
  ndm: NdmModule,
  contentId: string,
): Promise<ContentAvailabilityState> {
  if (
    contentId.startsWith("chunk:") || contentId.startsWith("mix256:") ||
    contentId.startsWith("sha256:")
  ) {
    const state = await ndm.queryChunkState({ chunk_id: contentId });
    return { kind: "chunk", state };
  }

  const state = await ndm.queryObjectById({ obj_id: contentId });
  return { kind: "object", state };
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
    console.log(
      `  POST ${sdkRoutePrefix}/selftest             (run all groups)`,
    );
    console.log(`  POST ${sdkRoutePrefix}/selftest/runtime`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/system_config`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/app_settings`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/task_manager`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/verify_hub`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/kevent`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/service_clients`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/sdk_utilities`);
    console.log(`  POST ${sdkRoutePrefix}/selftest/ndm_proxy`);
    console.log(
      `  POST ${sdkRoutePrefix}/ndm_query             (query FileObjId status)`,
    );
    console.log(
      `  GET  *                                       (static dist/)`,
    );
  },
}, async (req: Request) => {
  const reqId = ++requestSeq;
  const startedAt = Date.now();
  let url: URL;
  try {
    url = new URL(req.url);
  } catch (error) {
    console.warn(
      `[sys_test][req#${reqId}] failed to parse req.url=${
        JSON.stringify(req.url)
      }: ${error instanceof Error ? error.message : String(error)}`,
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
      `[sys_test][req#${reqId}] <- ${status} ${route} (${
        Date.now() - startedAt
      }ms)`,
    );
  };
  const tap = (route: string, response: Response): Response => {
    log(response.status, route);
    return response;
  };

  try {
    if (req.method === "GET" && url.pathname === `${sdkRoutePrefix}/healthz`) {
      return tap(
        "healthz",
        jsonResponse({
          ok: bootstrapState.kind === "ready",
          appId: bootstrapState.kind === "ready"
            ? bootstrapState.identity.appId
            : null,
          bootstrap: bootstrapState.kind,
        }),
      );
    }

    if (req.method === "GET" && url.pathname === `${sdkRoutePrefix}/runtime`) {
      if (bootstrapState.kind !== "ready") {
        return tap("runtime[unavail]", appServiceUnavailableResponse());
      }
      const { sdk, identity } = bootstrapState;
      const accountInfo = await sdk.buckyos.getAccountInfo();
      return tap(
        "runtime",
        jsonResponse({
          ok: true,
          mode: "app-service",
          appId: identity.appId,
          ownerUserId: identity.ownerUserId,
          zoneHost: sdk.buckyos.getZoneHostName(),
          hostGateway: getEnv("BUCKYOS_HOST_GATEWAY"),
          expectedTokenEnvKey: "BUCKYOS_APP_TOKEN",
          serviceUrls: {
            verifyHub: sdk.buckyos.getZoneServiceURL("verify-hub"),
            taskManager: sdk.buckyos.getZoneServiceURL("task-manager"),
            systemConfig: sdk.buckyos.getZoneServiceURL("system-config"),
            kevent: sdk.buckyos.getZoneServiceURL("kevent"),
            workflow: sdk.buckyos.getZoneServiceURL("workflow"),
            aicc: sdk.buckyos.getZoneServiceURL("aicc"),
            msgQueue: sdk.buckyos.getZoneServiceURL("kmsg"),
            msgCenter: sdk.buckyos.getZoneServiceURL("msg-center"),
            repo: sdk.buckyos.getZoneServiceURL("repo-service"),
          },
          accountInfo: accountInfo
            ? {
              userId: accountInfo.user_id ?? null,
              userType: accountInfo.user_type ?? null,
            }
            : null,
          tokenClaims: sdk.parseSessionTokenClaims(
            accountInfo?.session_token ?? null,
          ),
        }),
      );
    }

    // NDM query endpoint: receives FileObjId + FileObject + chunkList + qcid from the
    // frontend after upload, then uses ndm store APIs to:
    //   1. putObject — store chunklist/FileObject metadata so NDM knows about it
    //   2. queryObjectById — query the object state
    //   3. isObjectStored/queryChunkState — query content + qcid state
    // The qcid is stored alongside the FileObject so that future uploads
    // of the same file content can be resolved instantly (instant upload).
    if (
      req.method === "POST" && url.pathname === `${sdkRoutePrefix}/ndm_query`
    ) {
      if (bootstrapState.kind !== "ready") {
        return tap("ndm_query[unavail]", appServiceUnavailableResponse());
      }
      try {
        const body = await readJsonBody(req);
        const fileObjId = body.fileObjId as string | undefined;
        const fileObject = body.fileObject as
          | Record<string, unknown>
          | undefined;
        const qcid = body.qcid as string | undefined;
        const chunkList = body.chunkList;
        const preUploadState = body.preUploadState as
          | Record<string, unknown>
          | undefined;

        if (typeof fileObjId !== "string" || !fileObjId) {
          return tap(
            "ndm_query[bad-req]",
            jsonResponse({ ok: false, error: "fileObjId is required" }, 400),
          );
        }

        console.log(
          `[sys_test] ndm_query: fileObjId=${fileObjId}, qcid=${qcid ?? "N/A"}`,
        );

        const { ndm } = bootstrapState.sdk;
        const contentId = typeof fileObject?.content === "string" &&
            fileObject.content.length > 0
          ? fileObject.content
          : null;
        const isChunkListContent = contentId
          ? isChunkListContentId(contentId)
          : false;

        let putChunkListResult: { ok: boolean; error?: string } | null = null;
        if (contentId && isChunkListContent) {
          if (!isStringArray(chunkList)) {
            putChunkListResult = {
              ok: false,
              error:
                "chunkList is required when fileObject.content is a chunklist id",
            };
            console.warn(
              `[sys_test] ndm_query: missing chunkList for ${contentId}`,
            );
          } else {
            try {
              await ndm.putObject({
                obj_id: contentId,
                obj_data: JSON.stringify(chunkList),
              });
              console.log(
                `[sys_test] ndm_query: putObject OK for chunklist ${contentId}`,
              );
              putChunkListResult = { ok: true };
            } catch (e) {
              const msg = e instanceof Error ? e.message : String(e);
              console.warn(
                `[sys_test] ndm_query: put chunklist failed: ${msg}`,
              );
              putChunkListResult = { ok: false, error: msg };
            }
          }
        }

        let putObjectResult: { ok: boolean; error?: string } = { ok: true };
        if (fileObject) {
          const objDataToStore = qcid
            ? { ...fileObject, _qcid: qcid }
            : fileObject;
          try {
            await ndm.putObject({
              obj_id: fileObjId,
              obj_data: JSON.stringify(objDataToStore),
            });
            console.log(`[sys_test] ndm_query: putObject OK for ${fileObjId}`);
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(`[sys_test] ndm_query: putObject failed: ${msg}`);
            putObjectResult = { ok: false, error: msg };
          }
        }

        let objectState: QueryObjectByIdResponse | {
          state: "error";
          error: string;
        };
        try {
          objectState = await ndm.queryObjectById({ obj_id: fileObjId });
          console.log(
            `[sys_test] ndm_query: queryObjectById state=${objectState.state}`,
          );
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          console.warn(`[sys_test] ndm_query: queryObjectById failed: ${msg}`);
          objectState = { state: "error", error: msg };
        }

        let contentState:
          | { contentId: string; state: ContentAvailabilityState }
          | null = null;
        if (contentId) {
          try {
            const availabilityState = await queryContentAvailabilityState(
              ndm,
              contentId,
            );
            console.log(
              `[sys_test] ndm_query: content availability for ${contentId} queried`,
            );
            contentState = { contentId, state: availabilityState };
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(
              `[sys_test] ndm_query: content availability query failed for ${contentId}: ${msg}`,
            );
            contentState = {
              contentId,
              state: { kind: "object", state: { state: "error", error: msg } },
            };
          }
        }

        let contentStoredState:
          | {
            contentId: string;
            state: { stored: boolean } | { state: "error"; error: string };
          }
          | null = null;
        if (contentId) {
          try {
            const storedState = await ndm.isObjectStored({ obj_id: contentId });
            console.log(
              `[sys_test] ndm_query: isObjectStored(${contentId}) = ${storedState.stored}`,
            );
            contentStoredState = { contentId, state: storedState };
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(
              `[sys_test] ndm_query: isObjectStored(${contentId}) failed: ${msg}`,
            );
            contentStoredState = {
              contentId,
              state: { state: "error", error: msg },
            };
          }
        }

        let contentChunkStates: Array<{
          chunkId: string;
          state: QueryChunkStateResponse | { state: "error"; error: string };
        }> = [];
        if (contentId && isChunkListContent) {
          const chunkIds = isStringArray(chunkList)
            ? chunkList
            : contentState?.state.kind === "object" &&
                contentState.state.state.state === "object"
            ? extractChunkIdsFromChunkListObjectData(
              contentState.state.state.obj_data,
            ) ?? []
            : [];

          for (const chunkId of chunkIds) {
            try {
              const chunkState = await ndm.queryChunkState({
                chunk_id: chunkId,
              });
              console.log(
                `[sys_test] ndm_query: queryChunkState(${chunkId}) = ${chunkState.state}`,
              );
              contentChunkStates.push({ chunkId, state: chunkState });
            } catch (e) {
              const msg = e instanceof Error ? e.message : String(e);
              console.warn(
                `[sys_test] ndm_query: queryChunkState(${chunkId}) failed: ${msg}`,
              );
              contentChunkStates.push({
                chunkId,
                state: { state: "error", error: msg },
              });
            }
          }
        }

        let addSameAsResult:
          | { ok: boolean; skipped?: boolean; error?: string }
          | null = null;
        if (qcid && contentId && isChunkListContent) {
          const fileSize = typeof fileObject?.size === "number" &&
              Number.isFinite(fileObject.size)
            ? fileObject.size
            : null;
          const contentFullyStored = contentStoredState?.state &&
            "stored" in contentStoredState.state &&
            contentStoredState.state.stored === true;

          if (fileSize === null) {
            addSameAsResult = {
              ok: false,
              error:
                "fileObject.size is required when adding qcid same_as mapping",
            };
          } else if (!contentFullyStored) {
            addSameAsResult = {
              ok: false,
              error:
                `skip addChunkBySameAs because content ${contentId} is not fully stored yet`,
            };
            console.warn(
              `[sys_test] ndm_query: skip addChunkBySameAs for ${qcid} because ${contentId} is not fully stored`,
            );
          } else {
            try {
              await ndm.addChunkBySameAs({
                big_chunk_id: qcid,
                chunk_list_id: contentId,
                big_chunk_size: fileSize,
              });
              console.log(
                `[sys_test] ndm_query: addChunkBySameAs OK for ${qcid} -> ${contentId}`,
              );
              addSameAsResult = { ok: true };
            } catch (e) {
              const msg = e instanceof Error ? e.message : String(e);
              console.warn(
                `[sys_test] ndm_query: addChunkBySameAs failed: ${msg}`,
              );
              addSameAsResult = { ok: false, error: msg };
            }
          }
        } else if (qcid) {
          addSameAsResult = { ok: true, skipped: true };
        }

        let qcidState:
          | {
            chunkId: string;
            state: QueryChunkStateResponse | { state: "error"; error: string };
          }
          | null = null;
        if (qcid) {
          try {
            const chunkState = await ndm.queryChunkState({
              chunk_id: qcid,
            });
            console.log(
              `[sys_test] ndm_query: queryChunkState(${qcid}) = ${chunkState.state}`,
            );
            qcidState = { chunkId: qcid, state: chunkState };
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(
              `[sys_test] ndm_query: queryChunkState(${qcid}) failed: ${msg}`,
            );
            qcidState = {
              chunkId: qcid,
              state: { state: "error", error: msg },
            };
          }
        }

        return tap(
          "ndm_query",
          jsonResponse({
            ok: true,
            fileObjId,
            qcid: qcid ?? null,
            preUploadState: preUploadState ?? null,
            putChunkList: putChunkListResult,
            putObject: putObjectResult,
            addSameAs: addSameAsResult,
            objectState,
            contentState,
            contentStoredState,
            contentChunkStates,
            qcidState,
          }),
        );
      } catch (error) {
        return tap(
          "ndm_query[error]",
          jsonResponse(
            {
              ok: false,
              error: error instanceof Error ? error.message : String(error),
            },
            500,
          ),
        );
      }
    }

    // Per-group selftest endpoint, e.g.
    //   POST /sdk/appservice/selftest/system_config
    //   POST /sdk/appservice/selftest/app_settings
    //   POST /sdk/appservice/selftest/task_manager
    //   POST /sdk/appservice/selftest/verify_hub
    //   POST /sdk/appservice/selftest/kevent
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
      const groupId = url.pathname.slice(
        `${sdkRoutePrefix}/selftest/`.length,
      ) as GroupId;
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
    if (
      req.method === "POST" && url.pathname === `${sdkRoutePrefix}/selftest`
    ) {
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
      showIndex: true,
    });
    // Fallback: if the static file is not found and the request is a
    // navigation (not an asset), serve index.html so the SPA can handle
    // client-side routing.
    if (staticResponse.status === 404) {
      const accept = req.headers.get("accept") ?? "";
      if (accept.includes("text/html")) {
        const fallback = await serveDir(
          new Request(new URL("/index.html", req.url), req),
          { fsRoot: staticRoot, quiet: true, showIndex: true },
        );
        if (fallback.status === 200) {
          return tap("static[fallback]", fallback);
        }
      }
    }
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
        {
          ok: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      ),
    );
  }
});
