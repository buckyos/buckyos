// reflow — the reflow library of the user this Agent works for, and refreshing
// a page in this Agent's browser.
// 配方骨架同 gen_image.ts；(2)(4)(5) 换成 lib/reflow.ts：库是用户的桶
// `user/control-panel/<user>/reflow`，规则见
// doc/opendan/protocol/Agent Browser Protocol.md §5。`refresh` 不读写桶，
// 它调本 Agent 的 Agent Service（$OPENDAN_BROWSER_RPC）。

import http from "node:http";
import https from "node:https";

import { bailArgError, consumeCommon, parseArgvOrExit, requireString } from "../lib/cli.ts";
import {
  errorCode,
  getPackage,
  libraryBucketId,
  libraryState,
  listSites,
  PREFERENCE_FILE,
  putPackage,
  putResult,
  readPreference,
  reflowAddress,
  ReflowError,
  removeSite,
} from "../lib/reflow.ts";
import type { LibraryState, LibraryStore } from "../lib/reflow.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_AICC_FAILED,
  EXIT_ARG_ERROR,
  EXIT_IO_FAILED,
  EXIT_SUCCESS,
  EXIT_TIMEOUT,
  successResult,
} from "../lib/result.ts";
import type { JsonValue } from "../lib/types.ts";

const TOOL = "reflow";
const REFRESH_POLL_MS = 2000;
const DEFAULT_REFRESH_WAIT_MS = 120_000;

export const HELP = `Usage: reflow <subcommand> [args]

The reflow library of the user this Agent works for: per site, a package
(manifest.json, refresh.mjs, index.html and the files it uses), the results
made with it, fallback pages and, while the site has no package, the user's
preference for it (preference.json, written by the app). What the library has
for a page is shown on https://reflow.<zone>/ whether or not the browser is
running.

  reflow match <url>               the package that applies to <url>, the user's preference for
                                   the site, what was made for the page, and the page to show
  reflow ls                        the sites in the library, each with the user's preference
  reflow get <site> <dir>          copy the package of <site> into <dir> (new or empty)
  reflow put <site> <dir>          check the package in <dir> and make it the package of <site>
  reflow rm <site>                 remove everything of <site>: package, preference, results,
                                   fallback pages
  reflow result put <url> <file>   keep what was made for <url>: a .json file is the data the
                                   site's package draws, a .html file is a fallback page
  reflow refresh <url>             run the package's refresh.mjs on <url> in this Agent's browser

<site> is the host name with dots as dashes (www.example.com -> www-example-com;
a name without a dash gets -host: localhost -> localhost-host). \`reflow match\`
prints it.

A package (schema buckyos.reflow/1):
  manifest.json  { "schema": "buckyos.reflow/1", "site": "<site>",
                   "match": [ { "host": "www.example.com", "path": "/news/*" } ],
                   "preference": "<what the user wants to see>",
                   "author": { "kind": "agent" | "user", "id": "<DID or user id>" },
                   "refresh": { "script": "refresh.mjs", "timeout_s": 60 } }
                 match[].host is the site's host name in lower case; in path, * matches
                 anything (no path: every page of the host). put writes updated_at, and
                 author when the manifest has none.
  refresh.mjs    export default async ({ page, url, save }) => data
                 runs on the original page (a Playwright page); data must be JSON.
  index.html     draws results/<page>.json (page is in its ?page=) as text, never as HTML.
A package has refresh.mjs, index.html or both. put replaces the site's package: files the new
one does not have are removed, results and fallback pages stay. Hidden files are not uploaded;
results/, fallback/ and preference.json belong to the library and cannot be in a package
(get leaves them out).

The preference is the user's. When you write a package, write it into manifest.json: it is in
the request that asked for the page, and \`reflow match\` prints what the library has for the
site (its package's, or preference.json of a site without one). A manifest that states none
gets that one from put. Either way preference.json is removed once the package is in: the
manifest takes its place.

Options:
  --timeout <seconds>   refresh: how long to wait for the result (default 120)
  --no-wait             refresh: start it and return
  --made-by <did>       result put: who made it (default $OPENDAN_AGENT_DID)

An Agent in read-only mode cannot write the library: put, rm and result put fail with
permission_denied. Keep the package in the working directory, report it, and ask the user
to import it in the Agent Browser app.

Exit codes: 0 done, 1 bad arguments or an invalid package, 2 the service refused or failed,
4 refresh still running after --timeout, 5 local file error.`;

