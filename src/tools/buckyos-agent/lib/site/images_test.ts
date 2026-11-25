import { Net } from "./extract.ts";
import { downloadImage, FULL_EDGE, MAX_ORIGINAL_BYTES, NotAPicture, prepareImage, sniffImage, THUMB_EDGE } from "./images.ts";
import type { Resize } from "./images.ts";
import { assert, assertEquals, assertFails, cannedFetch, tempDir, tinyPng } from "./testing.ts";

const bytes = (...parts: (number[] | string)[]) =>
  Uint8Array.from(parts.flatMap((part) => typeof part === "string" ? [...part].map((ch) => ch.charCodeAt(0)) : part));

function png(w: number, h: number): Uint8Array {
  const be = (n: number) => [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
  return bytes([0x89], "PNG\r\n\x1a\n", be(13), "IHDR", be(w), be(h), [8, 6, 0, 0, 0]);
}

Deno.test("the kind and the size of an image are read from its bytes", () => {
  assertEquals(sniffImage(tinyPng()), { ext: "png", w: 1, h: 1 });
  assertEquals(sniffImage(png(1920, 1080)), { ext: "png", w: 1920, h: 1080 });
  assertEquals(sniffImage(bytes("GIF89a", [0x20, 0x03, 0x58, 0x02])), { ext: "gif", w: 800, h: 600 });
  const jpeg = bytes([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0, 0xff, 0xc2, 0x00, 0x11, 8, 0x02, 0xd0, 0x05, 0x00, 3, 0, 0]);
  assertEquals(sniffImage(jpeg), { ext: "jpg", w: 1280, h: 720 });
  const vp8x = bytes("RIFF", [0, 0, 0, 0], "WEBPVP8X", [10, 0, 0, 0, 0, 0, 0, 0], [0xff, 0x04, 0x00], [0xcf, 0x02, 0x00]);
  assertEquals(sniffImage(vp8x), { ext: "webp", w: 1280, h: 720 });
  const avif = bytes([0, 0, 0, 0x1c], "ftypavif", [0, 0, 0, 0], "avifmif1", "....ispe", [0, 0, 0, 0], [0, 0, 5, 0], [0, 0, 2, 0xd0]);
  assertEquals(sniffImage(avif), { ext: "avif", w: 1280, h: 720 });
  assertEquals(sniffImage(new TextEncoder().encode("<svg xmlns='http://www.w3.org/2000/svg'/>")), null);
  assertEquals(sniffImage(new Uint8Array(0)), null);
});

Deno.test("with ImageMagick an image is shrunk to 1280 and gets a 480 one; a GIF becomes a PNG", async () => {
  const calls: [string, string, number][] = [];
  const resize: Resize = (_bytes, from, to, edge) => {
    calls.push([from, to, edge]);
    return Promise.resolve(edge === FULL_EDGE ? png(1280, 720) : png(480, 270));
  };
  const big = png(3840, 2160);
  const both = await prepareImage(big, resize, true);
  assertEquals([both.full.ext, both.full.w, both.full.h], ["png", 1280, 720]);
  assertEquals([both.thumb?.w, both.thumb?.h], [480, 270]);
  assertEquals(calls, [["png", "png", FULL_EDGE], ["png", "png", THUMB_EDGE]]);
  const only = await prepareImage(big, resize, false);
  assertEquals([only.thumb, calls.length], [null, 3]);
  calls.length = 0;
  await prepareImage(bytes("GIF89a", [0x20, 0x03, 0x58, 0x02]), resize, false);
  assertEquals(calls, [["gif", "png", FULL_EDGE]]);
});

Deno.test("without ImageMagick the original is kept up to 2 MiB and serves as its own small picture", async () => {
  const original = png(1600, 900);
  const kept = await prepareImage(original, null, true);
  assertEquals([kept.full.bytes === original, kept.thumb === kept.full, kept.full.w], [true, true, 1600]);
  const huge = new Uint8Array(MAX_ORIGINAL_BYTES + 1);
  huge.set(png(8000, 6000));
  await assertFails(() => prepareImage(huge, null, false), "without ImageMagick");
  // What is no picture of a kind the site keeps says so by its class: the caller leaves it out instead of trying again.
  for (const body of ["<html><body>Not found</body></html>", "<svg xmlns='http://www.w3.org/2000/svg'/>", "BM\u0000\u0000", ""]) {
    const err = await assertFails(() => prepareImage(new TextEncoder().encode(body), null, false), "not a JPEG");
    assert(err instanceof NotAPicture, `${JSON.stringify(body)}: ${err.name}`);
  }
  assert(!(await assertFails(() => prepareImage(huge, null, false), "without ImageMagick") instanceof NotAPicture), "too large is a picture");
});

Deno.test("an image is fetched with the article as its Referer and at most 8 MiB", async () => {
  const { fetcher, requests } = cannedFetch({
    "https://img.example.com/a.png": { body: tinyPng(), headers: { "content-type": "image/png" } },
    "https://img.example.com/huge.png": { body: new Uint8Array(8 * 1024 * 1024 + 1) },
    "https://img.example.com/gone.png": { status: 404 },
  });
  const net = new Net({ fetcher, sleep: () => Promise.resolve() });
  const got = await downloadImage(net, "https://img.example.com/a.png", "https://news.example.com/a/1");
  assertEquals(got, tinyPng());
  assertEquals(requests[0].headers.referer, "https://news.example.com/a/1");
  await assertFails(() => downloadImage(net, "https://img.example.com/huge.png", "https://news.example.com/a/1"), "more than");
  await assertFails(() => downloadImage(net, "https://img.example.com/gone.png", "https://news.example.com/a/1"), "HTTP 404");
  assert(requests.length === 3, "one request each");
});

Deno.test("a process that may not run magick keeps originals, exactly as when it is not installed", async () => {
  // The permission is taken away after the start (or the program vanished): the resize fails as Deno reports it.
  const original = png(1600, 900);
  for (const failure of [new Deno.errors.NotCapable("Requires run access to \"magick\""), new Deno.errors.PermissionDenied("denied"), new Deno.errors.NotFound("magick")]) {
    const kept = await prepareImage(original, () => Promise.reject(failure), true);
    assertEquals([kept.full.bytes === original, kept.thumb === kept.full], [true, true], failure.name);
  }
  // Any other failure of ImageMagick is a failure of this image.
  await assertFails(() => prepareImage(original, () => Promise.reject(new Error("magick: corrupt image")), false), "corrupt image");

  // What findResize says under the permissions a launcher can give. `magick` here is a stand-in that only answers -version.
  const dir = await tempDir({
    "probe.ts": `import { findResize } from ${JSON.stringify(new URL("./images.ts", import.meta.url).href)};\nconsole.log((await findResize()) ? "magick" : "none");\n`,
    "bin/magick": "#!/bin/sh\nexit 0\n",
    "bin/convert": "#!/bin/sh\nexit 0\n",
  });
  try {
    await Deno.chmod(`${dir}/bin/magick`, 0o755);
    await Deno.chmod(`${dir}/bin/convert`, 0o755);
    const config = new URL("../../deno.json", import.meta.url).pathname;
    const probe = async (permissions: string[], path: string) => {
      const out = await new Deno.Command(Deno.execPath(), {
        args: ["run", "--no-prompt", "--config", config, "--allow-read", "--allow-env", ...permissions, `${dir}/probe.ts`],
        env: { PATH: path, NO_COLOR: "1" },
        clearEnv: true,
        stdout: "piped",
        stderr: "piped",
      }).output();
      return `${out.code} ${new TextDecoder().decode(out.stdout).trim()}`;
    };
    assertEquals(await probe([], `${dir}/bin`), "0 none", "no run permission");
    assertEquals(await probe(["--deny-run"], `${dir}/bin`), "0 none", "run denied");
    assertEquals(await probe(["--allow-run=convert"], `${dir}/bin`), "0 none", "another program allowed, not magick");
    assertEquals(await probe(["--allow-run=magick"], "/nonexistent"), "0 none", "allowed but not installed");
    assertEquals(await probe(["--allow-run=magick"], `${dir}/bin`), "0 magick", "allowed and on PATH: nothing but magick is started");
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
