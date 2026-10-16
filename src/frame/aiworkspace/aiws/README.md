# aiws v2 — the program host API

A wish program is an ES module, `program/main.js`, whose default export is `async function main(aiws)`. It reads this run's snapshot and writes **candidate results**; it does not modify the workspace directly. Use only Deno's built-in APIs and `aiws`; do not bring in third-party dependencies.

Debugging: `deno run -A lib/run.js` (prints a summary of the results; the full output is in `output/.aiws/results.json`). The host's `run_program` and a later "re-run only the program" run the same program with the same `lib/run.js` (with stricter permissions: it can only read the run directory and only write `output/`).

## Reading

```js
const sales = aiws.input('sales')        // get an input by input name (input names: see context/inputs.json and the output contract)
sales.kind                               // 'table' | 'richtext' | 'record' | 'asset' | 'folder' | 'annotation'
sales.fields()                           // [{ name, type, options?, id, visible }], in the table's field order
sales.rows()                             // [{ field name: value, _id }]; a view input follows the view's filter and sort
sales.rows({ fields: ['Month', 'Sales'], filter: { Status: 'Settled' }, sort: '-Sales', limit: 10 })
sales.rows({ filter: (r) => r['Sales'] > 0 })
aiws.input('rules').markdown()           // rich text → Markdown
aiws.input('profile').props()            // record → { property name: value }
aiws.input('logo').bytes()               // asset → Uint8Array (.text() for the text)
aiws.input('folder').members()           // folder members (each member also has rows/markdown/…)
aiws.inputs()                            // all input names
aiws.request                             // fixed parameters: { today, timezone, currency, … }; dates follow it
aiws.resolve('Orders') / aiws.find('Sales') / aiws.outline()   // return metadata only; to read content it must be an input
```

Value forms: a number is a number, a single select is the option name (a string), a multi-select is an array of names, a date is a `YYYY-MM-DD` string, and empty values do not appear in the row object (`r['x'] === undefined`).

## Writing results (candidates only)

```js
// table: key is the logical key (one or more columns) that keeps record identity across re-runs; fields can declare types and column order
aiws.result.table('monthly', rows, { key: ['Month'], fields: { Month: 'date', Sales: 'decimal', Orders: 'number' } })
// derived columns: written onto a table input, values by the _id of that input's rows; only columns this wish owns are written
aiws.result.columns('Diff', 'salary', Object.fromEntries(rows.map((r) => [r._id, { Diff: r['Salary'] - avg }])), { fields: { Diff: 'decimal' } })
aiws.result.record('kpi', { 'Total sales': 123.4, Orders: 12 })
aiws.result.text('commentary', markdown)            // text the program generates from a template (also updated when only the program is re-run)
aiws.result.file('chart', 'output/chart.svg', 'image/svg+xml')
aiws.result.html('dashboard', { html, css, js, bindings: { orders: 'input:sales', summary: 'result:monthly' } })
```

Field types: `text | number | decimal | boolean | date | datetime | select | multi_select`; inferred from the values when not declared. `decimal` keeps as many decimal places as the values need (at most 6; it never rounds to fewer); to fix the number of decimal places, use the array form `fields: [{ name: 'Gross margin', type: 'decimal', scale: 4 }]`, and the program then does its own rounding. A table is shown in the order of `rows` (up to 1000 rows; once the user sorts it on the Block, the user's order wins), so sort in the program when needed. A result name can be written only once; result names must be names in the output contract.

## Numbers, checks and per-item judgments

```js
aiws.facts({ total: 3301.5, growth: 0.091 })        // numbers cited in text must come from facts or result tables
aiws.check('total', Math.abs(a - b) < 0.01, { a, b }) // report a program check agreed in the analysis: id, passed or not, detail
const tags = await aiws.llm.map(texts, 'Judge the sentiment of this customer feedback', { type: 'string', enum: ['positive', 'neutral', 'negative'] })
const amounts = await aiws.llm.map(notes, 'Extract the amount (in yuan) from the note; null if there is none', { type: ['number', 'null'] })
```

`llm.map` calls the model item by item and validates each answer against the schema; results are cached by (model, instruction, input item), so re-running only the program calls the model only for new or changed items; failed items make the call throw (with `{ allowErrors: true }` they come back as `{ error }`). Use it for per-item semantic judgments; do not let the model read through the whole table.

## Example

```js
export default async function main(aiws) {
  const rows = aiws.input('sales').rows({ fields: ['Month', 'Customer', 'Sales', 'Status'] })
  const settled = rows.filter((r) => r['Status'] !== 'Unsettled')
  const sum = (xs) => Math.round(xs.reduce((s, x) => s + (x ?? 0), 0) * 100) / 100
  const byMonth = new Map()
  for (const r of settled) byMonth.set(r['Month'], [...(byMonth.get(r['Month']) ?? []), r])
  const monthly = [...byMonth].sort(([a], [b]) => a.localeCompare(b))
    .map(([Month, items]) => ({ Month, Sales: sum(items.map((r) => r['Sales'])), Orders: items.length }))
  aiws.result.table('monthly', monthly, { key: ['Month'], fields: { Month: 'date', Sales: 'decimal', Orders: 'number' } })
  const total = sum(settled.map((r) => r['Sales']))
  aiws.facts({ total, months: monthly.length })
  aiws.check('total', Math.abs(sum(monthly.map((m) => m['Sales'])) - total) < 0.01)
  aiws.check('months', ['2026-07', '2026-08', '2026-09'].every((m) => monthly.some((r) => r['Month'].startsWith(m))))
}
```
