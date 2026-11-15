// System rendering is cut off (Agent browser TODO §8.4): a page rendered by Workflow's `media.page` reaches no
// network and no cookie, and the render process is not the service's user.
//
//   deno run -A --unsafely-ignore-certificate-errors render_isolation.ts <site port> <site log dir> <report dir>
//
// The page asks the DV test site (site.mjs) on every address this host has for it, an outside address and an
// image, and writes what happened into itself; it then holds the render open for a few seconds so that the
// process can be caught with `ps`. Checks: the PDF says every request failed; the site's log has no request
// carrying this run's tag; the browser processes of the render belong to `nobody`.
import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import { ndm_proxy, ndn } from "buckyos";

const [portArg, siteLogDir, reportDir] = Deno.args;
if (!portArg || !siteLogDir || !reportDir) {
  console.error("usage: render_isolation.ts <site port> <site log dir> <report dir>");
  Deno.exit(2);
}
const port = Number(portArg);
const tag = `iso${Date.now().toString(36)}`;
await Deno.mkdir(reportDir, { recursive: true });

const targets: [string, string][] = [
  ["loopback", `http://127.0.0.1:${port}/leak?via=loopback&tag=${tag}`],
  ["docker-bridge", `http://172.17.0.1:${port}/leak?via=bridge&tag=${tag}`],
  ["outside", `https://example.com/?tag=${tag}`],
  ["zone-gateway", `http://127.0.0.1:3180/kapi/system_config?tag=${tag}`],
];
const html = `<!doctype html>
<html lang="zh"><head><meta charset="utf-8"><title>渲染隔离 DV ${tag}</title>
<style>body { font: 12pt/1.6 sans-serif; } pre { white-space: pre-wrap; }</style></head>
<body><h1>渲染隔离 DV ${tag}</h1>
<img id="img" src="http://172.17.0.1:${port}/pixel.png?via=img&tag=${tag}" alt="">
<pre id="out">PENDING</pre>
<script>
const targets = ${JSON.stringify(targets)};
(async () => {
  const lines = [];
  for (const [name, url] of targets) {
    try {
      const response = await fetch(url, { mode: 'no-cors', signal: AbortSignal.timeout(4000) });
      lines.push('FETCH ' + name + ': REACHED type=' + response.type + ' status=' + response.status);
    } catch (error) {
      lines.push('FETCH ' + name + ': FAILED ' + error.name);
    }
  }
  const img = document.getElementById('img');
  lines.push('IMAGE: ' + (img.complete && img.naturalWidth > 0 ? 'LOADED' : 'NOT-LOADED'));
  lines.push('COOKIE: [' + document.cookie + ']');
  try { localStorage.setItem('k', 'v'); lines.push('STORAGE: writable'); } catch (error) { lines.push('STORAGE: ' + error.name); }
  lines.push('LOCATION: ' + location.protocol + '//' + location.host);
  document.getElementById('out').textContent = lines.join('\\n');
  await new Promise((resolve) => setTimeout(resolve, 6000));
  window.__done = true;
})();
</script></body></html>`;
await Deno.writeTextFile(`${reportDir}/page.html`, html);

const { buckyos, zoneHost } = await initTestRuntime();
const sessionToken = (await buckyos.getAccountInfo())?.session_token ?? "";
const workflow = buckyos.getServiceRpcClient("workflow") as { call: (method: string, params: Record<string, unknown>) => Promise<any> };

// The page goes in as an object the caller holds (the way test/workflow_test/workflow_dv.ts uploads its fixtures).
const bytes = new TextEncoder().encode(html);
const chunkId = ndn.ChunkId.fromMix256Result(bytes.byteLength, ndn.sha256Bytes(bytes)).toString();
const proxy = ndm_proxy.createNdmProxyClient({
  endpoint: `https://${zoneHost}`,
  sessionToken,
  fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
    const target = typeof request === "string"
      ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
      : request instanceof URL
      ? new URL(request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"))
      : request;
    return fetch(target, init);
  },
});
await proxy.putChunk(chunkId, bytes);
const file = new ndn.FileObject(`${tag}-page.html`, bytes.byteLength, chunkId);
file.content_obj.create_time = Date.now();
const [objId, objStr] = file.genObjId();
await proxy.putObject({ obj_id: objId.toString(), obj_data: objStr });

const created = await workflow.call("create_run", {
  workflow_id: "media.page",
  input: { obj_id: objId.toString(), paper: "A4", margin_mm: 10, wait_ready: "window.__done === true" },
  auto_start: true,
  wait_ms: 1000,
});
if (created?.ok !== true) throw new Error(`create_run: ${JSON.stringify(created)}`);
console.log(`run ${created.run_id}`);

