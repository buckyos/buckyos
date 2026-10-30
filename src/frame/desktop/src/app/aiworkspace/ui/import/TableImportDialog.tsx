/* The table import wizard (表格数据导入 TODO §7): one screen. A dropped or pasted single file first asks: a table,
 * or a file (as PDFs do). Then the source's facts, the header switch, the title, and per column: include, name,
 * type (with "auto" while it is the inferred one), the default currency and what `$` / `¥` mean, and — when values do
 * not fit a type the user picked — how many, an example, and leave empty / whole column as text. Advanced: encoding,
 * delimiter, worksheet(s), SQLite tables and views, date order, time zone, keeping the original file. Then progress
 * with cancel; the report when something was lost. Mounted at the workspace shell, so the data tree can use it. */

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { checkColumn, columnsFor, defaultChoices, ENCODINGS, IMPORT_TYPES, ISO_CODES, MAX_CELLS, MAX_ROWS, retype, type ColumnChoice, type ColumnCheck, type DateOrder, type Delimiter, type Encoding, type ImportType, type TableChoices } from '@table/mod'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useOverlayMounted } from '../shell/popover'
import { ImportReportDialog } from '../canvas/ImportReportDialog'
import type { ImportOutcome } from '../canvas/drawioImport'
import { FIELD_TYPE_LABEL } from '../values'
import { runTableImport, wasmValidate, type ImportPart, type TableImportProgress } from './tableImport'
import { tableImports, type TableImportRequest } from './tableImportRequest'
import { baseName, KEEP_LIMIT, openSource, readPart, TableReader, TableReadError, type OpenedSource } from './tabularFiles'

export function TableImportHost() {
  const request = useSyncExternalStore(tableImports.subscribe, tableImports.snapshot)
  const [report, setReport] = useState<ImportOutcome | null>(null)
  return (
    <>
      {request && <TableImportDialog key={request.source.kind === 'file' ? request.source.name + request.source.bytes.length : 'clipboard'} request={request} onClose={() => tableImports.open(null)}
        onDone={(outcome) => { if (outcome.error || outcome.report.notes.size > 0) setReport(outcome) }} />}
      {report && <ImportReportDialog outcome={report} onClose={() => setReport(null)} />}
    </>
  )
}

const PREVIEW_ROWS = 8
const DELIMITER_LABEL: Record<Delimiter, string> = { ',': 'aiworkspace.import.delimiterComma', ';': 'aiworkspace.import.delimiterSemicolon', '\t': 'aiworkspace.import.delimiterTab', '|': 'aiworkspace.import.delimiterPipe' }

