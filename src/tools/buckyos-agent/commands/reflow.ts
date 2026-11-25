// reflow — fallback pages in the reflow library of the user this Agent works for.
// 配方骨架同 gen_image.ts；(2)(4)(5) 换成 lib/reflow.ts：库是用户的桶
// `user/control-panel/<user>/reflow`，规则见
// doc/opendan/protocol/Agent Browser Protocol.md §5。长期阅读的网站不在这里：
// 那是个人站的来源，命令是 `site`。

import { bailArgError, consumeCommon, parseArgvOrExit, requireString } from "../lib/cli.ts";
import {
  errorCode,
  libraryBucketId,
  libraryState,
  listSites,
  putFallback,
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
  successResult,
} from "../lib/result.ts";
import type { AgentToolResult, JsonValue } from "../lib/types.ts";

const TOOL = "reflow";

export const HELP = `Usage: reflow <subcommand> [args]

One page of a website, rearranged for the user this Agent works for: a fallback
page. It is kept in the user's reflow library and shown on https://reflow.<zone>/
whether or not the browser is running.

  reflow match <url>                    the site and the page of <url>, the fallback page kept
                                        for it, and the page to show
  reflow ls                             the sites that have fallback pages
  reflow rm <site>                      remove everything kept for <site>
  reflow result put <url> <page.html>   keep <page.html> as the fallback page of <url>

<site> is the host name with dots as dashes (www.example.com -> www-example-com;
a name without a dash gets -host: localhost -> localhost-host). \`reflow match\`
prints it. <url> is an http(s) address; its fragment does not count.

A fallback page is one .html file that stands by itself: it is shown as it is, at
<site>/fallback/<page>.html of https://reflow.<zone>/, and replaces the one kept
for the same address before.

A website the user keeps reading is not done page by page: it is a source of the
user's reading site, fetched by a program and kept up to date by a Goal. See
\`site --help\`.

Options:
  --made-by <did>       result put: who made it (default $OPENDAN_AGENT_DID)

An Agent in read-only mode cannot write the library: rm and result put fail with
permission_denied. Keep the page in the working directory and report it.

Exit codes: 0 done, 1 bad arguments, 2 the service refused or failed, 5 local file error.`;

function usage(message: string): never {
  bailArgError(TOOL, `${message} (reflow --help lists the subcommands)`);
}

function when(ms: number): string {
  return new Date(ms).toISOString();
}

function describeState(state: LibraryState): string {
  return [
    `${state.url}  site ${state.site}  page ${state.page}`,
    state.fallback
      ? `fallback: made ${when(state.fallback.made_at)} by ${state.fallback.made_by}`
      : "fallback: none",
    state.view ? `view: ${state.view} (on https://reflow.<zone>/)` : "view: nothing to show yet",
  ].join("\n");
}

const LOCAL_ERRORS = ["NotFound", "PermissionDenied", "AlreadyExists", "IsADirectory", "NotADirectory", "NotCapable"];

/** What a failure of a library operation is reported as, and the exit code that goes with it. */
export function failureResult(err: unknown, context: { write?: boolean } = {}): { result: AgentToolResult; exitCode: number } {
  const message = err instanceof Error ? err.message : String(err);
  if (err instanceof ReflowError) {
    return { result: errorResult(TOOL, `${TOOL} => invalid`, message, { error: message }), exitCode: EXIT_ARG_ERROR };
  }
  // A Deno file error has a `code` of its own (ENOENT); the name is what tells it from a service's.
  if (err instanceof Error && LOCAL_ERRORS.includes(err.name)) {
    return { result: errorResult(TOOL, `${TOOL} => io_failed`, message, { error: message }), exitCode: EXIT_IO_FAILED };
  }
  const code = errorCode(err);
  let title = "failed";
  let summary = message;
  if (code === "permission_denied" && context.write) {
    title = code;
    summary =
      "this Agent may not write the user's reflow library (its permission mode is read-only). Keep what you made in the working directory and report it.";
  } else if (code === "bucket_not_found") {
    title = "no_library";
    summary = "this Zone has no reflow library: its control-panel service does not declare the reflow bucket";
  }
  return {
    result: errorResult(TOOL, `${TOOL} => ${title}`, summary, { error: message, ...(code ? { error_code: code } : {}) }),
    exitCode: EXIT_AICC_FAILED,
  };
}

function done(title: string, summary: string, detail: unknown): never {
  emitAndExit(
    successResult(TOOL, `${TOOL} ${title} => done`, summary, detail as JsonValue),
    EXIT_SUCCESS,
  );
}

// One library operation: (3) the BuckyOS runtime — the library is the reflow
// bucket control-panel keeps for the owner — (4/5) the operation, (6) one
// AgentToolResult.
async function inLibrary(
  title: string,
  context: { write?: boolean },
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
    const failure = failureResult(err, context);
    emitAndExit(failure.result, failure.exitCode);
  }
  done(title, out.summary, out.detail);
}

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
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
  const unknown = [...consumeCommon(parsed.flags).keys()].find((name) => name !== "made-by");
  if (unknown) usage(`unsupported option: --${unknown}`);

  // ── 2–6. 每个子命令一次库操作 ──────────────────────────────────────────
  switch (sub) {
    case "match":
      expect(1, "match <url>");
      return await inLibrary("match", {}, async (store, bucketId) => {
        const state = await libraryState(store, bucketId, await reflowAddress(args[0]));
        return { summary: describeState(state), detail: state };
      });
    case "ls":
      expect(0, "ls");
      return await inLibrary("ls", {}, async (store, bucketId) => {
        const sites = await listSites(store, bucketId);
        return {
          summary: sites.map((entry) => `${entry.site}  ${entry.pages} fallback page(s)`).join("\n") ||
            "the library has no fallback pages",
          detail: { bucket_id: bucketId, sites },
        };
      });
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
        usage("expected: reflow result put <url> <page.html>");
      }
      const [, url, file] = args;
      const by = madeBy ?? Deno.env.get("OPENDAN_AGENT_DID");
      if (!by) usage("result put: $OPENDAN_AGENT_DID is not set here; pass --made-by <did>");
      return await inLibrary("result put", { write: true }, async (store, bucketId) => {
        const report = await putFallback(store, bucketId, await reflowAddress(url), file, by);
        return {
          summary: `kept the fallback page as ${report.key}\n${describeState(report.state)}`,
          detail: report,
        };
      });
    }
    default:
      usage(sub ? `unknown subcommand: ${sub}` : "a subcommand is required");
  }
}

if (import.meta.main) {
  await run(Deno.args);
}
