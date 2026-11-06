/**
 * `/ndn/` on a real Zone (bucket design §21 N9): through the Zone gateway and the
 * NodeGateway, with the tools the protocol was designed for. curl byte ranges against
 * the original file, including across a chunk boundary; ffprobe and ffmpeg on a file
 * larger than one chunk with its index at the end; HomeStation publishing and
 * withdrawing a public entry.
 *
 * Run: uv run test/run.py -p nfs_server_test, or cd test/nfs_server_test && pnpm run ndn
 * Env: BUCKYOS_TEST_ZONE_HOST (default test.buckyos.io), BUCKYOS_TEST_NODE_GATEWAY
 *      (default http://127.0.0.1:3180). A missing tool is reported as NOT VERIFIED.
 */
import { buckyos, ndn as sdkNdn } from "buckyos";

const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST") ?? "test.buckyos.io";
// A standalone nfs-server without authentication instead of a Zone (no gateway, no HomeStation).
const STANDALONE = Deno.env.get("BUCKYOS_TEST_NFS_BASE");
const ZONE = STANDALONE ?? `https://${zoneHost}`;
const NODE = STANDALONE ?? Deno.env.get("BUCKYOS_TEST_NODE_GATEWAY") ?? "http://127.0.0.1:3180";
const CHUNK = 32 * 1024 * 1024;

let session = "";
let user = "local";
if (!STANDALONE) {
  const { initTestRuntime } = await import("../test_helpers/buckyos_client.ts");
  const rt = await initTestRuntime();
  session = (await rt.buckyos.getAccountInfo())?.session_token as string;
  user = rt.userId as string;
}

let passed = 0;
const failures: string[] = [];
const unverified: string[] = [];
function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`assert failed: ${msg}`);
}
async function test(name: string, fn: () => Promise<void>) {
  try {
    await fn();
    passed++;
    console.log(`PASS ${name}`);
  } catch (e) {
    failures.push(`${name}: ${e instanceof Error ? e.message : e}`);
    console.log(`FAIL ${name}: ${e instanceof Error ? e.stack : e}`);
  }
}

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => {
  call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>>;
};
const nfs = new KRpcClient(`${ZONE}/kapi/nfs-server`, session);
const hs = new KRpcClient(`${ZONE}/kapi/homestation`, session);
const bearer = (): Record<string, string> => (session ? { "Authorization": `Bearer ${session}` } : {});

async function run(cmd: string, args: string[]): Promise<{ code: number; stdout: Uint8Array; stderr: string } | null> {
  try {
    const out = await new Deno.Command(cmd, { args, stdout: "piped", stderr: "piped" }).output();
    return { code: out.code, stdout: out.stdout, stderr: new TextDecoder().decode(out.stderr) };
  } catch (e) {
    if (e instanceof Deno.errors.NotFound) return null;
    throw e;
  }
}

