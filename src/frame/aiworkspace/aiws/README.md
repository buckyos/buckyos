# Writing a wish macro (aiws v3)

A wish's **macro** is the program that produces its results. It is written once, in the coding stage — by the model, an Agent or a person — and every later run of the wish runs it again on the current data, with no model loop: only the AI calls the macro itself makes. This guide is the one reference for all three; the coding prompt embeds it, the hosted run manual (`aiworkspace_run`) points to it.

A macro is an ES module, `program/main.js`, whose default export is `async function main(aiws)`. It reads the run's snapshot and writes **candidate results**; it never writes the workspace. Use only JavaScript built-ins and `aiws`: no imports, no clock, no network outside the Zone.

## 1. Declare first

```js
export default async function main(aiws) {
  aiws.declare({
    inputs: { sales: ['Month', 'Customer', 'Sales', 'Status'], rules: [] },   // table inputs: the fields read; other inputs: []
    ai: { complete: { max_calls: 4, max_tokens: 4000 }, map: { max_items: 500 } },   // only what is called
    params: ['today', 'timezone'],                                             // fixed parameters the macro relies on
  })
  // …
}
```

- Call it once, before anything else. Reading an input or a field that is not declared throws (`field X is not declared`), so a trial run shows it at once instead of a later run silently reading an empty value.
- The host resolves the declaration into the **manifest**: each field by its id and the name it has now. Every later run materializes only these inputs and fields, under these names — a field or a table renamed later changes nothing; a declared field deleted or changed to another type stops the wish before it runs ("needs coding").
- `ai`: a macro that does not declare `ai.complete` / `ai.map` / `ai.image` / `ai.video` cannot call them. The limits are per run; the workspace's own limits cap them. `ai.complete.models: ['llm.code']` (likewise `ai.image.models`) allows models other than the default.
- `zone: ['/kapi/…']` lists the Zone resources the macro reads (`aiws.zone.fetch`, GET / HEAD only; nothing unlisted is reached); `resources: { timeout_secs }` a shorter time limit.

## 2. Reading inputs

```js
const sales = aiws.input('sales')        // a declared input by its input name (context/inputs.json lists them)
sales.kind                               // 'table' | 'richtext' | 'record' | 'asset' | 'folder' | 'annotation' | 'control'
sales.fields()                           // the declared fields: [{ name, type, options?, id, visible }]
sales.rows()                             // [{ field name: value, _id }] of the declared fields; a view input follows the view
sales.rows({ fields: ['Month', 'Sales'], filter: { Status: 'Settled' }, sort: '-Sales', limit: 10 })
aiws.input('rules').markdown()           // rich text → Markdown
aiws.input('profile').props()            // record → { property name: value }
aiws.input('logo').bytes()               // asset → Uint8Array (.text() for the text)
aiws.input('logo').media()               // asset → a media reference (a picture to follow, a file to write as a result)
aiws.input('folder').members()           // folder members (each member also has rows/markdown/…)
aiws.input('branch').outline()           // a control (a mind map) or one of its nodes → its tree as a Markdown outline
aiws.request                             // fixed parameters: { today, timezone, currency, … }; dates follow it
```

Value forms: a number is a number, a single select is the option name (a string), a multi-select is an array of names, a date is a `YYYY-MM-DD` string, and empty values do not appear in the row object.

## 3. Set computations: `aiws.sql`

```js
const { rows } = await aiws.sql(`
  SELECT substr("Month", 1, 7) AS month, round(sum("Sales"), 2) AS sales, count(*) AS orders
  FROM sales WHERE "Status" <> 'Unsettled' GROUP BY 1 ORDER BY 1`)
// rows: [{ month: '2026-07', sales: 3301.5, orders: 12 }, …]; also { columns, truncated }
```

