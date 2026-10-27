// This Agent's buckets (declared by its template, owned by its user):
//   artifacts  documents, kept and backed up: what the Agent hands over
//   scratch    temp, 7 days: intermediate files
// A bucket object is named `obj://bucket/<bucket_id>/<key>`; report
// artifacts, `materialize_resource` and AICC inputs all take that form.
// Bytes go in once: an AICC result AICC already keeps (its `outputs` bucket,
// 7 days) is copied — a second binding of the same object — not uploaded.

import { buckyos, ndm } from "buckyos";

import type { AiccRuntime } from "./runtime.ts";

export const ARTIFACTS_BUCKET = "artifacts";
export const SCRATCH_BUCKET = "scratch";
export const BUCKET_URL_PREFIX = "obj://bucket/";
// `PUT /bucket/v1/...` takes one request body up to this size.
const MAX_PUT_BYTES = 32 * 1024 * 1024;

export interface StoredObject {
  url: string;
  obj_id: string;
  bytes: number | null;
  mime: string | null;
}

// deno-lint-ignore no-explicit-any
type BucketClient = any;

export function bucketClient(): BucketClient {
  // deno-lint-ignore no-explicit-any
  return (buckyos as any).getBucketClient();
}

export function agentBucketId(
  runtime: AiccRuntime,
  name: string = ARTIFACTS_BUCKET,
): string {
  return `user/${runtime.appId}/${runtime.ownerUserId}/${name}`;
}

export function bucketObjUrl(bucketId: string, key: string): string {
  return `${BUCKET_URL_PREFIX}${bucketId}/${key}`;
}

export function parseBucketObjUrl(
  url: string,
): { bucketId: string; key: string } | null {
  if (!url.startsWith(BUCKET_URL_PREFIX)) return null;
  const parts = url.slice(BUCKET_URL_PREFIX.length).split("/");
  if (parts.length < 5 || (parts[0] !== "user" && parts[0] !== "zone")) {
    return null;
  }
  const key = parts.slice(4).join("/");
  return key ? { bucketId: parts.slice(0, 4).join("/"), key } : null;
}

// `<16 hex>/<name>`: named after the content, as OpenDAN keys report artifacts.
export function artifactKey(contentId: string, name: string): string {
  const hash = contentId.slice(contentId.indexOf(":") + 1);
  const flat = name.replace(/[/\\]/g, "_") || "artifact";
  return `${hash.slice(0, 16)}/${flat}`;
}

function errorCode(err: unknown): string | undefined {
  const code = (err as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : undefined;
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// A content-named key already exists: the same object stored again.
async function sameObject(
  bucketId: string,
  key: string,
  objId?: string,
): Promise<{ obj_id: string; size: number }> {
  const existing = await bucketClient().get(bucketId, key);
  if (!existing?.obj_id || (objId && existing.obj_id !== objId)) {
    throw new Error(`${bucketObjUrl(bucketId, key)} already binds another object`);
  }
  return { obj_id: existing.obj_id, size: existing.size };
}

function serviceBase(): { endpoint: string; sessionToken: string | null } {
  const url = buckyos.getZoneServiceURL("nfs-server");
  const at = url.indexOf("/kapi/");
  return {
    endpoint: at >= 0 ? url.slice(0, at) : url,
    sessionToken: buckyos.getServiceRpcClient("nfs-server").getSessionToken() ??
      null,
  };
}

// uploadNamed: bytes into one of this Agent's buckets (`artifacts` unless
// `bucket` says otherwise), bound in the same request; success means the
// bucket retains them. Up to 32 MiB through `PUT /bucket/v1/...`, larger
// through the NDM upload that binds at finalize.
export async function uploadNamed(
  runtime: AiccRuntime,
  bytes: Uint8Array,
  name: string,
  options: { mime?: string; bucket?: string } = {},
): Promise<StoredObject> {
  const bucketId = agentBucketId(runtime, options.bucket);
  const mime = options.mime ?? null;
  if (bytes.byteLength <= MAX_PUT_BYTES) {
    const key = artifactKey(await sha256Hex(bytes), name);
    let objId: string;
    try {
      const put = await bucketClient().putBytes(bucketId, key, bytes, {
        ...(mime ? { contentType: mime } : {}),
        if_none_match: "*",
      });
      objId = put.obj_id;
    } catch (err) {
      if (errorCode(err) !== "precondition_failed") throw err;
      objId = (await sameObject(bucketId, key)).obj_id;
    }
    return { url: bucketObjUrl(bucketId, key), obj_id: objId, bytes: bytes.byteLength, mime };
  }
  const { endpoint, sessionToken } = serviceBase();
  const file = new File([bytes as BlobPart], name, mime ? { type: mime } : {});
  // deno-lint-ignore no-explicit-any
  const sdk = ndm as any;
  const snapshot = await sdk.importFiles([file], { mode: "single_file", endpoint, sessionToken });
  try {
    const key = artifactKey(snapshot.selection.objectId, name);
    const status = await sdk.ensureUploaded(snapshot.sessionId, {
      endpoint,
      sessionToken,
      bind: { bucketId, key, ...(mime ? { contentType: mime } : {}) },
    });
    return { url: bucketObjUrl(bucketId, key), obj_id: status.binding.obj_id, bytes: bytes.byteLength, mime };
  } finally {
    sdk.releaseImportSession(snapshot.sessionId);
  }
}

// Keep what `url` binds in `artifacts`: as it is when it is there already,
// otherwise copied under `<hash>/<name>` (no bytes move).
export async function copyToArtifacts(
  runtime: AiccRuntime,
  url: string,
  name?: string,
): Promise<StoredObject> {
  const src = parseBucketObjUrl(url);
  if (!src) throw new Error(`not an obj://bucket/ reference: ${url}`);
  const client = bucketClient();
  const binding = await client.get(src.bucketId, src.key);
  if (!binding?.obj_id) throw new Error(`${url} binds no object`);
  const dst = agentBucketId(runtime);
  const found = {
    obj_id: binding.obj_id as string,
    bytes: typeof binding.size === "number" ? binding.size : null,
    mime: binding.content_type ?? null,
  };
  if (src.bucketId === dst) return { url, ...found };
  const key = artifactKey(binding.obj_id, name ?? src.key.split("/").at(-1)!);
  try {
    await client.copy({
      src_bucket_id: src.bucketId,
      src_key: src.key,
      src_version: binding.version,
      dst_bucket_id: dst,
      dst_key: key,
      if_none_match: "*",
    });
  } catch (err) {
    if (errorCode(err) !== "precondition_failed") throw err;
    await sameObject(dst, key, binding.obj_id);
  }
  return { url: bucketObjUrl(dst, key), ...found };
}

// The object id a bucket object binds (to pass it on as an AICC input).
export async function boundObjectId(url: string): Promise<string> {
  const src = parseBucketObjUrl(url);
  if (!src) throw new Error(`not an obj://bucket/ reference: ${url}`);
  const binding = await bucketClient().get(src.bucketId, src.key);
  if (!binding?.obj_id) throw new Error(`${url} binds no object`);
  return binding.obj_id;
}

// The bytes of a bucket object, through the `/bucket/v1` data plane.
export async function readBucketObject(
  url: string,
): Promise<{ bytes: Uint8Array; mime?: string }> {
  const src = parseBucketObjUrl(url);
  if (!src) throw new Error(`not an obj://bucket/ reference: ${url}`);
  const response = await bucketClient().fetchObject(src.bucketId, src.key);
  return {
    bytes: new Uint8Array(await response.arrayBuffer()),
    mime: response.headers.get("content-type") ?? undefined,
  };
}