/** One page: a large red rectangle a browser test can see on screen. */
function markerPdf(): string {
  const content = "1 0 0 rg 72 300 468 400 re f\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>",
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
  ];
  let out = "%PDF-1.4\n";
  const offsets = objects.map((body, i) => {
    const at = out.length;
    out += `${i + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = out.length;
  const rows = offsets.map((at) => `${String(at).padStart(10, "0")} 00000 n \n`).join("");
  return `${out}xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${rows}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
}

function equal(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

// A file larger than one chunk with `moov` after the media.
const dir = await Deno.makeTempDir({ prefix: "ndn_dv_" });
const moviePath = `${dir}/movie.mp4`;
const made = await run("ffmpeg", [
  "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=6:size=1280x720:rate=25,noise=alls=40:allf=t",
  "-f", "lavfi", "-i", "sine=frequency=440:duration=6", "-c:a", "aac", "-pix_fmt", "yuv420p",
  "-c:v", "libx264", "-preset", "ultrafast", "-crf", "4", "-t", "6", moviePath,
]);
if (made === null || made.code !== 0) {
  console.log(`NOT VERIFIED: ffmpeg cannot produce the fixture (${made?.stderr ?? "not installed"})`);
  Deno.exit(0);
}
const movie = await Deno.readFile(moviePath);
assert(movie.length > CHUNK, `fixture is ${movie.length} bytes, not more than one chunk`);
console.log(`[fixture] ${movie.length} bytes`);

const ndn = sdkNdn as unknown as {
  sha256Bytes(d: Uint8Array): Uint8Array;
  ChunkId: { fromMix256Result(len: number, hash: Uint8Array): { toString(): string } };
  SimpleChunkList: new () => { appendChunk(id: unknown): void; genObjId(): [{ toString(): string }, string] };
  FileObject: new (name: string, size: number, content: string) => { genObjId(): [{ toString(): string }, string] };
};

async function writeChunk(piece: Uint8Array): Promise<unknown> {
  const chunkObj = ndn.ChunkId.fromMix256Result(piece.length, ndn.sha256Bytes(piece));
  const chunkId = chunkObj.toString();
  const r = await fetch(`${ZONE}/ndm/proxy/v1/write/chunk/${chunkId}`, {
    method: "PUT",
    headers: { ...bearer(), "Content-Type": "application/octet-stream", "NDM-Chunk-Size": String(piece.length) },
    body: piece as BodyInit,
  });
  assert(r.ok, `write chunk ${r.status} ${await r.text()}`);
  return chunkObj;
}

const stamp = Date.now();
const scratch = `user/homestation/${user}/scratch`;
const key = `uploads/ndn-dv-${stamp}/movie.mp4`;
const list = new ndn.SimpleChunkList();
for (let at = 0; at < movie.length; at += CHUNK) list.appendChunk(await writeChunk(movie.subarray(at, Math.min(at + CHUNK, movie.length))));
const [listId, listBody] = list.genObjId();
const [fileObjId, fileBody] = new ndn.FileObject("movie.mp4", movie.length, listId.toString()).genObjId();
const fileId = fileObjId.toString();
const finalized = await fetch(`${ZONE}/ndm/v1/import/finalize`, {
  method: "POST",
  headers: { ...bearer(), "Content-Type": "application/json" },
  body: JSON.stringify({
    objects: [{ obj_id: listId.toString(), obj_data: listBody }, { obj_id: fileId, obj_data: fileBody }],
    roots: [fileId],
    auto_bind: { bucket_id: scratch, key, expires_at: stamp + 3600_000 },
  }),
});
assert(finalized.ok, `finalize ${finalized.status} ${await finalized.text()}`);

const issued = await nfs.call("bucket.create_access_token", { bucket_id: scratch, key, actions: ["read"], expires_at: Date.now() + 3600_000 });
const token = issued.token as string;
const hint = `hint=${encodeURIComponent(JSON.stringify({ token }))}`;

const bases = STANDALONE ? [["standalone", STANDALONE]] : [["zone gateway", ZONE], ["node gateway", NODE]];
for (const [where, base] of bases) {
  const url = `${base}/ndn/${fileId}?${hint}`;
  const k = base.startsWith("https") ? ["-k"] : [];

  await test(`${where}: no token, no read`, async () => {
    const r = await run("curl", [...k, "-s", "-o", "/dev/null", "-w", "%{http_code}", `${base}/ndn/${fileId}`]);
    assert(new TextDecoder().decode(r!.stdout) === "404", `anonymous ${new TextDecoder().decode(r!.stdout)}`);
  });

  await test(`${where}: curl ranges match the file byte for byte`, async () => {
    const head = await run("curl", [...k, "-s", "-r", "0-1", url]);
    assert(equal(head!.stdout, movie.subarray(0, 2)), "first two bytes");
    const tail = await run("curl", [...k, "-s", "-r", "-100", url]);
    assert(equal(tail!.stdout, movie.subarray(movie.length - 100)), "last hundred bytes");
    const from = CHUNK - 5000;
    const across = await run("curl", [...k, "-s", "-r", `${from}-${from + 9999}`, url]);
    assert(equal(across!.stdout, movie.subarray(from, from + 10000)), "across the chunk boundary");
    const whole = await run("curl", [...k, "-s", url]);
    assert(equal(whole!.stdout, movie), "the whole file");
    const past = await run("curl", [...k, "-s", "-o", "/dev/null", "-w", "%{http_code}", "-r", `${movie.length}-`, url]);
    assert(new TextDecoder().decode(past!.stdout) === "416", "past the end");
  });

  await test(`${where}: curl -I describes it`, async () => {
    const r = await run("curl", [...k, "-s", "-I", url]);
    const text = new TextDecoder().decode(r!.stdout).toLowerCase();
    assert(text.startsWith("http/") && text.split("\r\n")[0].includes("200"), text.split("\r\n")[0]);
    assert(text.includes(`content-length: ${movie.length}`), "length");
    assert(text.includes("accept-ranges: bytes"), "ranges");
    assert(text.includes("cache-control: private"), "a token read is private");
    assert(text.includes("x-content-type-options: nosniff"), "nosniff");
    assert(text.includes("referrer-policy: no-referrer"), "no-referrer");
  });

  const tlsless = base.startsWith("https") ? ["-tls_verify", "0"] : [];
  await test(`${where}: ffprobe reads the index at the end`, async () => {
    const r = await run("ffprobe", ["-v", "error", ...tlsless, "-print_format", "json", "-show_format", "-show_streams", url]);
    if (r === null) {
      unverified.push(`${where}: ffprobe not installed`);
      return;
    }
    assert(r.code === 0, `ffprobe ${r.stderr.replaceAll(token, "***")}`);
    const info = JSON.parse(new TextDecoder().decode(r.stdout));
    assert(Number(info.format.duration) >= 5, `duration ${info.format.duration}`);
    assert(info.streams.some((s: { codec_type: string }) => s.codec_type === "audio"), "audio stream");
  });

  await test(`${where}: ffmpeg -ss seeks without reading it all`, async () => {
    const out = `${dir}/frame-${where.split(" ")[0]}.jpg`;
    const r = await run("ffmpeg", ["-v", "error", "-y", ...tlsless, "-ss", "5", "-i", url, "-frames:v", "1", out]);
    assert(r!.code === 0, `ffmpeg ${r!.stderr.replaceAll(token, "***")}`);
    assert((await Deno.stat(out)).size > 0, "frame written");
  });
}

if (STANDALONE) {
  await test("a provided object is public; withdrawing closes it", async () => {
    const url = `${ZONE}/ndn/${fileId}`;
    const status = async () => (await fetch(url, { method: "HEAD" })).status;
    assert((await status()) === 404, "not public yet");
    await nfs.call("bucket.provide_public", { entry: "dv", root_obj_id: fileId, requested_by: user });
    const r = await fetch(url, { method: "HEAD" });
    assert(r.status === 200 && r.headers.get("cache-control")?.includes("public"), `public ${r.status}`);
    await nfs.call("bucket.withdraw_public", { entry: "dv" });
    assert((await status()) === 404, "withdrawn");
  });
} else await test("publishing a public entry opens /ndn/; narrowing and withdrawing close it", async () => {
  const publicUrl = `${ZONE}/ndn/${fileId}`;
  const status = async () => {
    const r = await run("curl", ["-k", "-s", "-o", "/dev/null", "-w", "%{http_code}", "-r", "0-1", publicUrl]);
    return new TextDecoder().decode(r!.stdout);
  };
  assert((await status()) === "404", "not public yet");
  await hs.call("object.register", { objId: fileId, metadata: { mime: "video/mp4" }, key });
  const attachments = [{ id: "movie", kind: "video", name: "movie.mp4", status: "uploaded", object: fileId }];
  const task = await hs.call("publish.create", { key: `ndn-dv-${stamp}`, input: { text: "ndn dv", attachments, audience: { kind: "public" } } });
  const entry = task.entry as string;
  assert(entry, `publish ${JSON.stringify(task)}`);
  // It goes out to followers and collectors like any post: never leave it behind.
  try {
    assert((await status()) === "206", `public entry ${await status()}`);
    const r = await fetch(publicUrl, { method: "HEAD" });
    assert(r.headers.get("cache-control")?.includes("public"), "a public read is cacheable");
    // A read token for this file dies with the narrowing too (G3).
    const tokenUrl = `${publicUrl}?${hint}`;
    const tokenStatus = async () => (await fetch(tokenUrl, { method: "HEAD" })).status;
    assert((await tokenStatus()) === 200, `token before narrowing ${await tokenStatus()}`);
    await hs.call("entry.set_audience", { entry, audience: { kind: "friends" } });
    assert((await status()) === "404", `narrowed ${await status()}`);
    assert((await tokenStatus()) === 404, `token after narrowing ${await tokenStatus()}`);
    await hs.call("entry.set_audience", { entry, audience: { kind: "public" } });
    assert((await status()) === "206", `widened ${await status()}`);
  } finally {
    await hs.call("entry.withdraw", { entry });
  }
  assert((await status()) === "404", `withdrawn ${await status()}`);
});

// What a real browser does with the same URLs (N2, N9); needs Desktop's Playwright.
if (!STANDALONE) {
  await test("a browser runs nothing it fetched from the Desktop origin and seeks a <video>", async () => {
    // On the Desktop origin, where the content is really embedded; the app itself navigates away.
    const HOST_PAGE = `${ZONE}/favicon.ico`;
    const page = '<!doctype html><script>window.__ran=1;document.title="RAN";alert(1)</script>';
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" onload="document.title=&quot;RAN&quot;"><script>window.__ran=1;document.title="RAN"</script></svg>';
    const cases: Record<string, unknown>[] = [{ name: "control: a page without the policy runs", url: "data:text/html,<script>document.title='RAN'</script>", expect: "runs" }];
    for (const [name, body, type] of [["x.html", page, "text/html"], ["x.svg", svg, "image/svg+xml"], ["x.pdf", markerPdf(), "application/pdf"], ["x.txt", "plain text", "text/plain"]]) {
      const putKey = `uploads/ndn-dv-${stamp}/${name}`;
      const put = await fetch(`${ZONE}/bucket/v1/${scratch}/${putKey}`, {
        method: "PUT",
        headers: { ...bearer(), "Content-Type": type, "x-bucky-expires-at": String(Date.now() + 3600_000) },
        body,
      });
      assert(put.ok, `put ${name} ${put.status}`);
      const putId = (await put.json()).obj_id as string;
      const read = await nfs.call("bucket.create_access_token", { bucket_id: scratch, key: putKey, actions: ["read"], expires_at: Date.now() + 3600_000 });
      const planeUrl = `${ZONE}/bucket/v1/${scratch}/${putKey}?token=${read.token}`;
      const ndnUrl = `${ZONE}/ndn/${putId}?hint=${encodeURIComponent(JSON.stringify({ token: read.token }))}`;
      if (type === "application/pdf") {
        for (const [route, url] of [["/bucket/v1", planeUrl], ["/ndn/", ndnUrl]]) {
          cases.push({ name: `${route} PDF is shown`, url, expect: "pdf" });
          cases.push({ name: `${route} PDF is shown in an iframe`, url, origin: HOST_PAGE, expect: "pdf" });
        }
      } else if (type === "text/plain") {
        cases.push({ name: "control: no PDF, no red", url: planeUrl, expect: "no_pdf" });
      } else {
        cases.push({ name: `/bucket/v1 ${type}`, url: planeUrl, expect: "no_run" });
        cases.push({ name: `/ndn/ ${type}`, url: ndnUrl, expect: "no_run" });
      }
    }
    // A fresh token: narrowing the public entry above revoked the first one.
    const fresh = await nfs.call("bucket.create_access_token", { bucket_id: scratch, key, actions: ["read"], expires_at: Date.now() + 3600_000 });
    const freshHint = encodeURIComponent(JSON.stringify({ token: fresh.token }));
    cases.push({ name: "<video> seeks over /ndn/", origin: HOST_PAGE, url: `${ZONE}/ndn/${fileId}?hint=${freshHint}`, expect: "seek" });
    const urls = `${dir}/urls.json`;
    await Deno.writeTextFile(urls, JSON.stringify(cases));
    const r = await run("node", [new URL("./ndn_browser.mjs", import.meta.url).pathname, urls]);
    if (r === null) {
      unverified.push("browser: node not installed");
      return;
    }
    console.log(new TextDecoder().decode(r.stdout));
    if (r.stderr.includes("MODULE_NOT_FOUND")) {
      unverified.push("browser: Playwright not installed in src/frame/desktop");
      return;
    }
    assert(r.code === 0, `browser ${r.stderr.slice(0, 400)}`);
  });
}

await Deno.remove(dir, { recursive: true });
if (STANDALONE) unverified.push("the gateway routes and HomeStation: run without BUCKYOS_TEST_NFS_BASE on a Zone");
console.log(`\n${passed} passed, ${failures.length} failed, ${unverified.length} not verified`);
for (const u of unverified) console.log(`NOT VERIFIED ${u}`);
for (const f of failures) console.log(`FAILED ${f}`);
Deno.exit(failures.length ? 1 : 0);
