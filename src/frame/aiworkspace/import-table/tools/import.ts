/* The Deno host of the table importer (表格数据导入 TODO §8): a CSV / TSV / xlsx / SQLite file on disk → the same
 * grids, inference and plans the Desktop wizard makes with its defaults (and the few choices an Agent can pass) →
 * commits of the TableSource and its records through any client with `call` and `commit` (the websdk's
 * AiWorkspaceClient). No WASM here: the service checks every value and its refusal is reported as it is.
 *
 *   deno run -A tools/import.ts <file> [--header auto|yes|no] [--sheet <name> | --all-sheets] [--table <name>]…
 *                                      [--date-order md|dm] [--timezone <iana>] [--locale zh|en] [--title <t>]
 * prints the plan (fields, types, rows, report) without writing anything; the buckyos-agent command
 * `import_table` (src/tools/buckyos-agent/commands/import_table.ts) commits it. */

import { defaultChoices, listSqliteTables, openXlsx, planTable, readCsv, readSqliteTable, tableOperations, tabularKind, tidy, uniqueName, type Delimiter, type Encoding, type Grid, type TableChoices, type TablePlan } from '../mod.ts'
import { openSqliteFile } from './sqlite_deno.ts'

export interface ReadOptions { encoding?: Encoding | 'auto'; delimiter?: Delimiter | 'auto'; sheet?: string; allSheets?: boolean; tables?: string[]; views?: boolean }
export interface PlanOptions { title?: string; header?: 'auto' | 'yes' | 'no'; dateOrder?: 'md' | 'dm'; timezone?: string; locale?: string }

export class ImportFileError extends Error {}

/** The grids a file holds: one for a CSV, the chosen (else the first visible) sheet or every visible one, the chosen
 * (else every) SQLite table. */
export async function readTabularFile(path: string, options: ReadOptions = {}): Promise<{ kind: string; grids: Grid[] }> {
  const name = path.split(/[\\/]/).pop() ?? path
  const bytes = await Deno.readFile(path)
  const kind = tabularKind(name, bytes)
  switch (kind) {
    case 'xls': case 'numbers': case 'ods':
      throw new ImportFileError(`${kind} files are not supported: save as .xlsx or CSV`)
    case 'xlsx': {
      const wb = await openXlsx(bytes)
      const visible = wb.sheets.filter((s) => !s.hidden)
      const chosen = options.allSheets ? visible : options.sheet ? wb.sheets.filter((s) => s.name === options.sheet) : visible.slice(0, 1)
      if (chosen.length === 0) throw new ImportFileError(options.sheet ? `no sheet "${options.sheet}" (sheets: ${wb.sheets.map((s) => s.name).join(', ')})` : 'the workbook has no visible sheet')
      const grids: Grid[] = []
      for (const sheet of chosen) grids.push(tidy(await wb.read(sheet)))
      // a single sheet of a workbook takes the file's name
      if (!options.allSheets && !options.sheet && wb.sheets.length === 1) grids[0] = { ...grids[0], name: name.replace(/\.[^.]+$/, '') }
      return { kind, grids }
    }
    case 'sqlite': {
      const db = openSqliteFile(path)
      try {
        const tables = await listSqliteTables(db.query, options.views ?? false)
        const chosen = options.tables?.length ? options.tables : tables.map((t) => t.name)
        const missing = chosen.filter((t) => !tables.some((x) => x.name === t))
        if (missing.length) throw new ImportFileError(`no table ${missing.join(', ')} (tables: ${tables.map((t) => t.name).join(', ')})`)
        const grids: Grid[] = []
        for (const table of chosen) grids.push(tidy(await readSqliteTable(db.query, table)))
        return { kind, grids }
      } finally { db.close() }
    }
    case 'csv': case 'tsv': case null: {
      const read = readCsv(name, bytes, { encoding: options.encoding, delimiter: options.delimiter })
      return { kind: kind ?? 'csv', grids: [tidy(read.grid)] }
    }
  }
}

export interface FilePlan { grid: Grid; choices: TableChoices; plan: TablePlan }

/** What the wizard would do without a change (header switch, date order, time zone and title aside). */
export function planGrids(grids: Grid[], options: PlanOptions = {}): FilePlan[] {
  return grids.map((grid, i) => {
    const header = options.header === 'yes' ? true : options.header === 'no' ? false : undefined
    const choices = defaultChoices(grid, { locale: options.locale ?? 'en', dateOrder: options.dateOrder, timezone: options.timezone ?? 'UTC', header })
    if (options.title && grids.length === 1) choices.title = options.title
    else if (options.title) choices.title = `${options.title} ${i + 1}`
    return { grid, choices, plan: planTable(grid, choices) }
  })
}

/** A plan as an Agent reads it before committing (`--dry-run`). */
export function describePlans(plans: FilePlan[]) {
  return plans.map(({ choices, plan }) => ({
    title: plan.title, header: choices.header, rows: plan.records.length,
    fields: plan.fields, title_field_id: plan.titleFieldId,
    sample: plan.records.slice(0, 3),
    report: plan.report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples, message: n.message })),
  }))
}

