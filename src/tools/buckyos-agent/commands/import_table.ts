// import_table — a CSV / TSV / xlsx / SQLite file → AI Workspace TableSources
// Doc: notepads/aiworkspace-table-import-todo.md §8; the importer is the one the Desktop's wizard runs
// (src/frame/aiworkspace/import-table, `@table/` in deno.json): read the file, infer header and field types
// (numbers, decimals, percentages, dates, money with its currency, single-select…), shape every value, commit.
// Steps: 1) argv  2) read the file into grids  3) plan (the wizard's defaults)  4) --dry-run: print the plan
//        5) initRuntime() and commit through the AiWorkspace client  6) emit one AgentToolResult.

import { commitPlans, describePlans, parseImportArgs, planGrids, readTabularFile } from "@table/tools/import.ts";
import { initRuntime } from "../lib/runtime.ts";
import { bailArgError } from "../lib/cli.ts";
import { bailRuntimeError, emitAndExit, errorResult, EXIT_SUCCESS, successResult } from "../lib/result.ts";
import { EXIT_IO_FAILED, JsonValue } from "../lib/types.ts";

const TOOL = "import_table";

export const HELP = `Usage: import_table <file> --workspace <ws_…> [options]

Import a table file into an AI Workspace as TableSources (fields and records), with the same
inference as the Desktop's import wizard. Run with --dry-run first and read the plan: field names
and types, row count, sample records and what changed on the way (empty rows, formulas, BLOB
columns…). Do not infer types yourself or write table.insert_records by hand.

Files: .csv .tsv .txt (tab-separated) .xlsx .xlsm .sqlite .sqlite3 .db
  (.xls, .numbers and .ods are refused: save as .xlsx or CSV)

Options:
  --workspace <ws_…>        the workspace (required unless --dry-run)
  --parent <folder_id>      data folder for the tables (default: data, the data root)
  --title <text>            table title (several tables: numbered)
  --header auto|yes|no      first row is the header (default: auto)
  --sheet <name>            xlsx: this sheet (default: the first visible one)
  --all-sheets              xlsx: one table per visible sheet
  --table <name>            SQLite: this table (repeatable; default: every table)
  --views                   SQLite: views too
  --encoding <enc>          CSV: utf-8 | gb18030 | utf-16le | utf-16be | windows-1252 (default: sniffed)
  --delimiter <d>           CSV: , | ; | tab | | (default: sniffed)
  --date-order md|dm        dates like 3/4/2026 when the values do not tell (default: md)
  --timezone <iana>         times without an offset (default: UTC)
  --locale zh|en            reading of ¥ and bare amounts (zh: CNY; en: JPY / USD)
  --dry-run                 print the plan, write nothing
`;

export async function run(argv: string[]): Promise<never> {
  // 1) argv
  if (argv.includes("--help") || argv.includes("-h")) { console.error(HELP); Deno.exit(EXIT_SUCCESS); }
  let args;
  try { args = parseImportArgs(argv); } catch (err) { bailArgError(TOOL, err as Error); }
  if (!args.file) bailArgError(TOOL, "a file is required");
  const workspace = typeof args.flags.get("workspace") === "string" ? args.flags.get("workspace") as string : undefined;
  const parent = typeof args.flags.get("parent") === "string" ? args.flags.get("parent") as string : "data";
  const dryRun = args.flags.get("dry-run") === true;
  if (!dryRun && !workspace) bailArgError(TOOL, "--workspace is required (or --dry-run)");

  // 2) read  3) plan
  let plans;
  try {
    const { grids } = await readTabularFile(args.file, args.read);
    plans = planGrids(grids, args.plan);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    emitAndExit(errorResult(TOOL, `${TOOL} => unreadable`, msg, { error: msg, file: args.file }), EXIT_IO_FAILED);
  }
  const described = describePlans(plans) as unknown as JsonValue;

  // 4) the plan only
  if (dryRun) {
    emitAndExit(successResult(TOOL, `${TOOL} => plan`, `${plans.length} table(s), ${plans.reduce((n, p) => n + p.plan.records.length, 0)} rows (dry run, nothing written)`, { file: args.file, tables: described }), EXIT_SUCCESS);
  }

  // 5) commit
  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }
  const client = runtime.buckyos.getAiWorkspaceClient();
  let outcome;
  try {
    outcome = await commitPlans({ call: (m, p) => client.call(m, p), commit: (ws, r) => client.commit(ws, r) }, workspace!, plans, parent, (text) => console.error(text));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    emitAndExit(errorResult(TOOL, `${TOOL} => failed`, msg, { error: msg }), EXIT_IO_FAILED);
  }
  if (outcome.error) {
    emitAndExit(errorResult(TOOL, `${TOOL} => refused`, `${outcome.error.table}: ${outcome.error.code} ${outcome.error.detail}`, { workspace_id: workspace!, imported: outcome.imported as unknown as JsonValue, error: outcome.error as unknown as JsonValue }), EXIT_IO_FAILED);
  }

  // 6) emit
  emitAndExit(successResult(TOOL, `${TOOL} => done`, outcome.imported.map((t) => `${t.name}: ${t.rows} rows (${t.entity_id})`).join("; "), { workspace_id: workspace!, parent_id: parent, imported: outcome.imported as unknown as JsonValue, tables: described }), EXIT_SUCCESS);
}

if (import.meta.main) {
  await run(Deno.args);
}
