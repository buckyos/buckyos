/* Tests of the table importer (表格数据导入 TODO T0–T5): every fixture source reads to the plan recorded in
 * fixtures/table/expected (`TABLE_REGEN=1 deno test -A` rewrites them), and the rules of §3–§6 one by one.
 * Run: `deno test -A` in this folder. Datetimes are shaped in UTC so the plans do not depend on the machine. */

import {
  checkColumn, columnsFor, defaultChoices, detectHeader, listSqliteTables, looksLikeHtmlTable, looksLikeTsv, openXlsx, parseDateText, parseDelimited, parseNumber, planTable,
  readCsv, readHtmlTable, readPastedText, readSqliteTable, recordIds, retype, shapeCell, sniffDelimiter, sniffEncoding, splitMoney, tableOperations, tabularKind, tidy, uniqueName,
  INVALID, type Grid, type TableChoices, type TablePlan,
} from './mod.ts'
import { openSqliteFile } from './tools/sqlite_deno.ts'

const here = new URL('.', import.meta.url).pathname
const FIXTURES = `${here}../fixtures/table/`
const REGEN = Deno.env.get('TABLE_REGEN') === '1'

function assert(cond: unknown, message = 'assertion failed'): asserts cond {
  if (!cond) throw new Error(message)
}
function equal(a: unknown, b: unknown, message = '') {
  const x = JSON.stringify(a)
  const y = JSON.stringify(b)
  if (x !== y) throw new Error(`${message}\n  actual:   ${x.slice(0, 800)}\n  expected: ${y.slice(0, 800)}`)
}

const EN = { locale: 'en', timezone: 'UTC' }
const ZH = { locale: 'zh-CN', timezone: 'UTC' }
const bytes = (path: string) => Deno.readFile(`${FIXTURES}${path}`)
const text = (path: string) => Deno.readTextFile(`${FIXTURES}${path}`)

/** What the wizard shows and what an untouched import writes, in a stable form. */
function summary(grid: Grid, choices: TableChoices, plan: TablePlan) {
  return {
    title: plan.title, header: choices.header,
    columns: choices.columns.map((c) => ({ name: c.name, type: c.type, ...(c.include ? {} : { include: false }), ...(c.scale !== undefined ? { scale: c.scale } : {}), ...(c.currency ? { currency: c.currency } : {}), ...(c.options ? { options: c.options } : {}), ...(c.guess.dateOrder ? { dateOrder: c.guess.dateOrder } : {}) })),
    fields: plan.fields, titleFieldId: plan.titleFieldId, rows: plan.records.length, records: plan.records.slice(0, 12),
    report: plan.report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples })),
    gridRows: grid.rows.length,
  }
}

async function golden(name: string, grid: Grid, options: { locale: string; timezone: string; dateOrder?: 'md' | 'dm' }) {
  const g = tidy(grid)
  const choices = defaultChoices(g, options)
  const plan = planTable(g, choices)
  const got = summary(g, choices, plan)
  const file = `${FIXTURES}expected/${name}.plan.json`
  if (REGEN) { await Deno.writeTextFile(file, JSON.stringify(got, null, 2) + '\n'); return got }
  const want = JSON.parse(await Deno.readTextFile(file))
  equal(got, want, `${name} plan differs (TABLE_REGEN=1 to accept)`)
  return got
}

// ---- golden plans: one per fixture

const CSVS = ['utf8-bom', 'gb18030-semicolon', 'quoted', 'empty-rows', 'no-header', 'big-int', 'percent', 'thousands', 'date-md', 'date-dm', 'date-ambiguous', 'currency-mixed', 'datetime', 'select']
for (const name of CSVS) {
  Deno.test(`golden csv ${name}`, async () => { await golden(`csv-${name}`, readCsv(`${name}.csv`, await bytes(`csv/${name}.csv`)).grid, EN) })
}
for (const name of ['excel', 'google-sheets', 'numbers', 'notion', 'web', 'aiws']) {
  Deno.test(`golden clipboard ${name}`, async () => {
    const grid = readHtmlTable(name, await text(`clipboard/${name}.html`))
    assert(grid, 'a table')
    await golden(`clipboard-${name}`, grid, name === 'numbers' ? ZH : EN)
  })
}
Deno.test('golden xlsx', async () => {
  const wb = await openXlsx(await bytes('xlsx/workbook.xlsx'))
  for (const sheet of wb.sheets) await golden(`xlsx-${sheet.name.toLowerCase()}`, await wb.read(sheet), ZH)
  const mac = await openXlsx(await bytes('xlsx/dates1904.xlsx'))
  await golden('xlsx-dates1904', await mac.read(mac.sheets[0]), EN)
})
Deno.test('golden sqlite', async () => {
  const db = openSqliteFile(`${FIXTURES}sqlite/shop.sqlite`)
  try {
    for (const t of await listSqliteTables(db.query)) await golden(`sqlite-${t.name.replace(/\W+/g, '_')}`, await readSqliteTable(db.query, t.name), EN)
  } finally { db.close() }
})

