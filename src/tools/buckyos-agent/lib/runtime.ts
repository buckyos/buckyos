// Init the BuckyOS web SDK runtime. Mirrors test/test_helpers/buckyos_client.ts
// but tailored for the CLI: env-var-driven, single-shot login.

import { buckyos, parseSessionTokenClaims, RuntimeType } from "buckyos";
import { resolveRuntimeConnection } from "./runtime_config.ts";

export interface AiccRuntime {
  // deno-lint-ignore no-explicit-any
  buckyos: any;
  userId: string;
  ownerUserId: string;
  zoneHost: string;
  appId: string;
}

let cached: AiccRuntime | null = null;

function envOpt(name: string): string | undefined {
  const v = Deno.env.get(name);
  return typeof v === "string" && v.trim() ? v.trim() : undefined;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export async function initRuntime(): Promise<AiccRuntime> {
  if (cached) return cached;

  const connection = resolveRuntimeConnection(Deno.env.toObject());
  const { appId, ownerUserId, zoneHost } = connection;
  const homeDir = Deno.env.get("HOME") ?? "";
  const privateKeySearchPaths = [
    envOpt("BUCKYOS_APP_CLIENT_DIR"),
    envOpt("BUCKYOS_TEST_APP_CLIENT_DIR"),
    "/opt/buckyos/etc/.buckycli",
    "/opt/buckyos/etc",
    `${homeDir}/.buckycli`,
    `${homeDir}/.buckyos`,
  ].filter((p): p is string => !!p);

  // deno-lint-ignore no-explicit-any
  await (buckyos as any).initBuckyOS(appId, {
    appId,
    ownerUserId,
    runtimeType: RuntimeType.AppClient,
    zoneHost,
    defaultProtocol: connection.defaultProtocol,
    sessionToken: connection.sessionToken,
    privateKeySearchPaths,
    autoRenew: false,
  });

  if (!connection.usesInjectedSession) {
    // Local JWT signature uses a not-before timestamp ~1s in the future.
    await sleep(1100);
  }
  // deno-lint-ignore no-explicit-any
  const account = await (buckyos as any).login();
  if (!account?.session_token) {
    throw new Error("AppClient login failed to produce a session token");
  }
  const claims =
    (parseSessionTokenClaims(account.session_token) as Record<string, unknown> | null) ?? null;

  const userId = account.user_id ?? (claims?.sub as string | undefined) ??
    (claims?.userid as string | undefined) ?? ownerUserId;
  // deno-lint-ignore no-explicit-any
  const runtimeOwnerUserId = (buckyos as any).getBuckyOSConfig?.()?.ownerUserId ?? ownerUserId;

  cached = { buckyos, userId, ownerUserId: runtimeOwnerUserId, zoneHost, appId };
  return cached;
}

export function teardown(): void {
  try {
    // deno-lint-ignore no-explicit-any
    (cached?.buckyos as any)?.logout?.(false);
  } catch {
    // best-effort
  }
}
