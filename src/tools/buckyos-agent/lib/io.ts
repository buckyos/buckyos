// Input resource resolution + output artifact delivery.
//
// Input rules:
//   - "named_object:<obj_id>"          → ResourceRef::NamedObject
//   - "obj://bucket/<bucket_id>/<key>" → ResourceRef::NamedObject of what it binds
//   - "http(s)://..." / "data:..."     → ResourceRef::Url
//   - any local path                   → ResourceRef::Base64 (small/MVP)
//
// Output rules (deliverArtifact): every result goes into this Agent's
// `artifacts` bucket and is named by its `obj://bucket/...`:
//   - stored by AICC (its `outputs` bucket) → bucket.copy, no bytes move
//   - otherwise the bytes are read and uploaded (lib/bucket.ts uploadNamed):
//       named_object → ndm_proxy.openReader
//       url          → AICC Provider reader, then public fetch fallback
//       base64       → decode
// A local file is written only when the caller gives a path (the explicit
// fallback for tools that need one; `materialize_resource` does the same later).

import { buckyos } from "buckyos";

import {
  boundObjectId,
  copyToArtifacts,
  parseBucketObjUrl,
  readBucketObject,
  uploadNamed,
} from "./bucket.ts";
import type { AiccRuntime } from "./runtime.ts";
import {
  AiArtifact,
  AiccInferenceResponse,
  aiResponseArtifacts,
  ResourceRef,
} from "./types.ts";

const MIME_BY_EXT: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
  bmp: "image/bmp",
  pdf: "application/pdf",
  txt: "text/plain",
  json: "application/json",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  ogg: "audio/ogg",
  m4a: "audio/mp4",
  flac: "audio/flac",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

const EXT_BY_MIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/jpg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
  "audio/mpeg": "mp3",
  "audio/mp3": "mp3",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/ogg": "ogg",
  "video/mp4": "mp4",
  "video/webm": "webm",
  "video/quicktime": "mov",
  "application/pdf": "pdf",
  "text/plain": "txt",
  "application/json": "json",
};

export function mimeFromPath(path: string): string {
  const m = path.toLowerCase().match(/\.([a-z0-9]+)$/);
  if (!m) return "application/octet-stream";
  return MIME_BY_EXT[m[1]] ?? "application/octet-stream";
}

export function extFromMime(mime: string | undefined | null): string | null {
  if (!mime) return null;
  const k = mime.split(";")[0].trim().toLowerCase();
  return EXT_BY_MIME[k] ?? k.split("/").at(-1) ?? null;
}

function isRawPcm(mime: string | null | undefined): boolean {
  const type = (mime ?? "").split(";")[0].trim().toLowerCase();
  return type === "audio/l16" || type === "audio/pcm";
}

// Raw 16-bit PCM (`audio/l16`, `audio/pcm`; Gemini TTS sends little-endian
// samples, 24 kHz mono) plays nowhere: wrap it in a WAV header. Rate and
// channels come from the mime parameters when present.
export function wrapRawPcm(
  bytes: Uint8Array,
  mime: string | undefined,
): Uint8Array | null {
  if (!isRawPcm(mime)) return null;
  const params = (mime ?? "").split(";").slice(1).map((p) => p.trim().toLowerCase());
  const param = (name: string) =>
    Number(params.find((p) => p.startsWith(`${name}=`))?.split("=")[1]);
  const rate = param("rate") || 24000;
  const channels = param("channels") || 1;
  const out = new Uint8Array(44 + bytes.byteLength);
  const v = new DataView(out.buffer);
  const ascii = (at: number, s: string) => {
    for (let i = 0; i < s.length; i += 1) out[at + i] = s.charCodeAt(i);
  };
  ascii(0, "RIFF");
  v.setUint32(4, 36 + bytes.byteLength, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, channels, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * channels * 2, true);
  v.setUint16(32, channels * 2, true);
  v.setUint16(34, 16, true);
  ascii(36, "data");
  v.setUint32(40, bytes.byteLength, true);
  out.set(bytes, 44);
  return out;
}

export function isUrl(value: string): boolean {
  return /^(https?|data):/i.test(value);
}

export function isNamedObjectRef(value: string): boolean {
  return value.startsWith("named_object:") || value.startsWith("cyfile:") ||
    value.startsWith("chunk:");
}

