import { initTestRuntime } from "../../test_helpers/buckyos_client.ts";

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
type JsonObject = { [key: string]: JsonValue };

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type Message = {
  index: number;
  created_at: number;
  payload: number[];
  headers: Record<string, string>;
};

type StreamReader = {
  readFrame: (timeoutMs: number) => Promise<JsonObject>;
  close: () => void;
};

const assert = {
  ok(value: unknown, message?: string): void {
    if (!value) throw new Error(message ?? "assertion failed");
  },
  equal(actual: unknown, expected: unknown, message?: string): void {
    if (actual !== expected) {
      throw new Error(
        message ?? `expected ${String(expected)}, got ${String(actual)}`,
      );
    }
  },
};

function getEnv(name: string, fallback?: string): string | undefined {
  const value = Deno.env.get(name);
  if (typeof value === "string" && value.trim().length > 0) {
    return value.trim();
  }
  return fallback;
}

function repoRoot(): string {
  return new URL("../../..", import.meta.url).pathname;
}

function nowTag(): string {
  return `${Date.now()}-${crypto.randomUUID().slice(0, 8)}`;
}

function defaultQueueConfig(): JsonObject {
  return {
    max_messages: null,
    retention_seconds: null,
    sync_write: true,
    other_app_can_read: true,
    other_app_can_write: false,
    other_user_can_read: false,
    other_user_can_write: false,
  };
}

function encodePayload(value: JsonValue): number[] {
  return Array.from(new TextEncoder().encode(JSON.stringify(value)));
}

function decodePayload(message: Message): JsonValue {
  return JSON.parse(new TextDecoder().decode(new Uint8Array(message.payload)));
}

