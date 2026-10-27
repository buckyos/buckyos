import { buckyos } from "buckyos";

import { agentBucketId, artifactKey, parseBucketObjUrl } from "./bucket.ts";
import { aiccStoredArtifact, deliverArtifact } from "./io.ts";
import type { AiccRuntime } from "./runtime.ts";
import type { AiArtifact, AiccInferenceResponse } from "./types.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`expected ${expectedJson}, got ${actualJson}`);
  }
}

const runtime = { appId: "agentx", ownerUserId: "alice" } as AiccRuntime;
const ARTIFACTS = "user/agentx/alice/artifacts";
const OUTPUT = "obj://bucket/user/aicc/alice/outputs/agentx/0123456789abcdef/image_1.png";

// A bucket service of its own: what `buckyos.getBucketClient()` returns.
function fakeBuckets(existing: Record<string, string> = {}) {
  const calls: Array<[string, unknown]> = [];
  const bound: Record<string, string> = { ...existing };
  const client = {
    get(bucketId: string, key: string) {
      calls.push(["get", `${bucketId}/${key}`]);
      const objId = bound[`${bucketId}/${key}`];
      if (!objId) throw Object.assign(new Error("no such key"), { code: "no_such_key" });
      return Promise.resolve({ obj_id: objId, version: 3, size: 9, content_type: "image/png" });
    },
    copy(req: Record<string, unknown>) {
      calls.push(["copy", req]);
      const dst = `${req.dst_bucket_id}/${req.dst_key}`;
      if (bound[dst]) {
        throw Object.assign(new Error("exists"), { code: "precondition_failed" });
      }
      bound[dst] = bound[`${req.src_bucket_id}/${req.src_key}`];
      return Promise.resolve({ version: 1, etag: bound[dst], created: true });
    },
    putBytes(bucketId: string, key: string, body: Uint8Array, options: Record<string, unknown>) {
      calls.push(["putBytes", { key, bytes: body.byteLength, ...options }]);
      bound[`${bucketId}/${key}`] = "cyfile:uploaded";
      return Promise.resolve({ obj_id: "cyfile:uploaded", version: 1, etag: "cyfile:uploaded", created: true });
    },
  };
  // deno-lint-ignore no-explicit-any
  (buckyos as any).getBucketClient = () => client;
  return { calls, bound };
}

const generated = {
  task_id: "t-1",
  status: "succeeded",
  images: [{ kind: "named_object", obj_id: "cyfile:generated" }],
  extra: {
    materialized_artifacts: [{
      obj_id: "cyfile:generated",
      name: "image_1.png",
      mime: "image/png",
      bucket_ref: OUTPUT,
    }],
  },
} as unknown as AiccInferenceResponse;

Deno.test("bucket references parse and keys are named after the content", () => {
  assertEquals(agentBucketId(runtime), ARTIFACTS);
  assertEquals(agentBucketId(runtime, "scratch"), "user/agentx/alice/scratch");
  assertEquals(parseBucketObjUrl(`obj://bucket/${ARTIFACTS}/a/b.png`), {
    bucketId: ARTIFACTS,
    key: "a/b.png",
  });
  assertEquals(parseBucketObjUrl(`obj://bucket/${ARTIFACTS}`), null);
  assertEquals(parseBucketObjUrl("cyfile:abc"), null);
  assertEquals(artifactKey("cyfile:0123456789abcdef99", "a/b.png"), "0123456789abcdef/a_b.png");
  assertEquals(aiccStoredArtifact(generated, "cyfile:generated")?.bucket_ref, OUTPUT);
  assertEquals(aiccStoredArtifact(generated, "cyfile:other"), undefined);
});

Deno.test("an AICC-stored result is copied into artifacts, not uploaded", async () => {
  const { calls, bound } = fakeBuckets({ [OUTPUT.slice("obj://bucket/".length)]: "cyfile:generated" });
  const artifact: AiArtifact = { name: "image_1", resource: { kind: "named_object", obj_id: "cyfile:generated" } };
  const delivered = await deliverArtifact(runtime, generated, artifact, null, { tool: "gen_image" });
  assertEquals(delivered, {
    url: `obj://bucket/${ARTIFACTS}/generated/gen_image-image_1.png`,
    obj_id: "cyfile:generated",
    path: null,
    bytes: 9,
    mime: "image/png",
    source_kind: "named_object",
  });
  assertEquals(bound[`${ARTIFACTS}/generated/gen_image-image_1.png`], "cyfile:generated");
  assertEquals(calls.filter(([op]) => op === "putBytes").length, 0);
  const copy = calls.find(([op]) => op === "copy")![1] as Record<string, unknown>;
  assertEquals([copy.src_version, copy.if_none_match], [3, "*"]);
  // Delivered again: the same binding.
  const again = await deliverArtifact(runtime, generated, artifact, null, { tool: "gen_image" });
  assertEquals(again.url, delivered.url);
});

Deno.test("an inline result is uploaded once, raw PCM as WAV, with a local copy on request", async () => {
  const { calls } = fakeBuckets();
  const dir = await Deno.makeTempDir();
  try {
    const artifact: AiArtifact = {
      name: "audio",
      resource: { kind: "base64", mime: "audio/l16;rate=24000", data_base64: "AAABAA==" },
    };
    const response = { task_id: "t-2", status: "succeeded" } as unknown as AiccInferenceResponse;
    const delivered = await deliverArtifact(runtime, response, artifact, null, {
      tool: "text_to_speech",
      path: `${dir}/welcome.mp3`,
    });
    assertEquals(delivered.path, `${dir}/welcome.wav`);
    assertEquals(delivered.mime, "audio/wav");
    assertEquals(delivered.obj_id, "cyfile:uploaded");
    const put = calls.find(([op]) => op === "putBytes")![1] as Record<string, unknown>;
    assertEquals([put.bytes, put.contentType, put.if_none_match], [48, "audio/wav", "*"]);
    if (!String(put.key).endsWith("/welcome.wav")) throw new Error(`key ${put.key}`);
    assertEquals((await Deno.readFile(`${dir}/welcome.wav`)).byteLength, 48);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