export function isBucketObjectRef(value: string): boolean {
  return parseBucketObjUrl(value) !== null;
}

function base64FromBytes(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

function bytesFromBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
  return out;
}

export async function resolveInputResource(
  value: string,
  mimeHint?: string,
): Promise<ResourceRef> {
  if (isNamedObjectRef(value)) {
    const objId = value.startsWith("named_object:")
      ? value.slice("named_object:".length)
      : value;
    return { kind: "named_object", obj_id: objId };
  }
  if (isBucketObjectRef(value)) {
    return { kind: "named_object", obj_id: await boundObjectId(value) };
  }
  if (isUrl(value)) {
    return { kind: "url", url: value, mime_hint: mimeHint };
  }
  // Local path → base64. Caller is responsible for choosing this path only for
  // files small enough to fit in the request envelope.
  const bytes = await Deno.readFile(value);
  const inferredMime = mimeFromPath(value);
  const mime = mimeHint?.endsWith("/*")
    ? inferredMime
    : mimeHint ?? inferredMime;
  return { kind: "base64", mime, data_base64: base64FromBytes(bytes) };
}

export interface SavedOutput {
  path: string;
  bytes: number;
  mime?: string;
  source_kind: string;
  // The typed object id (e.g. `cyfile:…`) when the resource is a named object:
  // pass it on as `named_object:<id>` or list it as a report artifact.
  obj_id?: string;
}

export interface SaveResourceOptions {
  overwrite?: boolean;
}

async function ensureParentDir(path: string): Promise<void> {
  const idx = path.lastIndexOf("/");
  if (idx <= 0) return;
  await Deno.mkdir(path.slice(0, idx), { recursive: true });
}

function artifactMime(a: AiArtifact): string | undefined {
  return a.mime ?? a.resource.mime ?? a.resource.mime_hint ?? undefined;
}

// deno-lint-ignore no-explicit-any
type NdmProxyClient = any;

async function readNamedObject(
  ndmProxy: NdmProxyClient,
  objId: string,
): Promise<{ bytes: Uint8Array; mime?: string }> {
  const opened = await ndmProxy.openReader({ obj_id: objId });
  const bytes = new Uint8Array(await opened.response.arrayBuffer());
  const mime = opened.response.headers.get("content-type") ?? undefined;
  return { bytes, mime };
}

async function readUrlArtifact(
  url: string,
): Promise<{ bytes: Uint8Array; mime?: string }> {
  const serviceUrl = buckyos.getZoneServiceURL("aicc").replace(/\/$/, "");
  const rpcClient = buckyos.getServiceRpcClient("aicc");
  const token = rpcClient.getSessionToken();
  if (!token) throw new Error("AICC session token is unavailable");
  const aiccResponse = await fetch(`${serviceUrl}/artifact/open`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Auth": token,
    },
    body: JSON.stringify({ url }),
  });
  if (aiccResponse.ok) {
    return {
      bytes: new Uint8Array(await aiccResponse.arrayBuffer()),
      mime: aiccResponse.headers.get("content-type") ?? undefined,
    };
  }
  if (aiccResponse.status !== 404) {
    const detail = (await aiccResponse.text()).trim().slice(0, 4096);
    throw new Error(
      `AICC failed to open Provider artifact: ${aiccResponse.status} ${aiccResponse.statusText}${
        detail ? `: ${detail}` : ""
      }`,
    );
  }
  const publicResponse = await fetch(url);
  if (!publicResponse.ok) {
    throw new Error(
      `failed to download public artifact: ${publicResponse.status} ${publicResponse.statusText}`,
    );
  }
  return {
    bytes: new Uint8Array(await publicResponse.arrayBuffer()),
    mime: publicResponse.headers.get("content-type") ?? undefined,
  };
}

export async function saveArtifactToPath(
  artifact: AiArtifact,
  destPath: string,
  ndmProxy: NdmProxyClient,
): Promise<SavedOutput> {
  return saveResourceToPath(artifact.resource, destPath, ndmProxy, {
    overwrite: true,
  }, artifactMime(artifact));
}