function asObject(value: unknown, label: string): JsonObject {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is not an object: ${JSON.stringify(value)}`);
  }
  return value as JsonObject;
}

function asMessages(value: unknown): Message[] {
  if (!Array.isArray(value)) {
    throw new Error(`expected messages array, got ${JSON.stringify(value)}`);
  }
  return value as Message[];
}

async function rpcCall<T>(
  rpc: RpcClient,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  return await rpc.call(method, params) as T;
}

async function prepareAppClientConfig(): Promise<void> {
  const sourceDir = getEnv(
    "BUCKYOS_TEST_APP_CLIENT_DIR",
    "/opt/buckyos/etc/.buckycli",
  )!;
  const tempDir = await Deno.makeTempDir({ prefix: "buckyos-appclient-" });

  await Deno.copyFile(
    `${sourceDir}/user_private_key.pem`,
    `${tempDir}/user_private_key.pem`,
  );

  const userConfig = JSON.parse(
    await Deno.readTextFile(`${sourceDir}/user_config.json`),
  ) as JsonObject;
  if (Array.isArray(userConfig["@context"])) {
    const contexts = userConfig["@context"].filter((item) =>
      typeof item === "string"
    );
    userConfig["@context"] = contexts.at(-1) ??
      "https://buckyos.org/ns/owner/v1";
  }
  await Deno.writeTextFile(
    `${tempDir}/user_config.json`,
    `${JSON.stringify(userConfig, null, 2)}\n`,
  );

  try {
    await Deno.copyFile(
      `${sourceDir}/zone_config.json`,
      `${tempDir}/zone_config.json`,
    );
  } catch {
    // zone_config.json is optional for the WebSDK private key loader.
  }

  Deno.env.set("BUCKYOS_TEST_APP_CLIENT_DIR", tempDir);
}

async function createQueue(
  rpc: RpcClient,
  name: string,
  appId: string,
  ownerUserId: string,
): Promise<string> {
  return await rpcCall<string>(rpc, "create_queue", {
    name,
    appid: appId,
    app_owner: ownerUserId,
    config: defaultQueueConfig(),
  });
}

async function cleanupQueue(rpc: RpcClient, queueUrn: string | null) {
  if (!queueUrn) return;
  try {
    await rpc.call("delete_queue", { queue_urn: queueUrn });
  } catch (error) {
    console.log("[warn] cleanup delete_queue failed", String(error));
  }
}

async function openKeventStream(
  gatewayBaseUrl: string,
  pattern: string,
): Promise<StreamReader> {
  const controller = new AbortController();
  const response = await fetch(`${gatewayBaseUrl}/kapi/kevent/stream`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ patterns: [pattern], keepalive_ms: 1000 }),
    signal: controller.signal,
  });

  if (!response.ok || !response.body) {
    const text = await response.text().catch(() => "");
    throw new Error(
      `kevent stream failed: status=${response.status}, body=${text}`,
    );
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  async function readFrame(timeoutMs: number): Promise<JsonObject> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const newline = buffer.indexOf("\n");
      if (newline >= 0) {
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        if (line.length === 0) continue;
        return asObject(JSON.parse(line), "kevent stream frame");
      }

      const remaining = deadline - Date.now();
      const readPromise = reader.read();
      const timeoutPromise = new Promise<ReadableStreamReadResult<Uint8Array>>(
        (_resolve, reject) =>
          setTimeout(
            () => reject(new Error("kevent stream read timeout")),
            remaining,
          ),
      );
      const chunk = await Promise.race([readPromise, timeoutPromise]);
      if (chunk.done) throw new Error("kevent stream closed");
      buffer += decoder.decode(chunk.value, { stream: true });
    }
    throw new Error("kevent stream read timeout");
  }

  return {
    readFrame,
    close(): void {
      controller.abort();
      reader.cancel().catch(() => {});
    },
  };
}

async function publishKevent(
  gatewayBaseUrl: string,
  eventid: string,
  data: JsonObject,
): Promise<void> {
  const response = await fetch(`${gatewayBaseUrl}/kapi/kevent/publish`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ eventid, data }),
  });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(
      `kevent publish failed: status=${response.status}, body=${body}`,
    );
  }
  assert.equal(JSON.parse(body).status, "ok");
}

async function runCommand(
  args: string[],
  options: { cwd?: string; timeoutMs?: number } = {},
): Promise<string> {
  const command = new Deno.Command(args[0], {
    args: args.slice(1),
    cwd: options.cwd,
    stdout: "piped",
    stderr: "piped",
  });
  const child = command.spawn();
  const timeoutMs = options.timeoutMs ?? 120000;
  const timeout = setTimeout(() => {
    try {
      child.kill("SIGKILL");
    } catch {
      // Process may have already exited.
    }
  }, timeoutMs);
  const output = await child.output();
  clearTimeout(timeout);

  const stdout = new TextDecoder().decode(output.stdout);
  const stderr = new TextDecoder().decode(output.stderr);
  const text = `${stdout}${stderr}`;
  if (!output.success) {
    throw new Error(
      `command failed: ${args.join(" ")}\nstatus=${output.code}\n${text}`,
    );
  }
  return text;
}

async function waitForCheck(uv: string): Promise<string> {
  let last = "";
  for (let attempt = 0; attempt < 18; attempt += 1) {
    try {
      const output = await runCommand(
        [uv, "run", "src/check.py"],
        { cwd: repoRoot(), timeoutMs: 60000 },
      );
      last = output;
      if (output.includes("Overall Status: Running")) {
        return output;
      }
    } catch (error) {
      last = String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  throw new Error(`BuckyOS did not recover after restart:\n${last}`);
}

async function observeOldStream(stream: StreamReader): Promise<string> {
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    try {
      const frame = await stream.readFrame(5000);
      if (frame.type !== "keepalive") {
        return `frame:${JSON.stringify(frame)}`;
      }
    } catch (error) {
      return `closed:${String(error)}`;
    }
  }
  return "timeout:old stream produced only keepalive frames";
}

async function run(): Promise<void> {
  await prepareAppClientConfig();
  const { buckyos, userId, ownerUserId, zoneHost } = await initTestRuntime();
  const appId = getEnv("BUCKYOS_TEST_APP_ID", "buckycli")!;
  const gatewayBaseUrl = getEnv(
    "BUCKYOS_GATEWAY_BASE_URL",
    `https://${zoneHost}`,
  )!;
  const uv = getEnv("BUCKYOS_TEST_UV", "uv")!;
  const tag = nowTag();
  const kmsg = buckyos.getServiceRpcClient("kmsg") as RpcClient;

  let queueUrn: string | null = null;
  let oldStream: StreamReader | null = null;
  let newStream: StreamReader | null = null;
  try {
    queueUrn = await createQueue(
      kmsg,
      `kevent-kmsg-restart-${tag}`,
      appId,
      ownerUserId,
    );
    const firstPayload = {
      case: "DV-MANUAL-01",
      tag,
      value: "before-restart",
    };
    const firstIndex = await rpcCall<number>(kmsg, "post_message", {
      queue_urn: queueUrn,
      message: {
        index: 0,
        created_at: 0,
        payload: encodePayload(firstPayload),
        headers: { test_case: "DV-MANUAL-01" },
      },
    });
    const subId = await rpcCall<string>(kmsg, "subscribe", {
      queue_urn: queueUrn,
      userid: userId,
      appid: appId,
      sub_id: `kevent-kmsg-restart-sub-${tag}`,
      position: "Earliest",
    });
    const fetched = asMessages(
      await kmsg.call("fetch_messages", {
        sub_id: subId,
        length: 10,
        auto_commit: false,
      }),
    );
    assert.equal(fetched.length, 1);
    assert.equal(fetched[0].index, firstIndex);
    await kmsg.call("commit_ack", { sub_id: subId, index: firstIndex });

    const eventid = `/kevent_kmsg/restart/${tag}`;
    oldStream = await openKeventStream(gatewayBaseUrl, eventid);
    const ack = await oldStream.readFrame(5000);
    assert.equal(ack.type, "ack", "kevent stream should ack before restart");

    const oldStreamOutcomePromise = observeOldStream(oldStream);
    const startOutput = await runCommand(
      [uv, "run", "src/start.py", "--skip-update"],
      { cwd: repoRoot(), timeoutMs: 180000 },
    );
    const checkOutput = await waitForCheck(uv);
    const oldStreamOutcome = await oldStreamOutcomePromise;

    const kmsgAfterRestart = buckyos.getServiceRpcClient("kmsg") as RpcClient;
    const reread = asMessages(
      await kmsgAfterRestart.call("read_message", {
        queue_urn: queueUrn,
        cursor: firstIndex,
        length: 1,
      }),
    );
    assert.equal(reread.length, 1, "message should survive service restart");
    assert.equal(
      (decodePayload(reread[0]) as JsonObject).value,
      "before-restart",
    );

    let activeSubId = subId;
    let subscriptionAfterRestart = "preserved";
    try {
      const afterAck = asMessages(
        await kmsgAfterRestart.call("fetch_messages", {
          sub_id: subId,
          length: 10,
          auto_commit: false,
        }),
      );
      assert.equal(
        afterAck.length,
        0,
        "subscription cursor should not replay an acked message after restart",
      );
    } catch (error) {
      subscriptionAfterRestart = `missing:${String(error)}`;
      activeSubId = await rpcCall<string>(kmsgAfterRestart, "subscribe", {
        queue_urn: queueUrn,
        userid: userId,
        appid: appId,
        sub_id: `kevent-kmsg-restart-recovered-sub-${tag}`,
        position: "Latest",
      });
    }

    newStream = await openKeventStream(gatewayBaseUrl, eventid);
    const newAck = await newStream.readFrame(5000);
    assert.equal(newAck.type, "ack", "kevent stream should reconnect");

    const fallbackIndex = await rpcCall<number>(
      kmsgAfterRestart,
      "post_message",
      {
        queue_urn: queueUrn,
        message: {
          index: 0,
          created_at: 0,
          payload: encodePayload({
            case: "DV-MANUAL-02",
            tag,
            value: "after-restart",
          }),
          headers: { test_case: "DV-MANUAL-02" },
        },
      },
    );
    const fallbackFetched = asMessages(
      await kmsgAfterRestart.call("fetch_messages", {
        sub_id: activeSubId,
        length: 10,
        auto_commit: false,
      }),
    );
    assert.equal(fallbackFetched.length, 1);
    assert.equal(fallbackFetched[0].index, fallbackIndex);
    await kmsgAfterRestart.call("commit_ack", {
      sub_id: activeSubId,
      index: fallbackIndex,
    });

    await publishKevent(gatewayBaseUrl, eventid, {
      queue_urn: queueUrn,
      index: fallbackIndex,
      tag,
    });
    let eventFrame: JsonObject | null = null;
    for (let attempt = 0; attempt < 6; attempt += 1) {
      const frame = await newStream.readFrame(5000);
      if (frame.type === "event") {
        eventFrame = frame;
        break;
      }
    }
    assert.ok(eventFrame, "kevent stream should work after restart");

    console.log(
      JSON.stringify({
        status: "passed",
        queue_urn: queueUrn,
        indexes: { firstIndex, fallbackIndex },
        old_stream: oldStreamOutcome,
        subscription_after_restart: subscriptionAfterRestart,
        restart_seen: startOutput.includes("BuckyOS Startup Complete"),
        check_seen: checkOutput.includes("Overall Status: Running"),
      }),
    );
  } finally {
    if (oldStream) oldStream.close();
    if (newStream) newStream.close();
    await cleanupQueue(kmsg, queueUrn);
  }
}

await run();
