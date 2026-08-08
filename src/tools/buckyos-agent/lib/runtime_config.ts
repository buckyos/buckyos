export interface RuntimeEnvironment {
  [name: string]: string | undefined;
}

export interface RuntimeConnectionConfig {
  appId: string;
  ownerUserId: string;
  zoneHost: string;
  defaultProtocol: "http://" | "https://";
  sessionToken?: string;
  usesInjectedSession: boolean;
}

function value(env: RuntimeEnvironment, name: string): string | undefined {
  const raw = env[name];
  return typeof raw === "string" && raw.trim() ? raw.trim() : undefined;
}

function hostWithPort(host: string, port: string): string {
  if (/:\d+$/.test(host)) return host;
  return `${host}:${port}`;
}

export function resolveRuntimeConnection(
  env: RuntimeEnvironment,
): RuntimeConnectionConfig {
  const appId = value(env, "BUCKYOS_APP_ID") ??
    value(env, "BUCKYOS_TEST_APP_ID") ?? "buckyos-agent";
  const ownerUserId = value(env, "BUCKYOS_OWNER_USER_ID") ?? "devtest";
  const sessionToken = value(env, "BUCKYOS_APPCLIENT_SESSION_TOKEN");

  if (sessionToken) {
    const gatewayHost = value(env, "BUCKYOS_HOST_GATEWAY") ??
      "host.docker.internal";
    const gatewayPort = value(env, "BUCKYOS_NODE_GATEWAY_PORT") ?? "3180";
    return {
      appId,
      ownerUserId,
      zoneHost: hostWithPort(gatewayHost, gatewayPort),
      defaultProtocol: "http://",
      sessionToken,
      usesInjectedSession: true,
    };
  }

  return {
    appId,
    ownerUserId,
    zoneHost: value(env, "BUCKYOS_ZONE_HOST") ??
      value(env, "BUCKYOS_TEST_ZONE_HOST") ?? "test.buckyos.io",
    defaultProtocol: "https://",
    usesInjectedSession: false,
  };
}