// The bytes of a resource (and its mime, when the source says it).
export async function readResource(
  resource: ResourceRef,
  ndmProxy: NdmProxyClient,
  mimeHint?: string,
): Promise<{ bytes: Uint8Array; mime?: string }> {
  const mime = mimeHint ?? resource.mime ?? resource.mime_hint ?? undefined;
  if (resource.kind === "named_object") {
    const r = await readNamedObject(ndmProxy, resource.obj_id);
    return { bytes: r.bytes, mime: mime ?? r.mime };
  }
  if (resource.kind === "url") {
    const r = await readUrlArtifact(resource.url);
    return { bytes: r.bytes, mime: r.mime ?? mime };
  }
  if (resource.kind === "base64") {
    return { bytes: bytesFromBase64(resource.data_base64), mime };
  }
  throw new Error(`unsupported artifact resource kind: ${(resource as ResourceRef).kind}`);
}

function wavPath(path: string): string {
  return `${path.replace(/\.[a-z0-9]{1,8}$/i, "")}.wav`;
}

export async function saveResourceToPath(
  resource: ResourceRef,
  destPath: string,
  ndmProxy: NdmProxyClient,
  options: SaveResourceOptions = {},
  mimeHint?: string,
): Promise<SavedOutput> {
  let { bytes, mime } = await readResource(resource, ndmProxy, mimeHint);
  const wav = wrapRawPcm(bytes, mime);
  if (wav) {
    bytes = wav;
    mime = "audio/wav";
    destPath = wavPath(destPath);
  }
  await writeBytesToPath(destPath, bytes, options);
  return {
    path: destPath,
    bytes: bytes.byteLength,
    mime,
    source_kind: resource.kind,
    ...(resource.kind === "named_object" ? { obj_id: resource.obj_id } : {}),
  };
}

// A bucket object (`obj://bucket/...`) saved as a local file.
export async function saveBucketObjectToPath(
  url: string,
  destPath: string,
  options: SaveResourceOptions = {},
): Promise<SavedOutput> {
  const { bytes, mime } = await readBucketObject(url);
  await writeBytesToPath(destPath, bytes, options);
  return { path: destPath, bytes: bytes.byteLength, mime, source_kind: "bucket", obj_id: await boundObjectId(url) };
}

export async function writeBytesToPath(
  destPath: string,
  bytes: Uint8Array,
  options: SaveResourceOptions = {},
): Promise<void> {
  await ensureParentDir(destPath);
  const exclusive = options.overwrite === false;
  let file: Deno.FsFile | undefined;
  try {
    file = await Deno.open(destPath, {
      write: true,
      create: !exclusive,
      createNew: exclusive,
      truncate: !exclusive,
    });
    let offset = 0;
    while (offset < bytes.byteLength) {
      const written = await file.write(bytes.subarray(offset));
      if (written === 0) throw new Error("resource write made no progress");
      offset += written;
    }
  } catch (err) {
    if (file && exclusive) {
      try {
        await Deno.remove(destPath);
      } catch {
      }
    }
    throw err;
  } finally {
    file?.close();
  }
}

// What AICC says about a result it stored (`extra.materialized_artifacts`):
// its name, mime and the `obj://bucket/...` binding in AICC's `outputs`.
export function aiccStoredArtifact(
  response: AiccInferenceResponse,
  objId: string,
): { bucket_ref?: string; mime?: string; name?: string } | undefined {
  const extra = (response as unknown as { extra?: { materialized_artifacts?: unknown } }).extra;
  const list = Array.isArray(extra?.materialized_artifacts) ? extra.materialized_artifacts : [];
  const entry = list.find((e) =>
    e && typeof e === "object" && (e as Record<string, unknown>).obj_id === objId
  ) as Record<string, unknown> | undefined;
  if (!entry) return undefined;
  const text = (v: unknown) => typeof v === "string" && v ? v : undefined;
  return { bucket_ref: text(entry.bucket_ref), mime: text(entry.mime), name: text(entry.name) };
}

export type DeliveredOutput = {
  // `obj://bucket/<this Agent's artifacts>/<key>`: hand this over.
  url: string;
  obj_id: string;
  // The local copy, written only when a path was given.
  path: string | null;
  bytes: number | null;
  mime: string | null;
  source_kind: string;
};

