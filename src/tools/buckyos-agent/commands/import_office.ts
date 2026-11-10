// import_office — a .docx / .pptx file → an AI Workspace page you can edit
// Doc: notepads/done/aiworkspace-office-import-todo.md §7; the converter is the one the Desktop's import dialog runs
// (src/frame/aiworkspace/import-office, `@office/` in deno.json): a Word document becomes a flow page of rich text
// and picture Blocks, a PowerPoint deck a free canvas with one page per slide and a presentation path.
// Steps: 1) argv  2) read and convert the file  3) --dry-run: print the plan  4) initRuntime(), upload the pictures
//        (and the original) as Workspace assets, commit through the AiWorkspace client  5) emit one AgentToolResult.

import { commitOffice, describeOffice, parseOfficeArgs, planOfficeFile } from "@office/tools/import.ts";
import { initRuntime } from "../lib/runtime.ts";
import { bucketClient, uploadNamed } from "../lib/bucket.ts";
import { bailArgError } from "../lib/cli.ts";
import { bailRuntimeError, emitAndExit, errorResult, EXIT_SUCCESS, successResult } from "../lib/result.ts";
import { EXIT_IO_FAILED, JsonValue } from "../lib/types.ts";

const TOOL = "import_office";

export const HELP = `Usage: import_office <file> --workspace <ws_…> [options]

Import a Word or PowerPoint file into an AI Workspace as editable content, with the same converter as the
Desktop's import dialog: a .docx becomes a new flow page (rich text Blocks — headings, lists, tables, links —
and picture Blocks, the original file last); a .pptx becomes a new free canvas with one page per slide (text,
pictures and shapes as objects) and a presentation path. Run with --dry-run first and read the plan and its
report. Charts, SmartArt without its saved drawing, EMF / WMF pictures and formulas need LibreOffice to become
pictures: here they stay boxes (pptx) or are left out (docx) and are reported — for those, suggest importing on
the Desktop, where the content server's LibreOffice draws them (or "Import as it looks").

The import is one Commit: written whole or not at all, undone in one step. One Commit holds at most
32 MiB / 50 000 operations; a larger file is refused (LIMIT_EXCEEDED) — split it and import the parts.

Files: .docx .docm .pptx .pptm (save .doc / .ppt / .odt / .odp as one of these first)

Options:
  --workspace <ws_…>        the workspace (required unless --dry-run)
  --title <text>            the new page's title (default: the document's title, else the file name)
  --include-hidden          pptx: hidden slides too
  --keep-deleted            docx: keep deleted text of tracked changes, struck through (default: changes accepted)
  --no-original             do not keep the original file
  --no-path                 pptx: no presentation path
  --dry-run                 print the plan, write nothing
`;

export async function run(argv: string[]): Promise<never> {
  // 1) argv
  if (argv.includes("--help") || argv.includes("-h")) { console.error(HELP); Deno.exit(EXIT_SUCCESS); }
  let args;
  try { args = parseOfficeArgs(argv); } catch (err) { bailArgError(TOOL, err as Error); }
  if (!args.file) bailArgError(TOOL, "a file is required");
  const flag = (name: string) => args.flags.get(name) === true;
  const workspace = typeof args.flags.get("workspace") === "string" ? args.flags.get("workspace") as string : undefined;
  const title = typeof args.flags.get("title") === "string" ? args.flags.get("title") as string : undefined;
  const dryRun = flag("dry-run");
  if (!dryRun && !workspace) bailArgError(TOOL, "--workspace is required (or --dry-run)");

  // 2) read and convert
  let plan;
  try {
    plan = await planOfficeFile(args.file, { includeHidden: flag("include-hidden"), keepDeleted: flag("keep-deleted"), title });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    emitAndExit(errorResult(TOOL, `${TOOL} => unreadable`, msg, { error: msg, file: args.file }), EXIT_IO_FAILED);
  }
  const described = describeOffice(plan) as unknown as JsonValue;
  const size = plan.kind === "docx" ? `${plan.doc.segments.length} Blocks` : `${plan.pages.length} pages`;

  // 3) the plan only
  if (dryRun) {
    emitAndExit(successResult(TOOL, `${TOOL} => plan`, `${plan.kind}: ${size}, ${plan.report.notes.size} kinds of loss reported (dry run, nothing written)`, { file: args.file, plan: described }), EXIT_SUCCESS);
  }

  // 4) upload and commit
  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }
  const client = runtime.buckyos.getAiWorkspaceClient();
  let outcome;
  try {
    outcome = await commitOffice({
      call: (m, p) => client.call(m, p),
      commit: (ws, r) => client.commit(ws, r),
      // a picture: into the caller's own `assets` bucket of the service (as the Desktop uploads), else into this
      // Agent's bucket by object id; then a staged asset of the workspace
      upload: async (bytes, name, mediaType) => {
        const bucketId = `user/aiworkspace/${runtime.userId}/assets`;
        const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource)), (b) => b.toString(16).padStart(2, "0")).join("");
        const key = `${workspace}/office-${digest.slice(0, 16)}-${name.replace(/[/\\]/g, "_")}`;
        try {
          await bucketClient().putBytes(bucketId, key, bytes, { contentType: mediaType });
          const staged = await client.call("asset.import_object", { workspace_id: workspace!, bucket_id: bucketId, key }) as { object_id: string };
          return { object_id: staged.object_id };
        } catch {
          const stored = await uploadNamed(runtime, bytes, name, { mime: mediaType });
          const staged = await client.call("asset.import_object", { workspace_id: workspace!, obj_id: stored.obj_id }) as { object_id: string };
          return { object_id: staged.object_id };
        }
      },
    }, workspace!, plan, { keepOriginal: !flag("no-original"), createPath: !flag("no-path") }, (text) => console.error(text));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    emitAndExit(errorResult(TOOL, `${TOOL} => failed`, msg, { error: msg }), EXIT_IO_FAILED);
  }
  const report = plan.report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples, message: n.message })) as unknown as JsonValue;
  if (outcome.error) {
    emitAndExit(errorResult(TOOL, `${TOOL} => refused`, `${outcome.error.code} ${outcome.error.detail} (nothing was written)`, { workspace_id: workspace!, committed: outcome.committed as unknown as JsonValue, error: outcome.error as unknown as JsonValue }), EXIT_IO_FAILED);
  }

  // 5) emit
  const c = outcome.committed!;
  emitAndExit(successResult(TOOL, `${TOOL} => done`, `${plan.title}: ${size} on ${c.surface_id}${c.path_id ? `, path ${c.path_id}` : ""}`, { workspace_id: workspace!, ...(c as unknown as Record<string, JsonValue>), report }), EXIT_SUCCESS);
}

if (import.meta.main) {
  await run(Deno.args);
}