function usage(message: string): never {
  bailArgError(TOOL, `${message} (reflow --help lists the subcommands)`);
}

function when(ms: number): string {
  return new Date(ms).toISOString();
}

// `preference`: the user's preference for the site, when the caller looked it up.
function describeState(state: LibraryState, preference?: string): string {
  const made = (m: LibraryState["result"]) => m ? `made ${when(m.made_at)} by ${m.made_by}` : "none";
  return [
    `${state.url}  site ${state.site}  page ${state.page}`,
    state.package
      ? `package: updated ${when(state.package.updated_at)} by ${state.package.author.id}`
      : "package: none matches",
    ...(preference === undefined ? [] : [`preference: ${preference || "none"}`]),
    `result: ${made(state.result)}`,
    `fallback: ${made(state.fallback)}`,
    state.view ? `view: ${state.view} (on https://reflow.<zone>/)` : "view: nothing to show yet",
  ].join("\n");
}

const LOCAL_ERRORS = ["NotFound", "PermissionDenied", "AlreadyExists", "IsADirectory", "NotADirectory"];

// Every failure of a library operation ends here.
function bail(err: unknown, context: { write?: boolean; dir?: string } = {}): never {
  const message = err instanceof Error ? err.message : String(err);
  if (err instanceof ReflowError) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => invalid`, message, { error: message, problems: err.problems }),
      EXIT_ARG_ERROR,
    );
  }
  const code = errorCode(err);
  if (code === "permission_denied" && context.write) {
    const summary =
      "this Agent may not write the user's reflow library (its permission mode is read-only)." +
      (context.dir ? ` Keep the package in ${context.dir},` : " Keep what you made in the working directory,") +
      " report it, and ask the user to import it in the Agent Browser app.";
    emitAndExit(
      errorResult(TOOL, `${TOOL} => permission_denied`, summary, {
        error: message,
        error_code: code,
        ...(context.dir ? { package_dir: context.dir } : {}),
      }),
      EXIT_AICC_FAILED,
    );
  }
  if (code === "bucket_not_found") {
    const summary =
      "this Zone has no reflow library: its control-panel service does not declare the reflow bucket";
    emitAndExit(
      errorResult(TOOL, `${TOOL} => no_library`, summary, { error: message, error_code: code }),
      EXIT_AICC_FAILED,
    );
  }
  if (!code && err instanceof Error && LOCAL_ERRORS.includes(err.name)) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => io_failed`, message, { error: message }),
      EXIT_IO_FAILED,
    );
  }
  emitAndExit(
    errorResult(TOOL, `${TOOL} => failed`, message, {
      error: message,
      ...(code ? { error_code: code } : {}),
    }),
    EXIT_AICC_FAILED,
  );
}

function done(title: string, summary: string, detail: unknown): never {
  emitAndExit(
    successResult(TOOL, `${TOOL} ${title} => done`, summary, detail as JsonValue),
    EXIT_SUCCESS,
  );
}

// The Agent Service's errors travel as JSON inside the kRPC error text: the
// browser module's carry a stable `code`, the others a `kind` (Agent Browser
// Protocol §2.3, §2.5).
function serviceError(err: unknown): { code?: string; message: string } {
  const text = err instanceof Error ? err.message : String(err);
  const start = text.indexOf("{");
  if (start >= 0) {
    try {
      const wire = JSON.parse(text.slice(start));
      if (wire && typeof wire === "object") {
        return {
          code: [wire.code, wire.kind, errorCode(err)].find((code) => typeof code === "string"),
          message: typeof wire.message === "string" ? wire.message : text,
        };
      }
    } catch {
      // not JSON: the text is the message
    }
  }
  return { code: errorCode(err), message: text };
}

interface BrowserReflowState extends LibraryState {
  refresh: { status: "idle" | "running" | "failed"; error: string | null; at_ms: number | null };
}