/** What committing needs of the service client. */
export interface CommitClient {
  call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>>
  commit(workspaceId: string, request: Record<string, unknown>): Promise<Record<string, unknown>>
}

export interface Imported { entity_id: string; name: string; rows: number }

/** Commit the plans under `parentId` (a data folder, by default the data root): names made unique among its
 * children, every table its own commits (≤ 20 000 rows / 6 MiB each). Stops at the first refused commit. */
export async function commitPlans(client: CommitClient, workspaceId: string, plans: FilePlan[], parentId = 'data', progress?: (text: string) => void): Promise<{ imported: Imported[]; error?: { code: string; detail: string; table: string; rows_done: number } }> {
  const info = await client.call('ws.get_info', { workspace_id: workspaceId })
  const epoch = String(info.epoch)
  const outline = (await client.call('doc.outline', { workspace_id: workspaceId })).entities as { entity_id: string; parent_id?: string; order_key?: string; name?: string; deleted?: boolean }[]
  const siblings = outline.filter((e) => e.parent_id === parentId && !e.deleted).sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : 1))
  if (!outline.some((e) => e.entity_id === parentId && !e.deleted)) throw new ImportFileError(`no folder ${parentId} in ${workspaceId}`)
  const taken = new Set(siblings.map((e) => e.name).filter((n): n is string => Boolean(n)))
  let afterKey = siblings.at(-1)?.order_key ?? null
  const imported: Imported[] = []
  for (const { plan } of plans) {
    const name = uniqueName(plan.title, taken)
    taken.add(name)
    const ops = tableOperations(plan, { folderId: parentId, afterKey, name })
    afterKey = (ops.commits[0].operations[0].order_key as string) ?? afterKey
    let done = 0
    for (const [k, commit] of ops.commits.entries()) {
      progress?.(`${name}: rows ${commit.firstRow + 1}–${commit.firstRow + commit.rows} of ${plan.records.length}`)
      const result = await client.commit(workspaceId, { protocol_version: '1.0', epoch, idempotency_key: `import-table/${ops.sourceId}/${k}`, session_id: 'import-table', operations: commit.operations })
      if (result.status !== 'accepted') {
        const error = (result.errors as { code?: string; detail?: string }[] | undefined)?.[0]
        return { imported, error: { code: String(result.code ?? error?.code ?? result.status), detail: String(result.detail ?? error?.detail ?? JSON.stringify(result).slice(0, 400)), table: name, rows_done: done } }
      }
      done += commit.rows
    }
    imported.push({ entity_id: ops.sourceId, name, rows: done })
  }
  return { imported }
}

/** `--name value` / `--flag` arguments (repeatable `--table`). */
export function parseImportArgs(argv: string[]): { file: string | null; read: ReadOptions; plan: PlanOptions; flags: Map<string, string | true> } {
  const flags = new Map<string, string | true>()
  const tables: string[] = []
  let file: string | null = null
  const booleans = new Set(['all-sheets', 'views', 'dry-run'])
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (!a.startsWith('--')) { file ??= a; continue }
    const name = a.slice(2)
    if (booleans.has(name)) { flags.set(name, true); continue }
    const value = argv[++i]
    if (value === undefined) throw new ImportFileError(`--${name} needs a value`)
    if (name === 'table') tables.push(value); else flags.set(name, value)
  }
  const str = (k: string) => (typeof flags.get(k) === 'string' ? flags.get(k) as string : undefined)
  const header = str('header')
  if (header && !['auto', 'yes', 'no'].includes(header)) throw new ImportFileError('--header is auto, yes or no')
  const dateOrder = str('date-order')
  if (dateOrder && dateOrder !== 'md' && dateOrder !== 'dm') throw new ImportFileError('--date-order is md or dm')
  return {
    file,
    read: { sheet: str('sheet'), allSheets: flags.get('all-sheets') === true, tables, views: flags.get('views') === true, encoding: str('encoding') as Encoding | undefined, delimiter: str('delimiter') === 'tab' ? '\t' : str('delimiter') as Delimiter | undefined },
    plan: { title: str('title'), header: header as PlanOptions['header'], dateOrder: dateOrder as PlanOptions['dateOrder'], timezone: str('timezone'), locale: str('locale') },
    flags,
  }
}

if (import.meta.main) {
  const args = parseImportArgs(Deno.args)
  if (!args.file) { console.error('usage: deno run -A tools/import.ts <file> [--header auto|yes|no] [--sheet <name> | --all-sheets] [--table <name>]… [--date-order md|dm] [--timezone <iana>] [--locale zh|en] [--title <t>]'); Deno.exit(2) }
  const { grids } = await readTabularFile(args.file, args.read)
  console.log(JSON.stringify(describePlans(planGrids(grids, args.plan)), null, 2))
}
