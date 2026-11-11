// Runs one Goal check (libopendan `goal_check.rs`): `deno run <this> <script>`.
// stdin: one line `{ctx, state}`, then one line per answer to a host call.
// stdout: protocol lines start with MARK; anything else the script prints is
// its log. The script's default export is `check(ctx, state)` and returns
// `{run, reason?, state?, notify?}` (a bare boolean is `{run}`).

const MARK = "\u0001opendan-check ";
const encoder = new TextEncoder();

async function send(line: unknown): Promise<void> {
  await Deno.stdout.write(encoder.encode(MARK + JSON.stringify(line) + "\n"));
}

const lines = (async function* () {
  const decoder = new TextDecoder();
  let buffer = "";
  for await (const chunk of Deno.stdin.readable) {
    buffer += decoder.decode(chunk, { stream: true });
    let i: number;
    while ((i = buffer.indexOf("\n")) >= 0) {
      yield buffer.slice(0, i);
      buffer = buffer.slice(i + 1);
    }
  }
  if (buffer.trim()) yield buffer;
})();

async function nextLine(): Promise<string> {
  const r = await lines.next();
  if (r.done) throw new Error("the host closed the check's input");
  return r.value;
}

let calls = 0;
const host = {
  /** A read-only call answered by the host (`memory.status {full?}`). */
  async call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const id = ++calls;
    await send({ kind: "call", id, method, params });
    const answer = JSON.parse(await nextLine());
    if (answer.id !== id) throw new Error(`host answered call ${answer.id}, expected ${id}`);
    if (answer.error) throw new Error(`${method}: ${answer.error}`);
    return answer.value;
  },
};

function fileUrl(path: string): string {
  if (/^[a-z]+:\/\//i.test(path)) return path;
  const p = path.replaceAll("\\", "/");
  return p.startsWith("/") ? `file://${p}` : `file:///${p}`;
}

try {
  const script = Deno.args[0];
  if (!script) throw new Error("no check script given");
  const input = JSON.parse(await nextLine());
  const mod = await import(fileUrl(script));
  const check = mod.default ?? mod.check;
  if (typeof check !== "function") {
    throw new Error("the check script exports no function (export default async function check(ctx, state) {...})");
  }
  const result = await check({ ...input.ctx, host }, input.state ?? null);
  const value = typeof result === "boolean" ? { run: result } : result;
  const convention = "a check returns {run: boolean, reason?: string, state?: any, notify?: {title, message?, url?, level?}}";
  if (!value || typeof value !== "object" || typeof value.run !== "boolean") {
    throw new Error(`${convention}, got ${JSON.stringify(result)}`);
  }
  if (value.reason != null && typeof value.reason !== "string") {
    throw new Error("`reason` of a check result is a string");
  }
  const n = value.notify;
  if (n != null) {
    const text = (v: unknown) => v == null || typeof v === "string";
    if (typeof n !== "object" || typeof n.title !== "string" || !text(n.message) || !text(n.url) || !text(n.level)) {
      throw new Error(`\`notify\` of a check result is {title: string, message?: string, url?: string, level?: "quiet" | "normal" | "timely" | "urgent"}, got ${JSON.stringify(n)}`);
    }
  }
  const notify = n == null ? null : { title: n.title, message: n.message ?? null, url: n.url ?? null, level: n.level ?? null };
  await send({ kind: "result", value: { run: value.run, reason: value.reason ?? null, state: value.state ?? null, notify } });
  Deno.exit(0);
} catch (e) {
  const message = e instanceof Error ? (e.stack ?? e.message) : String(e);
  await send({ kind: "error", message });
  Deno.exit(1);
}