// One kRPC call to this Agent's Agent Service. Not `fetch` (the SDK's client uses it): it refuses the ports of its
// "bad port" list, and an Agent Service can be given one (10080).
function callAgentService<T>(endpoint: string, token: string, method: string, params: unknown): Promise<T> {
  return new Promise((resolve, reject) => {
    const target = new URL(endpoint);
    const text = JSON.stringify({ method, params, sys: [Date.now(), token] });
    const request = (target.protocol === "https:" ? https : http).request(
      target,
      {
        method: "POST",
        headers: { "content-type": "application/json", "content-length": new TextEncoder().encode(text).length },
      },
      (response) => {
        let answer = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => answer += chunk);
        response.on("error", reject);
        response.on("end", () => {
          if ((response.statusCode ?? 0) < 200 || (response.statusCode ?? 0) >= 300) {
            reject(new Error(`the Agent Service answered HTTP ${response.statusCode}`));
            return;
          }
          let body: { result?: T; error?: unknown };
          try {
            body = JSON.parse(answer);
          } catch {
            reject(new Error("the Agent Service sent an unreadable answer"));
            return;
          }
          if (body.error !== undefined && body.error !== null) reject(new Error(String(body.error)));
          else resolve(body.result as T);
        });
      },
    );
    request.setTimeout(60_000, () => request.destroy(new Error("the Agent Service did not answer in 60s")));
    request.on("error", (err: Error) => reject(new Error(`cannot reach the Agent Service at ${endpoint}: ${err.message}`)));
    request.end(text);
  });
}

async function refresh(url: string, wait: boolean, waitMs: number): Promise<never> {
  const endpoint = Deno.env.get("OPENDAN_BROWSER_RPC");
  const token = Deno.env.get("BUCKYOS_APPCLIENT_SESSION_TOKEN");
  if (!endpoint || !token) {
    const msg =
      "reflow refresh runs in a session on the Agent's own runtime: $OPENDAN_BROWSER_RPC and $BUCKYOS_APPCLIENT_SESSION_TOKEN are not both set here, so the Agent's browser is not reachable";
    emitAndExit(
      errorResult(TOOL, `${TOOL} refresh => no_browser`, msg, { error: msg }),
      EXIT_AICC_FAILED,
    );
  }
  const call = (method: string) => callAgentService<BrowserReflowState>(endpoint, token, method, { url });
  let state: BrowserReflowState;
  try {
    state = await call("browser.reflow_refresh");
    const deadline = Date.now() + waitMs;
    while (wait && state.refresh.status === "running" && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, REFRESH_POLL_MS));
      state = await call("browser.reflow_state");
    }
  } catch (err) {
    const { code, message } = serviceError(err);
    const summary = code === "reflow_no_package"
      ? `no package in the library matches ${url}: write one and \`reflow put <site> <dir>\`, or keep a fallback page with \`reflow result put <url> <page.html>\``
      : message;
    emitAndExit(
      errorResult(TOOL, `${TOOL} refresh => ${code ?? "failed"}`, summary, {
        error: message,
        ...(code ? { error_code: code } : {}),
      }),
      EXIT_AICC_FAILED,
    );
  }
  const detail = state as unknown as JsonValue;
  if (state.refresh.status === "failed") {
    emitAndExit(
      errorResult(
        TOOL,
        `${TOOL} refresh => refresh_failed`,
        `refresh.mjs failed on ${state.url}: ${state.refresh.error ?? "no reason given"}`,
        detail,
      ),
      EXIT_AICC_FAILED,
    );
  }
  if (state.refresh.status === "running") {
    if (!wait) done("refresh", `refreshing ${state.url}; \`reflow match\` shows the result when it is made`, detail);
    emitAndExit(
      errorResult(
        TOOL,
        `${TOOL} refresh => wait_timeout`,
        `${state.url} is still refreshing after ${Math.round(waitMs / 1000)}s; \`reflow match\` shows the result when it is made`,
        detail,
      ),
      EXIT_TIMEOUT,
    );
  }
  done("refresh", describeState(state), detail);
}