export function deliveredSummary(tool: string, files: DeliveredOutput[]): string {
  const written = files.flatMap((f) => f.path ? [f.path] : []);
  return `${tool} kept ${files.map((f) => f.url).join(", ")}${
    written.length ? `; wrote ${written.join(", ")}` : ""
  }`;
}

function deliveryName(tool: string, artifact: AiArtifact, mime: string | null): string {
  const base = `${tool}-${artifact.name || "result"}`.replace(/[^A-Za-z0-9._-]+/g, "-");
  return suffixPathByMime(base, mime ?? undefined);
}

// deliverArtifact: keep an AICC result in this Agent's `artifacts` bucket and
// optionally write a local copy at `path` (an extension is added from the
// mime when it has none; raw PCM becomes WAV). An AICC-stored result is
// copied from AICC's `outputs` without moving bytes; anything else is
// uploaded once.
export async function deliverArtifact(
  runtime: AiccRuntime,
  response: AiccInferenceResponse,
  artifact: AiArtifact,
  ndmProxy: NdmProxyClient,
  options: { tool: string; path?: string; defaultMime?: string },
): Promise<DeliveredOutput> {
  const stored = artifact.resource.kind === "named_object"
    ? aiccStoredArtifact(response, artifact.resource.obj_id)
    : undefined;
  let mime = artifactMime(artifact) ?? stored?.mime ?? null;
  let dest = options.path
    ? suffixPathByMime(options.path, mime ?? options.defaultMime)
    : undefined;
  let bytes: Uint8Array | undefined;
  let kept;
  if (stored?.bucket_ref && !isRawPcm(mime)) {
    const name = dest ? dest.split("/").at(-1)! : deliveryName(options.tool, artifact, mime ?? options.defaultMime ?? null);
    kept = await copyToArtifacts(runtime, stored.bucket_ref, name);
  } else {
    const read = stored?.bucket_ref
      ? await readBucketObject(stored.bucket_ref)
      : await readResource(artifact.resource, ndmProxy, mime ?? undefined);
    bytes = read.bytes;
    mime = mime ?? read.mime ?? null;
    const wav = wrapRawPcm(bytes, mime ?? undefined);
    if (wav) {
      bytes = wav;
      mime = "audio/wav";
      dest = dest ? wavPath(dest) : undefined;
    }
    const name = dest ? dest.split("/").at(-1)! : deliveryName(options.tool, artifact, mime ?? options.defaultMime ?? null);
    kept = await uploadNamed(runtime, bytes, name, mime ? { mime } : {});
  }
  if (dest) {
    bytes = bytes ?? (await readBucketObject(kept.url)).bytes;
    await writeBytesToPath(dest, bytes, { overwrite: true });
  }
  return {
    url: kept.url,
    obj_id: kept.obj_id,
    path: dest ?? null,
    bytes: kept.bytes ?? bytes?.byteLength ?? null,
    mime: mime ?? kept.mime,
    source_kind: artifact.resource.kind,
  };
}

// Pick the first artifact whose mime matches the desired top-level family
// ("image" / "audio" / "video"). Falls back to first artifact when no match.
export function pickArtifact(
  response: AiccInferenceResponse,
  family?: "image" | "audio" | "video",
): AiArtifact | null {
  const arts = aiResponseArtifacts(response);
  if (arts.length === 0) return null;
  if (!family) return arts[0];
  for (const a of arts) {
    const mime = artifactMime(a) ?? "";
    if (mime.startsWith(`${family}/`)) return a;
  }
  return arts[0];
}

// Append an extension to `path` (chosen by `mime`) if the path has none.
// Used by single-file output commands so `gen_image foo bar` still produces
// `bar.png` rather than an extension-less blob.
export function suffixPathByMime(
  path: string,
  mime: string | undefined,
): string {
  if (/\.[a-z0-9]{1,8}$/i.test(path)) return path;
  const ext = extFromMime(mime ?? "");
  return ext ? `${path}.${ext}` : path;
}

export function writeTextFile(path: string, text: string): Promise<void> {
  return ensureParentDir(path).then(() => Deno.writeTextFile(path, text));
}

export function writeJsonFile(path: string, value: unknown): Promise<void> {
  return writeTextFile(path, `${JSON.stringify(value, null, 2)}\n`);
}