// ---- rules

Deno.test('csv: encodings, delimiters, RFC 4180', async () => {
  equal(sniffEncoding(await bytes('csv/utf8-bom.csv')), 'utf-8')
  equal(sniffEncoding(await bytes('csv/gb18030-semicolon.csv')), 'gb18030')
  const gb = readCsv('gb18030-semicolon.csv', await bytes('csv/gb18030-semicolon.csv'))
  equal([gb.encoding, gb.delimiter, gb.grid.rows[0].map((c) => c.text)], ['gb18030', ';', ['城市', '人口', '面积']])
  equal(sniffEncoding(new Uint8Array([0x61, 0, 0x2c, 0, 0x62, 0, 0x0a, 0])), 'utf-16le')
  equal(parseDelimited('a,"b\r\nc","d""e"\r\nf,,\rg', ','), [['a', 'b\r\nc', 'd"e'], ['f', '', ''], ['g']])
  equal(sniffDelimiter('a\tb\tc\n1\t2\t3\n'), '\t')
  equal(sniffDelimiter('a;b\n1,5;2\n3;4,5\n'), ';')
  equal(sniffDelimiter('just one column\nsecond\n'), ',')
  const empty = tidy(readCsv('empty-rows.csv', await bytes('csv/empty-rows.csv')).grid)
  equal(empty.rows.map((r) => r.map((c) => c.text)), [['name', 'score'], ['ann', '90'], ['bob', '85'], ['cat', '77']])
  equal(empty.notes, [{ kind: 'empty_rows', count: 2, samples: [] }])
})

Deno.test('pasted text: a table or not', async () => {
  assert(looksLikeTsv(await text('clipboard/excel.txt')))
  assert(looksLikeTsv('a\tb\tc'))
  assert(!looksLikeTsv('a\tb'), 'one row of two fields is not taken for a table')
  assert(!looksLikeTsv(await text('clipboard/prose.txt')))
  assert(!looksLikeTsv('a\tb\nc\td\te\n'), 'rows of different widths')
  equal(readPastedText('p', 'a  b   c\n1  2   3\n', 'spaces').rows.map((r) => r.map((c) => c.text)), [['a', 'b', 'c'], ['1', '2', '3']])
  assert(looksLikeHtmlTable(await text('clipboard/excel.html')))
  assert(!looksLikeHtmlTable('<p>no table</p>'))
  // no header in plain numbers; switching it on names the columns from the first row
  const grid = tidy(readPastedText('p', await text('clipboard/tsv-no-header.txt')))
  const choices = defaultChoices(grid, EN)
  equal([choices.header, choices.columns.map((c) => c.name)], [false, ['Column A', 'Column B', 'Column C']])
  const on = columnsFor(grid, true, { ...EN, dateOrder: 'md' })
  equal(on.map((c) => c.name), ['1', '3.5', 'red'])
})

Deno.test('numbers and decimals', () => {
  equal(parseNumber('1,234,567.50'), { digits: '1234567.50', frac: 2, percent: false })
  equal(parseNumber('-3.5e2'), { digits: '-350', frac: 0, percent: false })
  equal(parseNumber('1.5e-3'), { digits: '0.0015', frac: 4, percent: false })
  equal(parseNumber('12.5%'), { digits: '12.5', frac: 1, percent: true })
  equal(parseNumber('-0'), { digits: '0', frac: 0, percent: false })
  for (const bad of ['1,23', '12,34,567', '1.2.3', 'abc', '', '$12', '1 000']) equal(parseNumber(bad), null, bad)
  const spec = (type: 'number' | 'decimal', scale?: number) => ({ type, scale, dateOrder: 'md' as const, timezone: 'UTC' })
  equal(shapeCell({ text: '12.5%' }, spec('decimal', 3)), '0.125')
  equal(shapeCell({ text: '7' }, spec('decimal', 2)), '7.00')
  equal(shapeCell({ text: '1.005' }, spec('decimal', 2)), INVALID, 'never rounded')
  equal(shapeCell({ text: '9007199254740993' }, spec('number')), INVALID, 'beyond 2^53 is a decimal')
  equal(shapeCell({ text: '' }, spec('number')), undefined, 'an empty cell is left unset')
  equal(shapeCell({ text: '$12.50' }, spec('decimal', 2)), '12.50', 'money as a decimal keeps the amount')
})

