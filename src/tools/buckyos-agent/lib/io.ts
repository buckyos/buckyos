// Input resource resolution + output artifact persistence.
//
// Input rules:
//   - "named_object:<obj_id>"          → ResourceRef::NamedObject
//   - "http(s)://..." / "data:..."     → ResourceRef::Url
//   - any local path                   → ResourceRef::Base64 (small/MVP) or
//                                        — when caller passes uploadNamed=true —
//                                        wrapped as a NamedObject via NDM.
//
// Output rules:
//   - artifact.resource.named_object   → download via ndm_proxy.openReader
//   - artifact.resource.url            → fetch
//   - artifact.resource.base64         → decode

import { AiArtifact, AiResponse, ResourceRef, aiResponseArtifacts } from "./types.ts";

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

export function isUrl(value: string): boolean {
  return /^(https?|data):/i.test(value);
}

export function isNamedObjectRef(value: string): boolean {
  return value.startsWith("named_object:") || value.startsWith("chunk:");
}

function base64FromBytes(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i]);
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
  if (isUrl(value)) {
    return { kind: "url", url: value, mime_hint: mimeHint };
  }
  // Local path → base64. Caller is responsible for choosing this path only for
  // files small enough to fit in the request envelope.
  const bytes = await Deno.readFile(value);
  const mime = mimeHint ?? mimeFromPath(value);
  return { kind: "base64", mime, data_base64: base64FromBytes(bytes) };
}

export interface SavedOutput {
  path: string;
  bytes: number;
  mime?: string;
  source_kind: string;
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

export async function saveArtifactToPath(
  artifact: AiArtifact,
  destPath: string,
  ndmProxy: NdmProxyClient,
): Promise<SavedOutput> {
  const sourceKind = artifact.resource?.kind ?? "unknown";
  let bytes: Uint8Array;
  let mime = artifactMime(artifact);

  if (sourceKind === "named_object" && artifact.resource.obj_id) {
    const r = await readNamedObject(ndmProxy, artifact.resource.obj_id);
    bytes = r.bytes;
    mime = r.mime ?? mime;
  } else if (sourceKind === "url" && artifact.resource.url) {
    const resp = await fetch(artifact.resource.url);
    if (!resp.ok) {
      throw new Error(
        `failed to download artifact from ${artifact.resource.url}: ${resp.status} ${resp.statusText}`,
      );
    }
    mime = resp.headers.get("content-type") ?? mime;
    bytes = new Uint8Array(await resp.arrayBuffer());
  } else if (sourceKind === "base64" && artifact.resource.data_base64) {
    bytes = bytesFromBase64(artifact.resource.data_base64);
  } else {
    throw new Error(`unsupported artifact resource kind: ${sourceKind}`);
  }

  await ensureParentDir(destPath);
  await Deno.writeFile(destPath, bytes);
  return { path: destPath, bytes: bytes.byteLength, mime, source_kind: sourceKind };
}

// Pick the first artifact whose mime matches the desired top-level family
// ("image" / "audio" / "video"). Falls back to first artifact when no match.
export function pickArtifact(
  response: AiResponse,
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
export function suffixPathByMime(path: string, mime: string | undefined): string {
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