// While it renders: who owns the browser, and what its environment says (readable only to its own user or root).
const seen = new Map<string, string>();
const environ: Record<string, string> = {};
let graph: any;
for (let i = 0; i < 400; i++) {
  const ps = await new Deno.Command("ps", { args: ["-eo", "user:20,pid,ppid,args"], stdout: "piped" }).output();
  for (const line of new TextDecoder().decode(ps.stdout).split("\n")) {
    if (!/page-render|chrome|chromium/.test(line) || !/\/tmp\/wf-|page-render/.test(line)) continue;
    const [user, pid] = line.trim().split(/\s+/);
    if (seen.has(pid)) continue;
    seen.set(pid, line.trim().slice(0, 400));
    try {
      const text = new TextDecoder().decode(await Deno.readFile(`/proc/${pid}/environ`));
      environ[pid] = text.split("\0").map((entry) => entry.split("=")[0]).filter(Boolean).join(" ");
    } catch (error) {
      environ[pid] = `unreadable as ${Deno.env.get("USER")}: ${(error as Error).name} (process of ${user})`;
    }
  }
  graph = await workflow.call("get_run_graph", { run_id: created.run_id });
  if (["completed", "failed", "aborted", "budget_exhausted"].includes(graph?.status)) break;
  await new Promise((resolve) => setTimeout(resolve, 250));
}
await Deno.writeTextFile(`${reportDir}/graph.json`, JSON.stringify(graph, null, 2));
await Deno.writeTextFile(`${reportDir}/processes.txt`, [...seen.values()].join("\n") + "\n");
await Deno.writeTextFile(`${reportDir}/environ.json`, JSON.stringify(environ, null, 2));
console.log(`status ${graph?.status} ${JSON.stringify(graph?.node_errors ?? {})}`);
console.log(`processes seen: ${seen.size}`);
for (const line of seen.values()) console.log(`  ${line.slice(0, 160)}`);

const failures: string[] = [];
if (graph?.status !== "completed") failures.push(`the run ended ${graph?.status}`);
const pdfObjId = String(graph?.output?.page_to_pdf?.pdf_obj_id ?? "");
let text = "";
if (pdfObjId) {
  const opened = await fetch(`https://${zoneHost}/ndm/proxy/v1/read/object/open`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${sessionToken}` },
    body: JSON.stringify({ obj_id: pdfObjId }),
  });
  await Deno.writeFile(`${reportDir}/page.pdf`, new Uint8Array(await opened.arrayBuffer()));
  const out = await new Deno.Command(Deno.env.get("AB_DV_PDFTOTEXT") ?? "/opt/buckyos/tools/bin/pdftotext", { args: ["-layout", `${reportDir}/page.pdf`, "-"], stdout: "piped" }).output();
  text = new TextDecoder().decode(out.stdout);
  await Deno.writeTextFile(`${reportDir}/page.txt`, text);
  console.log(text.trim());
}
if (!text.includes(tag)) failures.push("the PDF does not show the page");
if (/REACHED|IMAGE: LOADED/.test(text)) failures.push("the page reached something");
if (!/FETCH loopback: FAILED/.test(text) || !/IMAGE: NOT-LOADED/.test(text)) failures.push("the PDF does not say the requests failed");
if (!/COOKIE: \[\]/.test(text)) failures.push("the page saw a cookie");

let leaked: string[] = [];
try {
  leaked = (await Deno.readTextFile(`${siteLogDir}/requests.jsonl`)).split("\n").filter((line) => line.includes(tag));
} catch { /* no log yet: nothing was asked */ }
if (leaked.length) failures.push(`the test site got ${leaked.length} request(s) of this run`);
console.log(`requests of this run at the test site: ${leaked.length}`);

const users = new Set([...seen.values()].filter((line) => /chrome|chromium/.test(line)).map((line) => line.split(/\s+/)[0]));
console.log(`browser process owners: ${[...users].join(", ") || "(none caught)"}`);
if (!users.size) failures.push("no render process was caught");
else if ([...users].some((user) => user !== "nobody")) failures.push(`render processes belong to ${[...users].join(", ")}, not nobody`);
const readable = Object.values(environ).filter((value) => !value.startsWith("unreadable"));
if (readable.some((value) => /WORKFLOW_SESSION_TOKEN|BUCKYOS_/.test(value))) failures.push("the render environment carries service variables");
console.log(readable.length ? `environment variable names: ${readable[0]}` : "environment: not readable for this user; only the owner of the processes was checked");

await Deno.writeTextFile(`${reportDir}/result.json`, JSON.stringify({ tag, run_id: created.run_id, failures, users: [...users], environ_readable: readable.length > 0 }, null, 2));
console.log(failures.length ? `FAILED: ${failures.join("; ")}` : "PASSED");
Deno.exit(failures.length ? 1 : 0);
