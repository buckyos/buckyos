import { resolveRuntimeConnection } from "./runtime_config.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

Deno.test("injected AppClient session uses the internal node gateway", () => {
  assertEquals(
    resolveRuntimeConnection({
      BUCKYOS_APP_ID: "jarvis",
      BUCKYOS_OWNER_USER_ID: "alice",
      BUCKYOS_APPCLIENT_SESSION_TOKEN: "session-token",
      BUCKYOS_HOST_GATEWAY: "172.17.0.1",
    }),
    {
      appId: "jarvis",
      ownerUserId: "alice",
      zoneHost: "172.17.0.1:3180",
      defaultProtocol: "http://",
      sessionToken: "session-token",
      usesInjectedSession: true,
    },
  );
});

Deno.test(
  "injected AppClient session does not duplicate an explicit gateway port",
  () => {
    const config = resolveRuntimeConnection({
      BUCKYOS_APPCLIENT_SESSION_TOKEN: "session-token",
      BUCKYOS_HOST_GATEWAY: "host.docker.internal:43180",
    });
    assertEquals(config.zoneHost, "host.docker.internal:43180");
  },
);

Deno.test("standalone mode keeps the external zone and local signing flow", () => {
  assertEquals(
    resolveRuntimeConnection({
      BUCKYOS_TEST_APP_ID: "local-tool",
      BUCKYOS_TEST_ZONE_HOST: "test.example.com",
    }),
    {
      appId: "local-tool",
      ownerUserId: "devtest",
      zoneHost: "test.example.com",
      defaultProtocol: "https://",
      usesInjectedSession: false,
    },
  );
});
