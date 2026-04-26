/**
 * buckyos_client.ts — 公共的 AppClient Runtime 初始化函数
 *
 * 所有 DV 测试用例共享此模块，一行代码即可得到已登录的 buckyos runtime：
 *
 *   import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
 *   const { buckyos, userId } = await initTestRuntime();
 *
 * 初始化流程参照 buckyos-websdk/tests/app-client/integration/app_client_test.ts：
 *   1. 从环境变量或默认值构造 AppClient 配置
 *   2. 自动从本地搜索私钥 (~/.buckyos, /opt/buckyos/etc 等)
 *   3. 本地签名 JWT 并 login
 *
 * 环境变量（均可选，有默认值）：
 *   BUCKYOS_TEST_APP_ID          — appId，默认 "buckycli"
 *   BUCKYOS_TEST_ZONE_HOST       — zone 主机名，默认 "test.buckyos.io"
 *   BUCKYOS_TEST_VERIFY_HUB_URL  — verify-hub 服务地址
 *   BUCKYOS_TEST_SYSTEM_CONFIG_URL — system-config 服务地址
 *   BUCKYOS_TEST_APP_CLIENT_DIR  — 额外的私钥搜索目录
 */

import {
  buckyos,
  RuntimeType,
  parseSessionTokenClaims,
} from "buckyos";

// ---------------------------------------------------------------------------
// DV 环境 TLS：用自签证书，需要跳过校验
// ---------------------------------------------------------------------------

/**
 * DV 环境使用自签证书，需要在启动时加 --unsafely-ignore-certificate-errors 参数：
 *   deno run --allow-net --allow-read --allow-env \
 *     --unsafely-ignore-certificate-errors test_app_mgr.ts
 *
 * 此函数仅在 DV zone 时打印提示，实际跳过由 Deno 启动参数控制。
 */
function warnInsecureTlsIfNeeded(zoneHost: string) {
  if (zoneHost === "test.buckyos.io" || zoneHost.endsWith(".test.buckyos.io")) {
    console.log(
      "[warn] DV zone detected — make sure to run with --unsafely-ignore-certificate-errors",
    );
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function getEnv(name: string, fallback?: string): string | undefined {
  const val = Deno.env.get(name);
  if (typeof val === "string" && val.trim().length > 0) return val.trim();
  return fallback;
}

function getServiceUrl(zoneHost: string, serviceName: string): string {
  return `https://${zoneHost}/kapi/${serviceName}`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface TestRuntime {
  /** 已初始化并登录的 buckyos SDK 实例 */
  buckyos: typeof buckyos;
  /** 当前登录用户 ID */
  userId: string;
  /** zone 拥有者用户 ID，用于查询用户级资源 (apps/agents) */
  ownerUserId: string;
  /** zone 主机名 */
  zoneHost: string;
  /** 解析后的 session token claims */
  sessionTokenClaims: Record<string, unknown> | null;
}

let cached: TestRuntime | null = null;

/**
 * 初始化 buckyos AppClient Runtime 并登录。
 * 多次调用会返回同一个已登录实例（进程内单例）。
 */
export async function initTestRuntime(): Promise<TestRuntime> {
  if (cached) return cached;

  const appId = getEnv("BUCKYOS_TEST_APP_ID", "buckycli")!;
  const zoneHost = getEnv("BUCKYOS_TEST_ZONE_HOST", "test.buckyos.io")!;

  const homeDir = Deno.env.get("HOME") ?? "";
  const privateKeySearchPaths = [
    getEnv("BUCKYOS_TEST_APP_CLIENT_DIR"),
    "/opt/buckyos/etc/.buckycli",
    "/opt/buckyos/etc",
    `${homeDir}/.buckycli`,
    `${homeDir}/.buckyos`,
  ].filter((item): item is string => Boolean(item));

  warnInsecureTlsIfNeeded(zoneHost);

  await buckyos.initBuckyOS(appId, {
    appId,
    ownerUserId: "devtest",
    runtimeType: RuntimeType.AppClient,
    zoneHost,
    defaultProtocol: "https://",
    privateKeySearchPaths,
    autoRenew: false,
  } as unknown as Parameters<typeof buckyos.initBuckyOS>[1]);

  // 等待本地签名的 JWT 时间生效
  await sleep(1100);
  const accountInfo = await buckyos.login();
  console.log("accountInfo", accountInfo);
  if (!accountInfo?.session_token) {
    throw new Error("AppClient login failed to produce a session token");
  }

  const sessionTokenClaims = parseSessionTokenClaims(
    accountInfo.session_token,
  ) as Record<string, unknown> | null;

  const userId =
    accountInfo.user_id ??
    (sessionTokenClaims?.sub as string | undefined) ??
    (sessionTokenClaims?.userid as string | undefined) ??
    "devtest";

  const ownerUserId =
    buckyos.getBuckyOSConfig()?.ownerUserId ?? userId;

  cached = { buckyos, userId, ownerUserId, zoneHost, sessionTokenClaims };
  return cached;
}