- One SQLite `SELECT` over the declared table inputs. A table is named by its **input name**; its columns are the declared fields under their declared names (display values: a select's label, a reference's path, a currency's amount), `"<field>#raw"` the stored values (option ids, exact decimals), and `_id` (the record id). A view input is the rows its view shows.
- Filtering, grouping, aggregating, joining, window functions, de-duplicating: SQL. JavaScript arranges the results and does what SQL cannot (parsing free text).
- Bounds: one statement, no writes, no recursive CTEs, 2 seconds and 5000 rows per query, and a number of queries per run. To join large tables, put the smaller side in `WITH d AS MATERIALIZED (SELECT …)` and join to `d`.
- Parameters: `aiws.sql('SELECT … WHERE "Region" = ?', ['East'])`.

## 4. AI in a macro

```js
const facts = { total: 3301.5, growth: 0.091, months: 3 }
aiws.facts(facts)                                                   // the numbers text may cite
const { text } = await aiws.ai.complete({
  system: 'You write short business commentary. Use only the numbers given in the facts; do not compute or invent other numbers.',
  prompt: `Facts (JSON):\n${JSON.stringify(facts)}\n\nWrite a three-sentence commentary on the quarter.`,
  max_tokens: 800,
})
aiws.result.text('commentary', text)

const { value } = await aiws.ai.complete({ prompt: `…`, schema: { type: 'object', properties: { risk: { enum: ['low', 'high'] } }, required: ['risk'] } })
const tags = await aiws.llm.map(texts, 'Judge the sentiment of this customer feedback', { type: 'string', enum: ['positive', 'neutral', 'negative'] })
```

- `aiws.ai.complete({ system?, prompt | messages, schema?, model?, max_tokens? })` is one inference: no tools, no loop. Without `schema` it returns `{ text }`; with one, `{ value }` checked against it (type, enum, properties, required, items).
- `aiws.llm.map(items, instruction, schema, { allowErrors? })` judges each item on its own (sentiment tags, extracting an amount from a note); failed items make it throw unless `allowErrors`.
- **Numbers in text come from facts.** Give the model the facts the macro computed and ask it not to compute or invent numbers; numbers in a text result that are neither in the facts nor in a table result are flagged to the user.
- **Keep data apart from instructions.** Put input text in its own clearly marked block (JSON), never mixed into the instruction.
- **Constrain the output** with `schema` whenever the macro uses the answer as data.
- **Cache and budget**: answers are cached by content — the same request gives the same answer on the next run, and costs nothing. New calls count against the declared limits (calls, tokens, items); over them the call throws `AI_BUDGET_EXCEEDED`. A refusal of the AI service (`AI_QUOTA_EXHAUSTED`, `AI_QUOTA_BUDGET_EXCEEDED`, `AI_RESOURCE_NOT_PERMITTED`) ends the run with that reason; do not catch it.

## 4b. Generated pictures and video clips

```js
aiws.declare({ inputs: { cast: ['Name', 'Look', 'Color'] }, ai: { image: { max_calls: 6 }, video: { max_calls: 1 } } })
const cast = aiws.input('cast').rows()
const sheets = await Promise.all(cast.map((c) => aiws.ai.image({
  prompt: `Character sheet, full body, plain background, watercolor. ${c.Name}: ${c.Look}; main color ${c.Color}.`, aspect: '3:4' })))
sheets.forEach((img, i) => aiws.result.file(`sheet_${i + 1}`, img, { title: cast[i].Name }))
const clip = await aiws.ai.video({ prompt: 'The courier from @{hero} rides through neon rain at night, tracking shot.',
  duration: 6, aspect: '16:9', references: { hero: sheets[0] } })
aiws.result.file('teaser', clip, { title: 'Teaser' })
```

- `aiws.ai.image({ prompt, aspect?, size?, quality?, format?, negative?, seed?, images?, model? })` generates one picture (`aspect` `1:1 | 3:4 | 4:3 | 9:16 | 16:9`, `size` `WxH`, `quality` `low | medium | high`, `format` `png | jpeg | webp`). `images`: reference pictures to edit or to follow — earlier answers, or `aiws.input('photo').media()` (a folder input: its members' `.media()`).
- `aiws.ai.video({ prompt, duration?, aspect?, resolution?, first_frame?, last_frame?, references?, model? })` generates one clip (`duration` in seconds, `aspect` `16:9 | 9:16`, `resolution` `720p | 1080p`). `first_frame` / `last_frame` are pictures the clip starts or ends on; `references: { name: picture }` are pictures it follows — name them in the prompt as `@{name}`. Video models take few reference pictures (often one) and do not mix keyframes with references; what a model cannot honour fails with the reason. One clip is one continuous shot of seconds, not an edited film.
- Both answer a media reference `{ kind: 'image' | 'video', object_id, media_type, size, width?, height?, duration? }` (a picture's size as the Workspace read it, a clip's as the AI service measured it): the file is kept in the Workspace and the macro never holds its bytes — check sizes and durations on the reference. Write it with `aiws.result.file(name, media, { title })` — an `image` result for a picture, an `asset` result for a clip — or pass it to the next generation.
- Cached like text: the same request gives the same file without paying again, so a later run on unchanged inputs makes nothing new. Build prompts only from the inputs (never from the clock or random numbers). Start independent generations together with `Promise.all`; waiting for them does not count against the macro's time limit.
- `ai.image.max_calls` / `ai.video.max_calls` count new generations per run; the stage caps them (`AI_BUDGET_EXCEEDED`). Video is the expensive medium: one clip per wish unless the user asks for more.

## 5. Writing results (candidates only)

```js
aiws.result.table('monthly', rows, { key: ['Month'], fields: { Month: 'date', Sales: 'decimal', Orders: 'number' } })
aiws.result.columns('Diff', 'salary', Object.fromEntries(rows.map((r) => [r._id, { Diff: r['Salary'] - avg }])), { fields: { Diff: 'decimal' } })
aiws.result.record('kpi', { 'Total sales': 123.4, Orders: 12 })
aiws.result.text('commentary', markdown)
aiws.result.file('chart', 'output/chart.svg', 'image/svg+xml')
aiws.result.file('poster', await aiws.ai.image({ prompt: '…' }), { title: 'Poster' })
aiws.result.html('dashboard', { html, css, js, bindings: { orders: 'input:sales', summary: 'result:monthly' } })
aiws.result.control('plan', { kind: 'mindmap', children: '- Launch\n  - Market\n  - Product', config: { direction: 'right' }, title: 'Launch' })
aiws.result.control('more', { kind: 'mindmap', children: [{ text: 'Pricing', children: [] }], under: 'branch' })
aiws.result.control('board', { def: 'did:bns:alice#hello-board', config: { title: 'Week 44' },
  slots: { cards: ['result:summary', 'input:task_card'], header: [{ view: 'richtext', markdown: '# Week 44' }] } })
```

- Result names, types and keys follow the output contract; a name is written once. A table's `key` (one or more columns) keeps record identity across runs.
- Field types: `text | number | decimal | currency | boolean | date | datetime | select | multi_select`, inferred from the values when not declared. A `currency` value is `{ amount, currency: 'CNY' }` or a bare amount with `fields: [{ name: 'Price', type: 'currency', currency: 'CNY', scale: 2 }]`. `decimal` keeps up to 6 places; `fields: [{ name, type: 'decimal', scale: 4 }]` fixes them (round in the macro). A table shows in the order of `rows` (up to 1000 rows), so sort in the macro.
- **HTML results**: their html, css and js belong to the macro version — fixed when it is coded, confirmed when the candidate is applied. A later run may only change the data the page is bound to: pass data through `bindings` (`input:<name>`, `result:<name>`; the page reads them with `aiws.input('<name>')`), never by writing values into the code. A run whose HTML code differs from the coded code fails (`MACRO_CODE_CHANGED`) and the wish needs coding again.
- A control result never carries coordinates or sizes: the control lays its nodes out. Each item becomes a text node (at most 256 characters); at most 2000 nodes and 32 levels. With `under` the items go below that node of a control input.

## 6. Checks

```js
aiws.check('total', Math.abs(sumOfMonths - total) < 0.01, { sumOfMonths, total })   // a program check agreed in the analysis
```

Report each program check of the contract truthfully: when the data does not satisfy it, report a failure with the violating rows; do not narrow it to make it pass.

## 7. Zone resources

```js
const res = await aiws.zone.fetch('/kapi/some-service/…')      // a path of this Zone, or a URL on its hosts
const data = await res.json()
```

The request goes with the session of whoever runs the wish (the user's browser, or the Agent), and only to this Zone's hosts (else `ZONE_ONLY`). A macro only reads: GET or HEAD (else `ZONE_READ_ONLY`), and only under a path prefix (or URL prefix) it lists in `declare({ zone: [...] })` (else `ZONE_NOT_DECLARED`). What a macro reads this way is not part of its snapshot: the results' freshness cannot follow it, so data the macro uses every time should be a declared input instead.

## 8. What a macro does not do

- Reach anything outside the Zone (`fetch` of external URLs, WebSocket, importing modules).
- Read the clock: use `aiws.request.today` / `aiws.request.timezone`.
- Write the workspace, or files outside `output/`.
- Read inputs or fields it did not declare, or the rest of the workspace.

## Running it by hand

`deno run --allow-read=. --allow-write=output lib/run.js` in the stage directory runs the macro on the snapshot and writes `output/.aiws/results.json`; `aiws.sql`, `aiws.ai.*` and `aiws.zone.fetch` need an executor's bridge (`--bridge`, the hosted runner: `buckyos aiws run-program`).

## Example

```js
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['Month', 'Sales', 'Status'] }, ai: { complete: { max_calls: 2, max_tokens: 2000 } } })
  const { rows } = await aiws.sql(`SELECT "Month", round(sum("Sales"), 2) AS "Sales", count(*) AS "Orders"
    FROM sales WHERE "Status" <> 'Unsettled' GROUP BY 1 ORDER BY 1`)
  aiws.result.table('monthly', rows, { key: ['Month'], fields: { Month: 'date', Sales: 'decimal', Orders: 'number' } })
  const total = Math.round(rows.reduce((s, r) => s + r.Sales, 0) * 100) / 100
  aiws.facts({ total, months: rows.length })
  aiws.check('total', rows.every((r) => r.Sales >= 0), { total })
  const { text } = await aiws.ai.complete({
    system: 'Write two sentences of commentary. Use only the numbers in the facts.',
    prompt: `Facts: ${JSON.stringify({ total, months: rows.length })}`,
  })
  aiws.result.text('commentary', text)
}
```