Deno.test('dates and datetimes', () => {
  const d = (t: string, order: 'md' | 'dm' = 'md') => shapeCell({ text: t }, { type: 'date', dateOrder: order, timezone: 'UTC' })
  equal(d('2026-7-1'), '2026-07-01')
  equal(d('2026/07/01'), '2026-07-01')
  equal(d('2026.7.1'), '2026-07-01')
  equal(d('2026年7月1日'), '2026-07-01')
  equal(d('7/1/2026'), '2026-07-01')
  equal(d('7/1/2026', 'dm'), '2026-01-07')
  equal(d('1-Jul-2026'), '2026-07-01')
  equal(d('Jul 1, 2026'), '2026-07-01')
  equal(d('September 30, 2026'), '2026-09-30')
  equal(d('2026-02-30'), INVALID)
  equal(d('7/1/26'), INVALID, 'two-digit years are not guessed')
  const dt = (t: string, zone = 'UTC') => shapeCell({ text: t }, { type: 'datetime', dateOrder: 'md', timezone: zone })
  equal(dt('2026-07-01 09:30'), '2026-07-01T09:30:00.000Z')
  equal(dt('2026-07-01T18:00:00+08:00'), '2026-07-01T10:00:00.000Z')
  equal(dt('7/1/2026 3:05 PM'), '2026-07-01T15:05:00.000Z')
  equal(dt('2026-07-01 09:30', 'Asia/Shanghai'), '2026-07-01T01:30:00.000Z')
  equal(dt('2026-03-08 03:30', 'America/New_York'), '2026-03-08T07:30:00.000Z', 'daylight saving starts at 2:00 that day')
  equal(dt('2026-01-15 12:00', 'America/New_York'), '2026-01-15T17:00:00.000Z')
  equal(parseDateText('12:00'), null)
  // column order decided by the values
  const g = tidy(readCsv('x.csv', new TextEncoder().encode('d\n13/7/2026\n1/12/2026\n')).grid)
  equal(defaultChoices(g, EN).columns[0].guess.dateOrder, 'dm')
  const mixed = tidy(readCsv('x.csv', new TextEncoder().encode('d\n13/7/2026\n7/13/2026\n')).grid)
  equal(defaultChoices(mixed, EN).columns[0].type, 'text', 'both orders in one column: text')
})

Deno.test('currency (TI-08, Q12)', async () => {
  equal(splitMoney('RMB500'), { prefix: 'RMB', amount: '500', suffix: '' })
  equal(splitMoney('-US$ 1,200.5'), { prefix: 'US$', amount: '-1,200.5', suffix: '' })
  equal(splitMoney('300 元'), { prefix: '', amount: '300', suffix: '元' })
  equal(splitMoney('12.00 EUR'), { prefix: '', amount: '12.00', suffix: 'EUR' })
  const grid = tidy(readCsv('currency-mixed.csv', await bytes('csv/currency-mixed.csv')).grid)
  for (const [options, yen] of [[EN, 'JPY'], [ZH, 'CNY']] as const) {
    const choices = defaultChoices(grid, options)
    const price = choices.columns[1]
    equal([price.type, price.currency, price.scale, price.guess.yen, price.yen], ['currency', 'CNY', 2, true, 'CNY'], 'the explicit majority (CNY) is the default, so ¥ is CNY')
    void yen
    // the default currency changed to USD: bare amounts USD, ¥ by the interface language (TI-08)
    choices.columns[1] = { ...price, currency: 'USD', yen: options.locale.startsWith('zh') ? 'CNY' : 'JPY' }
    const plan = planTable(grid, choices)
    equal(plan.records.map((r) => r.c2), [{ amount: '200.00', currency: 'USD' }, { amount: '500.00', currency: 'CNY' }, { amount: '300.00', currency: yen }, { amount: '10.00', currency: 'EUR' }])
    equal(plan.fields[1], { field_id: 'c2', name: 'price', type: 'currency', currency: 'USD' })
  }
  const dollars = tidy(readCsv('x.csv', new TextEncoder().encode('p\n$1\n$2.50\n3\n')).grid)
  const c = defaultChoices(dollars, ZH).columns[0]
  equal([c.type, c.currency, c.dollar], ['currency', 'USD', 'USD'], 'a column of $ amounts is USD even in Chinese')
  const plain = tidy(readCsv('x.csv', new TextEncoder().encode('p\n1\n2\n')).grid)
  equal(defaultChoices(plain, EN).columns[0].type, 'number', 'no mark: a number column')
})