function TableImportDialog({ request, onClose, onDone }: { request: TableImportRequest; onClose: () => void; onDone: (outcome: ImportOutcome) => void }) {
  useOverlayMounted()
  const store = useStore()
  const { t, locale, dateTime } = useAiwsText()
  const [step, setStep] = useState(request.step)
  const reader = useMemo(() => new TableReader(), [])
  useEffect(() => () => reader.dispose(), [reader])
  const [pastedName] = useState(() => t('aiworkspace.import.pastedName', { time: dateTime(Date.now()) }))
  const [opened, setOpened] = useState<OpenedSource | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [encoding, setEncoding] = useState<Encoding | 'auto'>('auto')
  const [delimiter, setDelimiter] = useState<Delimiter | 'auto'>('auto')
  const [sheet, setSheet] = useState<string>('')
  const [allSheets, setAllSheets] = useState(false)
  const [views, setViews] = useState(false)
  const [tables, setTables] = useState<string[] | null>(null)
  const [parts, setParts] = useState<ImportPart[] | null>(null)
  const [current, setCurrent] = useState(0)
  const [dateOrder, setDateOrder] = useState<DateOrder>('md')
  const [timezone, setTimezone] = useState('local')
  const [keep, setKeep] = useState<boolean | null>(null)
  const [advanced, setAdvanced] = useState(false)
  const [progress, setProgress] = useState<TableImportProgress | null>(null)
  const abort = useRef<AbortController | null>(null)
  const { source } = request
  const fileBytes = source.kind === 'file' ? source.bytes.length : 0
  const live = store.session.status().kind === 'live'

  // 1 open the source (again when the encoding or delimiter changes)
  useEffect(() => {
    if (step !== 'wizard') return
    let cancelled = false
    openSource(source, reader, pastedName, { encoding, delimiter }).then((o) => {
      if (cancelled) return
      setOpened(o)
      if (o.kind === 'xlsx') setSheet((s) => s || (o.sheets.find((x) => !x.hidden) ?? o.sheets[0])?.name || '')
      if (o.kind === 'sqlite') setTables((x) => x ?? o.tables.map((tb) => tb.name))
    }, (e: unknown) => { if (!cancelled) setError(e instanceof TableReadError ? e.message : t('aiworkspace.import.unreadable', { reason: e instanceof Error ? e.message : String(e) })) })
    return () => { cancelled = true }
  }, [step, source, reader, pastedName, encoding, delimiter, t])

  // the SQLite views switch lists them (or not)
  useEffect(() => {
    if (opened?.kind !== 'sqlite') return
    let cancelled = false
    void reader.call({ op: 'list-tables', views }).then((r) => {
      if (cancelled || !r.tables) return
      setOpened({ ...opened, tables: r.tables })
      setTables((x) => (x ?? []).filter((name) => r.tables!.some((tb) => tb.name === name)))
    })
    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only the switch re-lists
  }, [views])

  // 2 the grids to import: the paste or file, a worksheet (or all), the chosen SQLite tables
  const wanted = useMemo(() => {
    if (!opened) return null
    if (opened.kind === 'xlsx') return allSheets ? opened.sheets.filter((s) => !s.hidden).map((s) => s.name) : sheet ? [sheet] : []
    if (opened.kind === 'sqlite') return tables ?? []
    return ['']
  }, [opened, allSheets, sheet, tables])
  useEffect(() => {
    if (!opened || !wanted) return
    let cancelled = false
    const options = { locale, dateOrder, timezone, columnName: (letter: string) => t('aiworkspace.import.columnName', { letter }) }
    const load = async (): Promise<ImportPart[]> => {
      if (opened.kind === 'clipboard') return [{ key: 'clipboard', label: opened.name, grid: opened.grid, choices: defaultChoices(opened.grid, options) }]
      if (opened.kind === 'csv' || opened.kind === 'tsv') return [{ key: 'file', label: opened.name, grid: opened.read.grid, choices: { ...defaultChoices(opened.read.grid, options), title: baseName(opened.name) } }]
      const out: ImportPart[] = []
      for (const name of wanted) {
        const known = parts?.find((p) => p.key === name)
        if (known) { out.push(known); continue }
        const grid = await readPart(reader, name)
        const choices = defaultChoices(grid, options)
        out.push({ key: name, label: name, grid, choices: { ...choices, title: opened.kind === 'xlsx' && !allSheets && opened.sheets.length === 1 ? baseName(opened.name) : name } })
      }
      return out
    }
    load().then((loaded) => { if (!cancelled) { setParts(loaded); setCurrent((c) => Math.min(c, Math.max(0, loaded.length - 1))) } }, (e: unknown) => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)) })
    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- parts are reused, not a trigger
  }, [opened, wanted, reader, locale, t])

  const part = parts?.[current] ?? null
  const setChoices = (next: TableChoices) => setParts((ps) => (ps ? ps.map((p, i) => (i === current ? { ...p, choices: next } : p)) : ps))
  const setColumn = (index: number, next: ColumnChoice) => part && setChoices({ ...part.choices, columns: part.choices.columns.map((c, i) => (i === index ? next : c)) })
  // date order and time zone apply to every part
  const setShaping = (next: { dateOrder?: DateOrder; timezone?: string }) => {
    if (next.dateOrder) setDateOrder(next.dateOrder)
    if (next.timezone) setTimezone(next.timezone)
    setParts((ps) => ps?.map((p) => ({ ...p, choices: { ...p.choices, ...next } })) ?? ps)
  }

  // what does not fit: only columns the user changed (inferred types fit by construction); the core decides
  const validate = useMemo(() => wasmValidate(store), [store])
  const checks = useMemo<(ColumnCheck | null)[]>(() => {
    if (!part) return []
    return part.choices.columns.map((c, i) => (c.include && (c.type !== c.guess.type || c.scale !== c.guess.scale || c.options?.length !== c.guess.options?.length) ? checkColumn(part.grid, part.choices, i, validate) : null))
  }, [part, validate])

  const keepDefault = opened?.kind === 'xlsx' || opened?.kind === 'sqlite'
  const keepAllowed = source.kind === 'file' && fileBytes <= KEEP_LIMIT && live
  const keeping = keepAllowed && (keep ?? keepDefault)
  const running = progress !== null
  const body = part ? part.grid.rows.length - (part.choices.header ? 1 : 0) : 0
  const included = part ? part.choices.columns.filter((c) => c.include).length : 0
  const rowsTotal = parts?.reduce((n, p) => n + p.grid.rows.length - (p.choices.header ? 1 : 0), 0) ?? 0
  const overLimit = parts?.some((p) => { const rows = p.grid.rows.length - (p.choices.header ? 1 : 0); return rows > MAX_ROWS || rows * p.choices.columns.filter((c) => c.include).length > MAX_CELLS }) ?? false
  const problem = !parts ? null : parts.length === 0 ? t('aiworkspace.import.nothingChosen') : overLimit ? t('aiworkspace.import.tooLarge', { rows: body, cells: body * included }) : parts.some((p) => p.grid.rows.length === 0) ? t('aiworkspace.import.empty') : parts.some((p) => !p.choices.columns.some((c) => c.include)) ? t('aiworkspace.import.noColumns') : null

  const start = async () => {
    if (!parts || problem) return
    const control = new AbortController()
    abort.current = control
    setProgress({ stage: 'plan', done: 0, total: parts.length })
    let outcome: ImportOutcome
    try {
      outcome = await runTableImport(store, parts, request.destination, {
        name: parts.length === 1 ? parts[0].choices.title : baseName(source.kind === 'file' ? source.name : pastedName),
        keep: keeping && source.kind === 'file' ? { name: source.name, bytes: source.bytes } : null,
        signal: control.signal, onProgress: setProgress,
      })
    } catch (e) {
      setProgress(null)
      setError(t('aiworkspace.import.unreadable', { reason: e instanceof Error ? e.message : String(e) }))
      return
    }
    abort.current = null
    onClose()
    if (!outcome.error) store.notify('info', t('aiworkspace.import.done', { name: outcome.name, summary: outcome.summary ?? '' }))
    request.finish?.(outcome)
    onDone(outcome)
  }

  if (step === 'ask') {
    return (
      <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
        <div className="aiws-dialog aiws-pdf-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.import.askTitle')} data-testid="aiws-table-ask" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
          <div className="aiws-dialog-head"><b>{t('aiworkspace.import.askTitle')}</b></div>
          <div className="aiws-muted">{t('aiworkspace.import.askText', { name: source.kind === 'file' ? source.name : pastedName })}</div>
          <div className="aiws-dialog-actions">
            <button type="button" className="is-primary" autoFocus data-testid="aiws-table-ask-table" onClick={() => setStep('wizard')}>{t('aiworkspace.import.askTable')}</button>
            {request.insertAsFile && <button type="button" data-testid="aiws-table-ask-file" onClick={() => { onClose(); request.insertAsFile?.() }}>{t('aiworkspace.import.askFile')}</button>}
            <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          </div>
        </div>
      </div>
    )
  }

  const title = source.kind === 'file' ? t('aiworkspace.import.title', { name: source.name }) : t('aiworkspace.import.titlePasted')
  const preview = part ? part.grid.rows.slice(part.choices.header ? 1 : 0, (part.choices.header ? 1 : 0) + PREVIEW_ROWS) : []
  const place = 'canvas' in request.destination ? t(request.destination.canvas.isFree ? 'aiworkspace.import.placeCanvas' : 'aiworkspace.import.placePage')
    : t('aiworkspace.import.placeFolder', { name: store.outline.get(request.destination.folderId)?.title ?? store.outline.get(request.destination.folderId)?.name ?? request.destination.folderId })
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget && !running) onClose() }}>
      <div className="aiws-dialog aiws-pdf-dialog aiws-table-dialog" role="dialog" aria-modal="true" aria-label={title} data-testid="aiws-table-dialog" onKeyDown={(event) => { if (event.key === 'Escape' && !running) onClose() }}>
        <div className="aiws-dialog-head">
          <b>{title}</b>
          {part && <span className="aiws-muted" data-testid="aiws-table-size">{t('aiworkspace.import.size', { rows: body, columns: part.choices.columns.length })}</span>}
        </div>
        {error ? (
          <>
            <div className="aiws-error" role="alert" data-testid="aiws-table-error">{error}</div>
            <div className="aiws-dialog-actions"><button type="button" autoFocus onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
          </>
        ) : !part || !parts ? (
          parts && parts.length === 0 ? <div className="aiws-muted">{t('aiworkspace.import.nothingChosen')}</div> : <div className="aiws-muted" data-testid="aiws-table-reading">{t('aiworkspace.import.reading')}</div>
        ) : (
          <>
            {parts.length > 1 && (
              <div className="aiws-table-parts" role="tablist">
                {parts.map((p, i) => <button key={p.key} type="button" role="tab" aria-selected={i === current} className={i === current ? 'is-active' : undefined} onClick={() => setCurrent(i)}>{p.label}</button>)}
              </div>
            )}
            <fieldset className="aiws-field aiws-table-top" disabled={running}>
              <label className="aiws-inline-form"><input type="checkbox" checked={part.choices.header} data-testid="aiws-table-header"
                onChange={(e) => setChoices({ ...part.choices, header: e.target.checked, columns: columnsFor(part.grid, e.target.checked, { locale, dateOrder, columnName: (letter) => t('aiworkspace.import.columnName', { letter }) }, part.choices.columns) })} />{t('aiworkspace.import.header')}</label>
              <label className="aiws-inline-form">{t('aiworkspace.import.tableTitle')}<input value={part.choices.title} maxLength={128} data-testid="aiws-table-title" onChange={(e) => setChoices({ ...part.choices, title: e.target.value })} /></label>
              <span className="aiws-muted">{place}</span>
            </fieldset>
            <div className="aiws-table-preview">
              <table>
                <thead>
                  <tr><th>{t('aiworkspace.import.include')}</th>{part.choices.columns.map((c, i) => <th key={i}><input type="checkbox" checked={c.include} disabled={running} aria-label={t('aiworkspace.import.includeAria', { name: c.name })} data-testid={`aiws-table-include-${i}`} onChange={(e) => setColumn(i, { ...c, include: e.target.checked })} /></th>)}</tr>
                  <tr><th>{t('aiworkspace.import.field')}</th>{part.choices.columns.map((c, i) => <th key={i}><input value={c.name} maxLength={128} disabled={running || !c.include} aria-label={t('aiworkspace.import.nameAria', { n: i + 1 })} data-testid={`aiws-table-name-${i}`} onChange={(e) => setColumn(i, { ...c, name: e.target.value })} /></th>)}</tr>
                  <tr><th>{t('aiworkspace.import.type')}</th>{part.choices.columns.map((c, i) => (
                    <th key={i}>
                      <select value={c.type} disabled={running || !c.include} aria-label={t('aiworkspace.import.typeAria', { name: c.name })} data-testid={`aiws-table-type-${i}`} onChange={(e) => setColumn(i, retype(part.grid, part.choices.header, i, c, e.target.value as ImportType, locale))}>
                        {IMPORT_TYPES.map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
                      </select>
                      <ColumnBadge choice={c} />
                      {c.type === 'currency' && c.include && <CurrencyChoices choice={c} index={i} disabled={running} onChange={(next) => setColumn(i, next)} />}
                    </th>
                  ))}</tr>
                </thead>
                <tbody>
                  {preview.map((row, r) => (
                    <tr key={r}><td className="aiws-muted">{r + 1}</td>{part.choices.columns.map((c, i) => <td key={i} className={c.include ? undefined : 'is-excluded'} title={row[i]?.text}>{row[i]?.text ?? ''}</td>)}</tr>
                  ))}
                </tbody>
              </table>
            </div>
            {checks.map((check, i) => check && check.invalid > 0 && (
              <div key={i} className="aiws-warning aiws-table-invalid" data-testid={`aiws-table-invalid-${i}`}>
                {t('aiworkspace.import.invalid', { name: part.choices.columns[i].name, type: FIELD_TYPE_LABEL[part.choices.columns[i].type], count: check.invalid, sample: check.samples[0] ?? '' })}
                <label className="aiws-inline-form"><input type="radio" name={`policy-${i}`} checked={part.choices.columns[i].invalid === 'unset'} data-testid={`aiws-table-policy-unset-${i}`} onChange={() => setColumn(i, { ...part.choices.columns[i], invalid: 'unset' })} />{t('aiworkspace.import.policyUnset')}</label>
                <label className="aiws-inline-form"><input type="radio" name={`policy-${i}`} checked={part.choices.columns[i].invalid === 'text'} data-testid={`aiws-table-policy-text-${i}`} onChange={() => setColumn(i, { ...part.choices.columns[i], invalid: 'text' })} />{t('aiworkspace.import.policyText')}</label>
              </div>
            ))}
            <details className="aiws-table-advanced" open={advanced} onToggle={(e) => setAdvanced((e.target as HTMLDetailsElement).open)}>
              <summary data-testid="aiws-table-advanced">{t('aiworkspace.import.advanced')}</summary>
              <fieldset className="aiws-field" disabled={running}>
                {(opened?.kind === 'csv' || opened?.kind === 'tsv') && (
                  <>
                    <label className="aiws-inline-form">{t('aiworkspace.import.encoding')}
                      <select value={encoding === 'auto' ? opened.read.encoding : encoding} data-testid="aiws-table-encoding" onChange={(e) => { setParts(null); setEncoding(e.target.value as Encoding) }}>
                        {ENCODINGS.map((enc) => <option key={enc} value={enc}>{enc}</option>)}
                      </select>
                    </label>
                    <label className="aiws-inline-form">{t('aiworkspace.import.delimiter')}
                      <select value={delimiter === 'auto' ? opened.read.delimiter : delimiter} data-testid="aiws-table-delimiter" onChange={(e) => { setParts(null); setDelimiter(e.target.value as Delimiter) }}>
                        {(Object.keys(DELIMITER_LABEL) as Delimiter[]).map((d) => <option key={d} value={d}>{t(DELIMITER_LABEL[d])}</option>)}
                      </select>
                    </label>
                  </>
                )}
                {opened?.kind === 'xlsx' && (
                  <>
                    <label className="aiws-inline-form">{t('aiworkspace.import.sheet')}
                      <select value={sheet} disabled={allSheets} data-testid="aiws-table-sheet" onChange={(e) => setSheet(e.target.value)}>
                        {opened.sheets.map((s) => <option key={s.name} value={s.name}>{s.hidden ? t('aiworkspace.import.hiddenSheet', { name: s.name }) : s.name}</option>)}
                      </select>
                    </label>
                    <label className="aiws-inline-form"><input type="checkbox" checked={allSheets} data-testid="aiws-table-all-sheets" onChange={(e) => setAllSheets(e.target.checked)} />{t('aiworkspace.import.allSheets')}</label>
                  </>
                )}
                {opened?.kind === 'sqlite' && (
                  <div className="aiws-table-tables" data-testid="aiws-table-tables">
                    <span>{t('aiworkspace.import.tables')}</span>
                    {opened.tables.map((tb) => (
                      <label key={tb.name} className="aiws-inline-form"><input type="checkbox" checked={tables?.includes(tb.name) ?? false} data-testid={`aiws-table-table-${tb.name}`}
                        onChange={(e) => setTables((x) => { const set = new Set(x ?? []); if (e.target.checked) set.add(tb.name); else set.delete(tb.name); return opened.tables.map((o) => o.name).filter((n) => set.has(n)) })} />
                        {t(tb.kind === 'view' ? 'aiworkspace.import.viewRows' : 'aiworkspace.import.tableRows', { name: tb.name, rows: tb.rows })}</label>
                    ))}
                    <label className="aiws-inline-form"><input type="checkbox" checked={views} data-testid="aiws-table-views" onChange={(e) => setViews(e.target.checked)} />{t('aiworkspace.import.views')}</label>
                  </div>
                )}
                <label className="aiws-inline-form">{t('aiworkspace.import.dateOrder')}
                  <select value={dateOrder} data-testid="aiws-table-date-order" onChange={(e) => setShaping({ dateOrder: e.target.value === 'dm' ? 'dm' : 'md' })}>
                    <option value="md">{t('aiworkspace.import.dateMD')}</option><option value="dm">{t('aiworkspace.import.dateDM')}</option>
                  </select>
                </label>
                <label className="aiws-inline-form">{t('aiworkspace.import.timezone')}
                  <select value={timezone} data-testid="aiws-table-timezone" onChange={(e) => setShaping({ timezone: e.target.value })}>
                    <option value="local">{t('aiworkspace.import.tzLocal', { zone: Intl.DateTimeFormat().resolvedOptions().timeZone })}</option><option value="UTC">UTC</option>
                  </select>
                </label>
                {source.kind === 'file' && (
                  <label className="aiws-inline-form" title={!keepAllowed ? t(live ? 'aiworkspace.import.keepTooLarge' : 'aiworkspace.import.keepOffline') : undefined}>
                    <input type="checkbox" checked={keeping} disabled={!keepAllowed} data-testid="aiws-table-keep" onChange={(e) => setKeep(e.target.checked)} />{t('aiworkspace.import.keep')}
                    {!keepAllowed && <span className="aiws-muted"> {t(live ? 'aiworkspace.import.keepTooLarge' : 'aiworkspace.import.keepOffline')}</span>}
                  </label>
                )}
              </fieldset>
            </details>
            {problem && <div className="aiws-warning" data-testid="aiws-table-problem">{problem}</div>}
            {running && progress && (
              <div className="aiws-pdf-progress" data-testid="aiws-table-progress">
                <progress max={Math.max(1, progress.total)} value={progress.done} />
                <span className="aiws-muted">{progress.stage === 'commit' ? t('aiworkspace.import.progress', { done: progress.done, total: progress.total }) : t(progress.stage === 'upload' ? 'aiworkspace.import.progressUpload' : 'aiworkspace.import.progressPlan')}</span>
              </div>
            )}
            <div className="aiws-dialog-actions">
              {running ? (
                <button type="button" data-testid="aiws-table-cancel" onClick={() => abort.current?.abort()}>{t('aiworkspace.common.cancel')}</button>
              ) : (
                <>
                  <button type="button" className="is-primary" autoFocus data-testid="aiws-table-start" disabled={problem !== null} onClick={() => { void start() }}>{t('aiworkspace.import.start', { count: rowsTotal })}</button>
                  <button type="button" data-testid="aiws-table-cancel" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}

/** "auto" while the column keeps the inferred type (with its currency or option count), "from the source" for declared ones. */
function ColumnBadge({ choice }: { choice: ColumnChoice }) {
  const { t } = useAiwsText()
  if (choice.type !== choice.guess.type) return null
  const base = choice.guess.fromSource ? t('aiworkspace.import.fromSource') : t('aiworkspace.import.auto')
  const extra = choice.type === 'currency' && choice.currency ? choice.currency : choice.options ? t('aiworkspace.import.optionCount', { count: choice.options.length }) : ''
  return <span className="aiws-muted aiws-table-badge">{extra ? `${base} · ${extra}` : base}</span>
}

/** A currency column: its default currency, and what `$` / `¥` mean when the column holds them (Q12). */
function CurrencyChoices({ choice, index, disabled, onChange }: { choice: ColumnChoice; index: number; disabled: boolean; onChange: (next: ColumnChoice) => void }) {
  const { t } = useAiwsText()
  const codes = (current: string | undefined) => (current && !ISO_CODES.includes(current) ? [current, ...ISO_CODES] : ISO_CODES)
  return (
    <span className="aiws-table-currency">
      <select value={choice.currency ?? 'USD'} disabled={disabled} aria-label={t('aiworkspace.import.currency')} title={t('aiworkspace.import.currency')} data-testid={`aiws-table-currency-${index}`} onChange={(e) => onChange({ ...choice, currency: e.target.value })}>
        {codes(choice.currency).map((code) => <option key={code} value={code}>{code}</option>)}
      </select>
      {choice.guess.dollar && (
        <label>{t('aiworkspace.import.dollarAs')}<select value={choice.dollar ?? 'USD'} disabled={disabled} data-testid={`aiws-table-dollar-${index}`} onChange={(e) => onChange({ ...choice, dollar: e.target.value })}>
          {['USD', 'AUD', 'CAD', 'HKD', 'SGD', 'TWD', 'NZD', 'MXN'].map((code) => <option key={code} value={code}>{code}</option>)}
        </select></label>
      )}
      {choice.guess.yen && (
        <label>{t('aiworkspace.import.yenAs')}<select value={choice.yen ?? 'CNY'} disabled={disabled} data-testid={`aiws-table-yen-${index}`} onChange={(e) => onChange({ ...choice, yen: e.target.value })}>
          {['CNY', 'JPY'].map((code) => <option key={code} value={code}>{code}</option>)}
        </select></label>
      )}
    </span>
  )
}
