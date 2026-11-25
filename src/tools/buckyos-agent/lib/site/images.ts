// The images of an article: Personal Site Protocol §5.1 step 3. With
// ImageMagick on PATH an image is shrunk to a long edge of 1280 and stripped of
// its metadata, and the list gets a 480 one; without it the original is kept
// when it is at most 2 MiB.

import type { Net } from "./extract.ts";

export const MAX_DOWNLOAD_BYTES = 8 * 1024 * 1024;
export const MAX_ORIGINAL_BYTES = 2 * 1024 * 1024;
export const FULL_EDGE = 1280;
export const THUMB_EDGE = 480;

export type ImageExt = "jpg" | "png" | "gif" | "webp" | "avif";

export interface ImageInfo {
  ext: ImageExt;
  w: number;
  h: number;
}

export interface Prepared extends ImageInfo {
  bytes: Uint8Array;
}

/** The kind and size of an image, read from its first bytes; `null` when it is none of the kinds a site keeps. */
export function sniffImage(b: Uint8Array): ImageInfo | null {
  const ascii = (from: number, text: string) =>
    b.length >= from + text.length && [...text].every((ch, i) => b[from + i] === ch.charCodeAt(0));
  const be16 = (at: number) => (b[at] << 8) | b[at + 1];
  const be32 = (at: number) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;
  const le16 = (at: number) => b[at] | (b[at + 1] << 8);
  const le24 = (at: number) => b[at] | (b[at + 1] << 8) | (b[at + 2] << 16);

  if (b.length >= 24 && b[0] === 0x89 && ascii(1, "PNG")) return { ext: "png", w: be32(16), h: be32(20) };
  if (ascii(0, "GIF8") && b.length >= 10) return { ext: "gif", w: le16(6), h: le16(8) };
  if (b[0] === 0xff && b[1] === 0xd8) {
    let at = 2;
    while (at + 9 < b.length && b[at] === 0xff) {
      const marker = b[at + 1];
      if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) {
        return { ext: "jpg", w: be16(at + 7), h: be16(at + 5) };
      }
      at += 2 + be16(at + 2);
    }
    return { ext: "jpg", w: 0, h: 0 };
  }
  if (ascii(0, "RIFF") && ascii(8, "WEBP") && b.length >= 30) {
    if (ascii(12, "VP8X")) return { ext: "webp", w: 1 + le24(24), h: 1 + le24(27) };
    if (ascii(12, "VP8L")) {
      return {
        ext: "webp",
        w: 1 + (b[21] | ((b[22] & 0x3f) << 8)),
        h: 1 + ((b[22] >> 6) | (b[23] << 2) | ((b[24] & 0x0f) << 10)),
      };
    }
    if (ascii(12, "VP8 ")) return { ext: "webp", w: le16(26) & 0x3fff, h: le16(28) & 0x3fff };
  }
  if (ascii(4, "ftyp") && (ascii(8, "avif") || ascii(8, "avis"))) {
    for (let at = 12; at + 16 <= Math.min(b.length, 4096); at += 1) {
      if (ascii(at, "ispe")) return { ext: "avif", w: be32(at + 8), h: be32(at + 12) };
    }
    return { ext: "avif", w: 0, h: 0 };
  }
  return null;
}

/** What was fetched is not a picture of a kind the site keeps (an SVG icon, an error page, …). */
export class NotAPicture extends Error {
  constructor() {
    super("not a JPEG, PNG, GIF, WebP or AVIF image");
    this.name = "NotAPicture";
  }
}

/** Shrinks an image so its long edge is at most `edge` and removes its metadata. */
export type Resize = (bytes: Uint8Array, from: ImageExt, to: ImageExt, edge: number) => Promise<Uint8Array>;

async function magickResize(bytes: Uint8Array, from: ImageExt, to: ImageExt, edge: number): Promise<Uint8Array> {
  const dir = await Deno.makeTempDir({ prefix: "site-img-" });
  try {
    await Deno.writeFile(`${dir}/in.${from}`, bytes);
    const { success, stderr } = await new Deno.Command("magick", {
      args: [
        `${dir}/in.${from}[0]`,
        "-auto-orient",
        "-resize",
        `${edge}x${edge}>`,
        "-strip",
        ...(to === "jpg" ? ["-quality", "85"] : []),
        `${dir}/out.${to}`,
      ],
      stdout: "null",
      stderr: "piped",
    }).output();
    if (!success) throw new Error(`magick: ${new TextDecoder().decode(stderr).trim().split("\n").at(-1)}`);
    return await Deno.readFile(`${dir}/out.${to}`);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
}

/**
 * ImageMagick when this process may run it and it is on PATH. `magick`, by
 * that name, is the only program the site code ever starts: the launcher
 * grants `--allow-run=magick` and nothing else.
 */
export async function findResize(): Promise<Resize | null> {
  try {
    if ((await Deno.permissions.query({ name: "run", command: "magick" })).state !== "granted") return null;
    const { success } = await new Deno.Command("magick", { args: ["-version"], stdout: "null", stderr: "null" }).output();
    return success ? magickResize : null;
  } catch {
    return null;
  }
}

// Not allowed to run it, or it is not there: the same as having no ImageMagick.
function cannotRun(err: unknown): boolean {
  return err instanceof Deno.errors.NotCapable || err instanceof Deno.errors.PermissionDenied ||
    err instanceof Deno.errors.NotFound;
}

/** Fetches an image as a reader of `referer` would. */
export async function downloadImage(net: Net, url: string, referer: string): Promise<Uint8Array> {
  const response = await net.get(url, {
    headers: { referer, accept: "image/avif,image/webp,image/png,image/jpeg,image/gif;q=0.9,*/*;q=0.5" },
    maxBytes: MAX_DOWNLOAD_BYTES,
  });
  if (response.status !== 200) throw new Error(`${url}: HTTP ${response.status}`);
  return response.bytes;
}

/** What is stored for one downloaded image: the picture for the article and, when asked, the one for lists. */
export async function prepareImage(
  bytes: Uint8Array,
  resize: Resize | null,
  withThumb: boolean,
): Promise<{ full: Prepared; thumb: Prepared | null }> {
  const info = sniffImage(bytes);
  if (!info) throw new NotAPicture();
  if (resize) {
    const to: ImageExt = info.ext === "gif" ? "png" : info.ext === "avif" ? "jpg" : info.ext;
    const shrink = async (edge: number): Promise<Prepared> => {
      const out = await resize(bytes, info.ext, to, edge);
      const made = sniffImage(out);
      if (!made) throw new Error("ImageMagick wrote something that is not an image");
      return { ...made, bytes: out };
    };
    try {
      return { full: await shrink(FULL_EDGE), thumb: withThumb ? await shrink(THUMB_EDGE) : null };
    } catch (err) {
      if (!cannotRun(err)) throw err;
    }
  }
  if (bytes.byteLength > MAX_ORIGINAL_BYTES) {
    throw new Error(`${bytes.byteLength} bytes, and without ImageMagick an image is kept only up to ${MAX_ORIGINAL_BYTES}`);
  }
  const full = { ...info, bytes };
  return { full, thumb: withThumb ? full : null };
}