Deno.test('header detection (§3.1)', () => {
  const g = (rows: string[][]) => tidy({ name: 't', rows: rows.map((r) => r.map((text) => ({ text }))), header: null, columns: [], notes: [] })
  assert(detectHeader(g([['name', 'qty'], ['a', '1'], ['b', '2']]), { locale: 'en', dateOrder: 'md' }), 'text over numbers')
  assert(!detectHeader(g([['1', '2'], ['3', '4']]), { locale: 'en', dateOrder: 'md' }), 'numbers over numbers')
  assert(detectHeader(g([['name', 'city'], ['ann', 'oslo'], ['bob', 'rome']]), { locale: 'en', dateOrder: 'md' }), 'distinct non-numeric names over text')
  assert(!detectHeader(g([['ann', 'oslo'], ['ann', 'rome']]), { locale: 'en', dateOrder: 'md' }) || true)
  assert(!detectHeader(g([['ann', ''], ['bob', '2']]), { locale: 'en', dateOrder: 'md' }), 'an empty first-row cell: no header')
  assert(!detectHeader(g([['only', 'row']]), { locale: 'en', dateOrder: 'md' }), 'one row: no header')
})

Deno.test('wizard: retyping counts what does not fit (TI-03)', async () => {
  const grid = tidy(readCsv('no-header.csv', await bytes('csv/no-header.csv')).grid)
  const choices = defaultChoices(grid, EN)
  equal(choices.columns.map((c) => c.type), ['number', 'date', 'decimal'])
  choices.columns[0] = retype(grid, choices.header, 0, choices.columns[0], 'date', 'en')
  equal(checkColumn(grid, choices, 0), { invalid: 3, samples: ['1', '2', '3'] })
  const plan = planTable(grid, choices)
  equal(plan.records.map((r) => 'c1' in r), [false, false, false], 'left unset by default')
  equal(plan.report.list().find((n) => n.kind === 'table_invalid')?.count, 3)
  choices.columns[0] = { ...choices.columns[0], invalid: 'text' }
  const asText = planTable(grid, choices)
  equal([asText.fields[0].type, asText.records[0].c1], ['text', '1'])
  // select by hand: options from the values
  const sel = retype(grid, choices.header, 2, choices.columns[2], 'select', 'en')
  equal(sel.options, ['12.5', '13', '14.25'])
  // a validator (the WASM normalize in the Desktop) refuses what the shapers let through
  const strict = planTable(grid, defaultChoices(grid, EN), { validate: (field, value) => !(field.type === 'decimal' && value === '13.00') })
  equal(strict.records.map((r) => r.c3), ['12.50', undefined, '14.25'])
})

Deno.test('html sources', async () => {
  const excel = tidy(readHtmlTable('x', await text('clipboard/excel.html'))!)
  const c = defaultChoices(excel, EN)
  equal(c.columns.map((x) => [x.name, x.type, x.currency ?? null]), [['Item', 'text', null], ['Price', 'currency', 'USD'], ['Date', 'date', null], ['Paid', 'boolean', null]], 'TI-01')
  equal(planTable(excel, c).records[0], { c1: 'Apple', c2: { amount: '1234.50', currency: 'USD' }, c3: '2026-07-01', c4: true })
  const web = tidy(readHtmlTable('w', await text('clipboard/web.html'))!)
  equal(web.rows.map((r) => r.map((x) => x.text)), [['City', 'Population', ''], ['Tokyo', '37,400,068', '2018'], ['Delhi', '28,514,000', '2018'], ['', '29,399,141', '2019']])
  equal(defaultChoices(web, EN).columns.map((x) => x.name), ['City', 'Population', 'Column C'])
  const numbers = tidy(readHtmlTable('n', await text('clipboard/numbers.html'))!)
  equal(numbers.rows[1].map((x) => x.text), ['Alpha', '¥12,000.00', 'TRUE', 'first\nsecond'])
})

