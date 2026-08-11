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

type QueueStats = {
  message_count: number;
  first_index: number;
  last_index: number;
  size_bytes: number;
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

  const userConfigPath = `${sourceDir}/user_config.json`;
  const userConfig = JSON.parse(
    await Deno.readTextFile(userConfigPath),
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
    // zone_config.json is not required by the WebSDK private key loader.
  }

  Deno.env.set("BUCKYOS_TEST_APP_CLIENT_DIR", tempDir);
}

function nowTag(): string {
  return `${Date.now()}-${crypto.randomUUID().slice(0, 8)}`;
}

function defaultQueueConfig(): JsonObject {
  return {
    max_messages: null,
    retention_seconds: null,
    sync_write: false,
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
  const raw = await rpc.call(method, params);
  return raw as T;
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

async function cleanupQueue(
  rpc: RpcClient,
  queueUrn: string | null,
): Promise<void> {
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
    body: JSON.stringify({
      patterns: [pattern],
      keepalive_ms: 1000,
    }),
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
  const parsed = JSON.parse(body);
  assert.equal(parsed.status, "ok", "kevent publish response should be ok");
}

async function probeKmsgGet(gatewayBaseUrl: string): Promise<void> {
  const response = await fetch(`${gatewayBaseUrl}/kapi/kmsg`, {
    method: "GET",
  });
  console.log(
    JSON.stringify({
      case: "DV-03B",
      note: "GET is recorded for current behavior only",
      status: response.status,
    }),
  );
}

async function run(): Promise<void> {
  await prepareAppClientConfig();
  const { buckyos, userId, ownerUserId, zoneHost } = await initTestRuntime();
  const appId = getEnv("BUCKYOS_TEST_APP_ID", "buckycli")!;
  const gatewayBaseUrl = getEnv(
    "BUCKYOS_GATEWAY_BASE_URL",
    `https://${zoneHost}`,
  )!;
  const tag = nowTag();
  const kmsg = buckyos.getServiceRpcClient("kmsg") as RpcClient;

  let queueUrn: string | null = null;
  let stream: StreamReader | null = null;
  try {
    queueUrn = await createQueue(
      kmsg,
      `kevent-kmsg-dv-${tag}`,
      appId,
      ownerUserId,
    );
    const firstPayload = { case: "DV-03", tag, value: "durable-message" };
    const firstIndex = await rpcCall<number>(kmsg, "post_message", {
      queue_urn: queueUrn,
      message: {
        index: 0,
        created_at: 0,
        payload: encodePayload(firstPayload),
        headers: { test_case: "DV-03" },
      },
    });
    assert.ok(firstIndex > 0, "post_message should return a positive index");

    const subId = await rpcCall<string>(kmsg, "subscribe", {
      queue_urn: queueUrn,
      userid: userId,
      appid: appId,
      sub_id: `kevent-kmsg-dv-sub-${tag}`,
      position: "Earliest",
    });
    const fetched = asMessages(
      await kmsg.call("fetch_messages", {
        sub_id: subId,
        length: 10,
        auto_commit: false,
      }),
    );
    assert.equal(
      fetched.length,
      1,
      "kmsg fetch should return the first message",
    );
    assert.equal(fetched[0].index, firstIndex);
    assert.equal(
      (decodePayload(fetched[0]) as JsonObject).value,
      "durable-message",
    );
    await kmsg.call("commit_ack", { sub_id: subId, index: firstIndex });

    const stats = await rpcCall<QueueStats>(kmsg, "get_queue_stats", {
      queue_urn: queueUrn,
    });
    assert.equal(
      stats.message_count,
      1,
      "queue stats should count first message",
    );

    const reread = asMessages(
      await kmsg.call("read_message", {
        queue_urn: queueUrn,
        cursor: firstIndex,
        length: 1,
      }),
    );
    assert.equal(
      reread.length,
      1,
      "reconnect/read smoke should see durable data",
    );

    const eventid = `/kevent_kmsg/dv/${tag}`;
    stream = await openKeventStream(gatewayBaseUrl, eventid);
    const ack = await stream.readFrame(5000);
    assert.equal(ack.type, "ack", "kevent stream should ack");

    const signalPayload = { case: "DV-05", tag, value: "event-driven-message" };
    const signalIndex = await rpcCall<number>(kmsg, "post_message", {
      queue_urn: queueUrn,
      message: {
        index: 0,
        created_at: 0,
        payload: encodePayload(signalPayload),
        headers: { test_case: "DV-05" },
      },
    });
    await publishKevent(gatewayBaseUrl, eventid, {
      queue_urn: queueUrn,
      index: signalIndex,
      tag,
    });

    let eventFrame: JsonObject | null = null;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const frame = await stream.readFrame(5000);
      if (frame.type === "event") {
        eventFrame = frame;
        break;
      }
    }
    assert.ok(eventFrame, "kevent stream should receive published event");
    const event = asObject(eventFrame!.event, "kevent event");
    assert.equal(event.eventid, eventid);
    const eventData = asObject(event.data, "kevent event data");
    assert.equal(eventData.queue_urn, queueUrn);
    assert.equal(eventData.index, signalIndex);

    const readSignal = asMessages(
      await kmsg.call("read_message", {
        queue_urn: queueUrn,
        cursor: signalIndex,
        length: 1,
      }),
    );
    assert.equal(
      readSignal.length,
      1,
      "kmsg durable payload should be readable after event",
    );
    assert.equal(
      (decodePayload(readSignal[0]) as JsonObject).value,
      "event-driven-message",
    );

    const signalFetched = asMessages(
      await kmsg.call("fetch_messages", {
        sub_id: subId,
        length: 10,
        auto_commit: false,
      }),
    );
    assert.equal(
      signalFetched.length,
      1,
      "signal message should be fetched once",
    );
    assert.equal(signalFetched[0].index, signalIndex);
    await kmsg.call("commit_ack", { sub_id: subId, index: signalIndex });

    await publishKevent(gatewayBaseUrl, eventid, {
      queue_urn: queueUrn,
      index: signalIndex,
      tag,
      duplicate: true,
    });
    let duplicateFrame: JsonObject | null = null;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const frame = await stream.readFrame(5000);
      if (frame.type === "event") {
        duplicateFrame = frame;
        break;
      }
    }
    assert.ok(duplicateFrame, "duplicate kevent should still be delivered");
    const afterDuplicate = asMessages(
      await kmsg.call("fetch_messages", {
        sub_id: subId,
        length: 10,
        auto_commit: false,
      }),
    );
    assert.equal(
      afterDuplicate.length,
      0,
      "duplicate kevent must not reprocess an acked kmsg message",
    );

    const fallbackPayload = { case: "DV-05", tag, value: "polling-fallback" };
    const fallbackIndex = await rpcCall<number>(kmsg, "post_message", {
      queue_urn: queueUrn,
      message: {
        index: 0,
        created_at: 0,
        payload: encodePayload(fallbackPayload),
        headers: { test_case: "DV-05-fallback" },
      },
    });
    const fallbackRead = asMessages(
      await kmsg.call("read_message", {
        queue_urn: queueUrn,
        cursor: fallbackIndex,
        length: 1,
      }),
    );
    assert.equal(
      fallbackRead.length,
      1,
      "polling fallback should read without event",
    );
    assert.equal(
      (decodePayload(fallbackRead[0]) as JsonObject).value,
      "polling-fallback",
    );

    await probeKmsgGet(gatewayBaseUrl);
    console.log(
      JSON.stringify({
        status: "passed",
        queue_urn: queueUrn,
        indexes: { firstIndex, signalIndex, fallbackIndex },
      }),
    );
  } finally {
    if (stream) stream.close();
    await cleanupQueue(kmsg, queueUrn);
  }
}

await run();