// One library operation: (3) the BuckyOS runtime — the library is the reflow
// bucket control-panel keeps for the owner — (4/5) the operation, (6) one
// AgentToolResult.
async function inLibrary(
  title: string,
  context: { write?: boolean; dir?: string },
  operation: (store: LibraryStore, bucketId: string) => Promise<{ summary: string; detail: unknown }>,
): Promise<never> {
  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }
  const store = runtime.buckyos.getBucketClient() as unknown as LibraryStore;
  let out;
  try {
    out = await operation(store, libraryBucketId(runtime.ownerUserId));
  } catch (err) {
    bail(err, context);
  }
  done(title, out.summary, out.detail);
}

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  const parsed = parseArgvOrExit(TOOL, HELP, argv, ["no-wait"]);
  const [sub, ...args] = parsed.positional;
  const expect = (count: number, form: string) => {
    if (args.length !== count) usage(`expected: reflow ${form}`);
  };
  let madeBy: string | undefined;
  try {
    madeBy = requireString(parsed.flags, "made-by");
  } catch (err) {
    bailArgError(TOOL, err as Error);
  }
  const unknown = [...consumeCommon(parsed.flags).keys()].find((name) =>
    name !== "made-by" && name !== "no-wait"
  );
  if (unknown) usage(`unsupported option: --${unknown}`);

  // ── 2–6. 每个子命令一次库操作；refresh 交给 Agent Service ──────────────
  switch (sub) {
    case "match":
      expect(1, "match <url>");
      return await inLibrary("match", {}, async (store, bucketId) => {
        const state = await libraryState(store, bucketId, await reflowAddress(args[0]));
        const preference = await readPreference(store, bucketId, state.site);
        return { summary: describeState(state, preference), detail: { ...state, preference } };
      });
    case "ls":
      expect(0, "ls");
      return await inLibrary("ls", {}, async (store, bucketId) => {
        const sites = await listSites(store, bucketId);
        const lines = sites.map((entry) =>
          (entry.package
            ? `${entry.site}  ${
              entry.package.match.map((rule) => `${rule.host}${rule.path ?? "/*"}`).join(" ")
            }  updated ${when(entry.package.updated_at)} by ${entry.package.author.id}`
            : `${entry.site}  no package`) +
          (entry.preference ? `  preference: ${entry.preference}` : "")
        );
        return {
          summary: lines.length ? lines.join("\n") : "the library is empty",
          detail: { bucket_id: bucketId, sites },
        };
      });
    case "get": {
      expect(2, "get <site> <dir>");
      const [site, dir] = args;
      return await inLibrary("get", {}, async (store, bucketId) => {
        const files = await getPackage(store, bucketId, site, dir);
        const path = await Deno.realPath(dir);
        return {
          summary: `wrote the ${files.length} file(s) of ${site} to ${path}`,
          detail: { site, dir: path, files },
        };
      });
    }
    case "put": {
      expect(2, "put <site> <dir>");
      const [site, dir] = args;
      return await inLibrary("put", { write: true, dir }, async (store, bucketId) => {
        const report = await putPackage(store, bucketId, site, dir, {
          agentDid: Deno.env.get("OPENDAN_AGENT_DID") || undefined,
        });
        const gone = report.removed.filter((file) => file !== PREFERENCE_FILE);
        const notes = [
          gone.length ? `removed ${gone.join(", ")}` : "",
          report.removed.includes(PREFERENCE_FILE)
            ? `the manifest's preference took the place of ${PREFERENCE_FILE}`
            : "",
          report.manifest.preference ? `preference: ${report.manifest.preference}` : "",
        ].filter(Boolean).map((note) => `; ${note}`).join("");
        return {
          summary: `${site}: the package (${report.files.length} file(s)) is in the library${notes}`,
          detail: report,
        };
      });
    }
    case "rm": {
      expect(1, "rm <site>");
      const [site] = args;
      return await inLibrary("rm", { write: true }, async (store, bucketId) => {
        const removed = await removeSite(store, bucketId, site);
        return {
          summary: removed
            ? `removed ${site} (${removed} file(s))`
            : `the library has nothing for ${site}`,
          detail: { site, removed },
        };
      });
    }
    case "result": {
      if (args[0] !== "put" || args.length !== 3) {
        usage("expected: reflow result put <url> <file>");
      }
      const [, url, file] = args;
      const by = madeBy ?? Deno.env.get("OPENDAN_AGENT_DID");
      if (!by) usage("result put: $OPENDAN_AGENT_DID is not set here; pass --made-by <did>");
      return await inLibrary("result put", { write: true }, async (store, bucketId) => {
        const report = await putResult(store, bucketId, await reflowAddress(url), file, by);
        const what = report.kind === "result" ? "result" : "fallback page";
        return {
          summary: `kept the ${what} as ${report.key}\n${describeState(report.state)}`,
          detail: report,
        };
      });
    }
    case "refresh":
      expect(1, "refresh <url>");
      return await refresh(
        args[0],
        parsed.flags.get("no-wait") !== true,
        parsed.common.timeoutMs ?? DEFAULT_REFRESH_WAIT_MS,
      );
    default:
      usage(sub ? `unknown subcommand: ${sub}` : "a subcommand is required");
  }
}

if (import.meta.main) {
  await run(Deno.args);
}
