import { fromFileUrl } from "jsr:@std/path@^1";

const assertModule = "node:assert/strict";
const { default: assert } = await import(assertModule);

function unusedPort(): number {
  const listener = Deno.listen({ hostname: "127.0.0.1", port: 0 });
  const port = listener.addr.port;
  listener.close();
  return port;
}

async function waitFor<T>(read: () => Promise<T | null>): Promise<T> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const result = await read();
    if (result !== null) return result;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("timed out waiting for AppService bootstrap");
}

async function startApp(gatewayPort: number) {
  const sdkRoot = await Deno.makeTempDir({ prefix: "systest-bootstrap-" });
  const sdkUrl = new URL(
    "./node_modules/buckyos/dist/node.mjs",
    import.meta.url,
  );
  await Deno.mkdir(`${sdkRoot}/dist`);
  await Deno.writeTextFile(
    `${sdkRoot}/dist/node.mjs`,
    `
import * as sdk from ${JSON.stringify(sdkUrl.href)};
export const { VerifyHubClient, RuntimeType, parseSessionTokenClaims } = sdk;
export const buckyos = {
  kRPCClient: class extends sdk.buckyos.kRPCClient {
    constructor() { super("http://127.0.0.1:${gatewayPort}/kapi/verify-hub"); }
  },
  async initBuckyOS(appId, options) {
    if (options.sessionToken !== "session" || options.refreshToken !== "refresh") {
      throw new Error("bootstrap did not pass the Verify Hub token pair");
    }
  },
  async login() {},
  getZoneHostName: () => "bootstrap.test",
};
`,
  );
  const port = unusedPort();
  const app = new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "--no-prompt",
      "--allow-net=127.0.0.1,0.0.0.0",
      "--allow-env",
      "--allow-read",
      fromFileUrl(new URL("./main.ts", import.meta.url)),
    ],
    cwd: new URL(".", import.meta.url),
    env: {
      BUCKYOS_SERVICE_PORT: String(port),
      BUCKYOS_WEBSDK_ROOT: sdkRoot,
      BUCKYOS_APP_ID: "bootstrap.test",
      BUCKYOS_OWNER_USER_ID: "test-user",
      BUCKYOS_APP_INSTANCE_ID: "bootstrap.test@test-user",
      BUCKYOS_APP_TOKEN: "device-assertion",
      BUCKYOS_HOST_GATEWAY: "127.0.0.1",
      BUCKYOS_ZONE_HOST: "bootstrap.test",
      app_instance_config: JSON.stringify({
        node_execution_spec: { app_instance_id: "bootstrap.test@test-user" },
      }),
    },
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const stdout = new Response(app.stdout).text();
  const stderr = new Response(app.stderr).text();
  return {
    async health() {
      try {
        const response = await fetch(
          `http://127.0.0.1:${port}/sdk/appservice/healthz`,
        );
        return await response.json() as { ok: boolean; bootstrap: string };
      } catch {
        return null;
      }
    },
    async failure() {
      const response = await fetch(
        `http://127.0.0.1:${port}/sdk/appservice/runtime`,
      );
      assert.equal(response.status, 503);
      return (await response.json()).error as string;
    },
    async close() {
      try {
        app.kill("SIGTERM");
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
      await app.status;
      const logs = (await stdout) + (await stderr);
      await Deno.remove(sdkRoot, { recursive: true });
      return logs;
    },
  };
}

Deno.test("AppService recovers after Deno connection refusal and gateway 503", async () => {
  const gatewayPort = unusedPort();
  const app = await startApp(gatewayPort);
  let gateway: Deno.HttpServer | undefined;
  let requests = 0;
  let logs = "";
  try {
    assert.equal((await waitFor(() => app.health())).bootstrap, "failed");
    assert.match(await app.failure(), /RPC call failed: .*Connection refused/i);
    gateway = Deno.serve({
      hostname: "127.0.0.1",
      port: gatewayPort,
      onListen() {},
    }, async (req) => {
      requests += 1;
      const body = await req.json();
      assert.equal(body.method, "login_by_jwt");
      assert.equal(
        body.params.target.app_instance_id,
        "bootstrap.test@test-user",
      );
      return requests === 1
        ? new Response("starting", { status: 503 })
        : Response.json({
          sys: body.sys,
          result: { session_token: "session", refresh_token: "refresh" },
        });
    });
    await waitFor(async () => {
      const state = await app.health();
      return state?.ok ? state : null;
    });
    assert.equal(requests, 2);
  } finally {
    logs = await app.close();
    await gateway?.shutdown();
  }
  assert.ok(/AppService initialized .* after \d+ retries/.test(logs));
});

for (const failure of [401, 403, "invalid login assertion"]) {
  Deno.test(`AppService does not retry authentication failure: ${failure}`, async () => {
    let requests = 0;
    const gateway = Deno.serve({
      hostname: "127.0.0.1",
      port: 0,
      onListen() {},
    }, async (req) => {
      requests += 1;
      await req.body?.cancel();
      return typeof failure === "number"
        ? new Response("denied", { status: failure })
        : Response.json({ error: failure });
    });
    const app = await startApp(gateway.addr.port);
    try {
      assert.equal((await waitFor(() => app.health())).bootstrap, "failed");
      assert.ok((await app.failure()).includes(String(failure)));
      await new Promise((resolve) => setTimeout(resolve, 800));
      assert.equal((await app.health())?.bootstrap, "failed");
      assert.equal(requests, 1);
    } finally {
      const logs = await app.close();
      await gateway.shutdown();
      assert.ok(!logs.includes("AppService initialization retry"));
    }
  });
}
