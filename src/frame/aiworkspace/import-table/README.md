# import-table — table data into a TableSource

Implements [表格数据导入 TODO](../../../../notepads/aiworkspace-table-import-todo.md) §2–§6. A dependency-free
TypeScript module like `../import-drawio` and `../import-pdf` (whose `ImportReport`, `keysAfter` and XML reader it
reuses): the Desktop (Vite alias `@table/*`) and Deno (the Agent command `import_table`, tests) run the same code.
SQLite is reached through a query function the host hands in (sqlite-wasm in a browser Worker, `node:sqlite` in
Deno). Files import each other with explicit `.ts` extensions.

```text
grid.ts        Grid / Cell (display text, raw value, hint, currency, exact wire value); tidy (empty rows and columns)
csv.ts         encoding (BOM → strict UTF-8 → GB 18030; UTF-16 without BOM), delimiter (, ; tab |), RFC 4180
text.ts        pasted TSV (is it a table?), split on runs of spaces on request
html.ts        clipboard HTML tables: spans laid out, Excel x:num / mso-number-format, Google data-sheets-*, aiws marks
xlsx.ts        zip (stored / deflate) + workbook, shared strings, styles, sheets, merged cells, 1900 / 1904 dates
sqlite.ts      tables (views on request), declared types by SQLite affinity, BLOB columns left out
format.ts      number formats → date / datetime / time / money / percent; Excel serial numbers
currency.ts    ISO codes, names, symbols; `$` and `¥` per column (Q12)
infer.ts       column types (boolean → number → decimal → date → datetime → currency → select → text), shaping
plan.ts        header detection, field names, the wizard's choices, the plan (fields, records, report)
operations.ts  plan → commits (entity.create + table.insert_records ≤ 500; ≤ 20 000 rows and 6 MiB per commit)
files.ts       which file is a table (extension, magic numbers); the picker's accept list
mod.ts         the public entry
tools/import.ts       the Deno host: a file → grids → plans → commits through an AiWorkspace client; dry run alone
tools/sqlite_deno.ts  the SQLite query function on node:sqlite
```

Inference is lossless: a column gets a type only when every non-empty value fits it, else it is text; values that
do not fit only appear when the user picks another type in the wizard (left empty, or the whole column as text) and
are reported. The core's own checks run in the browser through WASM `normalize_value`; the service checks again.

## Test

```bash
deno test -A                                   # table_test.ts: golden plans of every fixture, the rules one by one
TABLE_REGEN=1 deno test -A                     # rewrite ../fixtures/table/expected/*.plan.json after an intended change
deno run -A tools/import.ts ../fixtures/table/xlsx/workbook.xlsx --all-sheets --locale zh   # print a file's plan
cd ../fixtures/table/src && python3 make_xlsx.py && python3 make_sqlite.py                  # rebuild the binary fixtures
```

The clipboard fixtures (`../fixtures/table/clipboard/`) are written to the formats Excel, Google Sheets, Numbers,
Notion and web pages put on the clipboard; they were not captured from those applications. The Desktop e2e suite
(`tests/aiworkspace/table-import.spec.ts`, TI-01 – TI-12) drives the importer through the real app.