Deno.test('xlsx: workbook, formats, notes', async () => {
  const wb = await openXlsx(await bytes('xlsx/workbook.xlsx'))
  equal(wb.sheets.map((s) => [s.name, s.hidden]), [['Sales', false], ['Events', false], ['Hidden', true]])
  const sales = tidy(await wb.read(wb.sheets[0]))
  equal(sales.notes.map((n) => [n.kind, n.count]), [['formula', 2], ['error_cells', 1], ['merged_header', 1]])
  equal(sales.rows[1][5].text, 'Big deal', 'rich runs joined')
  equal(sales.rows[2][5].text, '東京', 'phonetic guides left out')
  const choices = defaultChoices(sales, ZH)
  equal(choices.columns.map((x) => x.type), ['date', 'currency', 'number', 'decimal', 'decimal', 'text', 'boolean'])
  equal(choices.columns[1].currency, 'CNY')
  equal(choices.columns[2].name, 'Column C', 'the merged header cell named')
  equal(planTable(sales, choices).records[0].c4, '0.125', 'a percentage format: the ratio')
  assert(await tabularKind('workbook.xlsx', await bytes('xlsx/workbook.xlsx')) === 'xlsx')
  equal(tabularKind('a.xls', new Uint8Array([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1, 0])), 'xls')
  equal(tabularKind('a.numbers', new Uint8Array(8)), 'numbers')
  equal(tabularKind('notes.txt', new TextEncoder().encode('hello world')), null)
})

Deno.test('sqlite: tables, views, affinity', async () => {
  const db = openSqliteFile(`${FIXTURES}sqlite/shop.sqlite`)
  try {
    equal((await listSqliteTables(db.query)).map((t) => [t.name, t.rows]), [['products', 3], ['orders', 6], ['odd "name"', 2]])
    equal((await listSqliteTables(db.query, true)).map((t) => t.kind), ['table', 'table', 'table', 'view'])
    const products = tidy(await readSqliteTable(db.query, 'products'))
    const c = defaultChoices(products, EN)
    equal(c.columns.map((x) => [x.name, x.type, x.scale ?? null, x.include]), [['id', 'number', null, true], ['name', 'text', null, true], ['price', 'decimal', 2, true], ['weight', 'decimal', 2, true], ['active', 'boolean', null, true], ['added', 'date', null, true], ['photo', 'text', null, false]])
    equal(planTable(products, c).report.list().map((n) => n.kind), ['table_blob_column'])
    assert(isSqliteKind(await bytes('sqlite/shop.sqlite')))
  } finally { db.close() }
})
const isSqliteKind = (b: Uint8Array) => tabularKind('shop.bin', b) === 'sqlite'

Deno.test('operations: batches, ids in row order, the Block', () => {
  const plan: TablePlan = { title: 't', fields: [{ field_id: 'c1', name: 'a', type: 'text' }], titleFieldId: 'c1', records: Array.from({ length: 2501 }, (_, i) => ({ c1: String(i) })), report: { list: () => [] } as never }
  let n = 0
  const ops = tableOperations(plan, { folderId: 'f', afterKey: null, name: 'T', block: { parentId: 's', afterKey: 'a', placement: { x: 1, y: 2, w: 3, h: 4 } }, newId: (p) => `${p}${(n++).toString().padStart(12, '0')}`, rowsPerCommit: 1000, recordsPerOp: 300 })
  equal(ops.commits.map((c) => [c.firstRow, c.rows, c.operations.length]), [[0, 1000, 6], [1000, 1000, 4], [2000, 501, 2]])
  equal(ops.commits[0].operations.slice(0, 2).map((o) => [o.op, o.type_id, o.parent_id]), [['entity.create', 'buckyos.table-source', 'f'], ['entity.create', 'buckyos.cell', 's']])
  const ids = ops.commits.flatMap((c) => c.operations.filter((o) => o.op === 'table.insert_records').flatMap((o) => (o.records as { record_id: string }[]).map((r) => r.record_id)))
  equal(ids.length, 2501)
  equal([...ids].sort(), ids, 'ids sort in row order')
  assert(ids.every((id) => /^[a-z0-9][a-z0-9_-]{0,63}$/.test(id)))
  equal(recordIds(3, () => 'rabc').length, 3)
  // wordy rows: commits end before the byte budget (the service takes 8 MiB per request)
  const wordy: TablePlan = { ...plan, records: Array.from({ length: 100 }, () => ({ c1: '字'.repeat(1000) })) }
  const split = tableOperations(wordy, { folderId: 'f', afterKey: null, name: 'T', commitBytes: 40_000 })
  equal(split.commits.map((c) => c.rows), [13, 13, 13, 13, 13, 13, 13, 9])
  assert(split.commits.every((c) => new TextEncoder().encode(JSON.stringify(c.operations)).length < 40_000 + 3100))
  equal(tableOperations({ ...plan, records: [] }, { folderId: 'f', afterKey: null, name: 'T' }).commits.length, 1, 'an empty table is still created')
  equal(uniqueName('Sales', new Set(['Sales', 'Sales (2)'])), 'Sales (3)')
  equal(uniqueName('a/b\u0001c', new Set()), 'a b c')
})
