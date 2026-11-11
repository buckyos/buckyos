/* Spreadsheets as a grid: xlsx / xlsm and csv / tsv read from the original bytes, and the xlsx a Pipeline converts
 * xls / et / ods / numbers to. The workbook is parsed in a Worker (sheet.worker.ts); rows and columns are
 * virtualized, the column letters and row numbers stay in view, merged cells span their range. */

import { useVirtualizer } from '@tanstack/react-virtual'
import { columnLetter } from '@table/grid.ts'
import clsx from 'clsx'
import { FileWarning } from 'lucide-react'
import { useEffect, useImperativeHandle, useMemo, useRef, useState, type ReactNode, type Ref, type RefObject } from 'react'
import { readAll } from './io'
import { DIRECT_SIZE_BUDGET, SHEET_ROW_LIMIT } from './mediaTypes'
import { usePinchZoom, type TranslateFn, type ZoomHandle, type ZoomViewInfo } from './pinchZoom'
import { RendererNotice } from './RendererNotice'
import type { SheetData, SheetErrorCode, SheetKind, SheetRequest, SheetResponse } from './sheetData'
import { isAbortError, PreviewError, type PreviewReadRef } from './types'

const ROW_HEIGHT = 26
const HEADER_HEIGHT = 24
const GUTTER_WIDTH = 46
const TOOLBAR_CLEARANCE = 44
const MIN_SCALE = 0.5
const MAX_SCALE = 3
const NUMERIC = /^[-+(]?[\d,]*\.?\d+(?:[eE][-+]?\d+)?[%)]?$/

type SheetCall = (request: SheetRequest extends infer R ? (R extends unknown ? Omit<R, 'id'> : never) : never) => Promise<SheetResponse>

export function SheetRenderer({
  ref,
  readRef,
  kind,
  name,
  compact,
  textView,
  onRendered,
  onFailure,
  onDownload,
  onOpenWith,
  onViewChange,
  t,
}: {
  ref: Ref<ZoomHandle>
  readRef: PreviewReadRef
  kind: SheetKind
  name: string
  compact: boolean
  /** The same content as plain text (csv / tsv): offered next to the grid. */
  textView?: ReactNode
  onRendered: (media?: undefined, opts?: { degraded?: boolean }) => void
  onFailure: (failure: 'corrupted' | 'network') => void
  onDownload: () => void
  onOpenWith?: () => void
  onViewChange: (view: ZoomViewInfo | null) => void
  t: TranslateFn
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const callRef = useRef<SheetCall | null>(null)
  const loaded = useRef(new Map<number, SheetData>())
  const [sheets, setSheets] = useState<string[] | null>(null)
  const [active, setActive] = useState(0)
  const [shown, setShown] = useState<{ index: number; sheet: SheetData } | null>(null)
  const [notice, setNotice] = useState<Exclude<SheetErrorCode, 'corrupted'> | null>(null)
  const [scale, setScale] = useState(1)
  const [asText, setAsText] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    const worker = new Worker(new URL('./sheet.worker.ts', import.meta.url), { type: 'module' })
    const pending = new Map<number, (response: SheetResponse) => void>()
    let nextId = 1
    const fail = (code: SheetErrorCode) => {
      if (controller.signal.aborted) return
      if (code === 'corrupted') {
        onFailure('corrupted')
        return
      }
      setNotice(code)
      onRendered(undefined, { degraded: true })
    }
    worker.onmessage = (event: MessageEvent<SheetResponse>) => {
      pending.get(event.data.id)?.(event.data)
      pending.delete(event.data.id)
    }
    worker.onerror = () => fail('corrupted')
    const call: SheetCall = (request) =>
      new Promise((resolve) => {
        const id = nextId
        nextId += 1
        pending.set(id, resolve)
        worker.postMessage({ ...request, id }, request.type === 'open' ? [request.bytes] : [])
      })
    callRef.current = call
    const cache = loaded.current
    void (async () => {
      try {
        const blob = await readAll(readRef, DIRECT_SIZE_BUDGET.sheet, controller.signal)
        const bytes = await blob.arrayBuffer()
        const opened = await call({ type: 'open', bytes, kind, name, rowLimit: SHEET_ROW_LIMIT })
        if (controller.signal.aborted) return
        if ('error' in opened) fail(opened.error)
        else if ('sheets' in opened) setSheets(opened.sheets)
      } catch (err) {
        if (controller.signal.aborted || isAbortError(err)) return
        if (err instanceof PreviewError && err.code === 'TOO_LARGE') fail('too-large')
        else onFailure('network')
      }
    })()
    return () => {
      controller.abort()
      worker.terminate()
      callRef.current = null
      cache.clear()
    }
  }, [kind, name, onFailure, onRendered, readRef])

  useEffect(() => {
    if (!sheets) return
    const known = loaded.current.get(active)
    let cancelled = false
    void (async () => {
      let sheet = known
      if (!sheet) {
        const response = await callRef.current?.({ type: 'read', index: active })
        if (cancelled || !response) return
        if ('error' in response) {
          if (response.error === 'corrupted') onFailure('corrupted')
          else {
            setNotice(response.error)
            onRendered(undefined, { degraded: true })
          }
          return
        }
        if (!('sheet' in response)) return
        sheet = response.sheet
        loaded.current.set(active, sheet)
      }
      if (cancelled) return
      setShown({ index: active, sheet })
      scrollRef.current?.scrollTo({ left: 0, top: 0 })
      onRendered()
    })()
    return () => {
      cancelled = true
    }
  }, [active, onFailure, onRendered, sheets])

  const gridShown = shown !== null && !asText
  const zoomTo = usePinchZoom(scrollRef, contentRef, scale, MIN_SCALE, MAX_SCALE, setScale, gridShown)
  useImperativeHandle(ref, () => ({ zoomBy: (factor) => zoomTo(scale * factor), fit: () => zoomTo(1) }), [scale, zoomTo])
  useEffect(() => {
    onViewChange(gridShown ? { scale, mode: scale === 1 ? 'fit' : 'custom', rotation: 0 } : null)
  }, [gridShown, onViewChange, scale])

  if (notice) {
    return (
      <RendererNotice
        testId={`content-preview-sheet-${notice}`}
        icon={notice === 'too-large' ? <FileWarning size={28} /> : undefined}
        title={notice === 'encrypted' ? t('preview.sheet.locked', 'This spreadsheet is password protected') : t('preview.sheet.tooLarge', 'This spreadsheet is too large to show here')}
        body={t('preview.sheet.noticeBody', 'Download the file or open it with another application.')}
        onDownload={onDownload}
        onOpenWith={onOpenWith}
        t={t}
      />
    )
  }
  if (!sheets || !shown) return null

  const showTabs = sheets.length > 1 || Boolean(textView)
  return (
    <div className="absolute inset-0 flex flex-col bg-[color:var(--cp-surface-opaque)]" data-testid="content-preview-sheet" data-sheet-count={sheets.length} data-rows={shown.sheet.rows.length} data-scale={scale}>
      <div className="relative min-h-0 flex-1">
        {asText ? (
          textView
        ) : (
          <div
            ref={scrollRef}
            className="desktop-scrollbar absolute inset-x-0 bottom-0 overflow-auto"
            style={{ top: compact ? 0 : TOOLBAR_CLEARANCE, touchAction: 'pan-x pan-y' }}
            data-testid="content-preview-sheet-scroll"
          >
            <SheetGrid key={`${shown.index}:${scale}`} sheet={shown.sheet} scale={scale} scrollRef={scrollRef} contentRef={contentRef} />
          </div>
        )}
      </div>
      {shown.sheet.truncated && !asText ? (
        <div
          className="border-t border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_18%,var(--cp-surface-opaque))] px-4 py-1.5 text-center text-[11px] text-[color:var(--cp-text)]"
          data-testid="content-preview-sheet-truncated"
        >
          {t('preview.sheet.truncated', 'Showing the first {{rows}} rows — download the file to see all of it', { rows: shown.sheet.rows.length })}
        </div>
      ) : null}
      {showTabs ? (
        <div className="flex h-9 shrink-0 items-stretch border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2-opaque)] text-[12px]">
          <div className="flex min-w-0 flex-1 items-stretch overflow-x-auto" role="tablist">
            {asText
              ? null
              : sheets.map((sheet, index) => (
                  <button
                    key={index}
                    type="button"
                    role="tab"
                    aria-selected={index === active}
                    data-testid="content-preview-sheet-tab"
                    className={clsx(
                      'max-w-[200px] shrink-0 truncate border-r border-[color:var(--cp-border)] px-3',
                      index === active ? 'bg-[color:var(--cp-surface-opaque)] font-semibold text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]',
                    )}
                    onClick={() => setActive(index)}
                  >
                    {sheet}
                  </button>
                ))}
          </div>
          {textView ? (
            <button
              type="button"
              className="shrink-0 border-l border-[color:var(--cp-border)] px-3 font-medium text-[color:var(--cp-text)] hover:text-[color:var(--cp-accent)]"
              data-testid="content-preview-sheet-text-toggle"
              onClick={() => setAsText((value) => !value)}
            >
              {asText ? t('preview.sheet.asTable', 'Table') : t('preview.sheet.asText', 'Plain text')}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function SheetGrid({
  sheet,
  scale,
  scrollRef,
  contentRef,
}: {
  sheet: SheetData
  scale: number
  scrollRef: RefObject<HTMLDivElement | null>
  contentRef: RefObject<HTMLDivElement | null>
}) {
  const rowHeight = ROW_HEIGHT * scale
  const headerHeight = HEADER_HEIGHT * scale
  const gutter = GUTTER_WIDTH * scale
  const { lefts, width } = useMemo(() => {
    const out: number[] = []
    let at = 0
    for (const w of sheet.widths) {
      out.push(at)
      at += w * scale
    }
    return { lefts: out, width: at }
  }, [scale, sheet.widths])
  const covered = useMemo(() => {
    const flags = new Uint8Array(sheet.rows.length * sheet.columns)
    for (const [r0, c0, r1, c1] of sheet.merges) {
      for (let r = r0; r <= r1; r += 1) flags.fill(1, r * sheet.columns + c0, r * sheet.columns + c1 + 1)
    }
    return flags
  }, [sheet])

  const rowVirtualizer = useVirtualizer({ count: sheet.rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => rowHeight, overscan: 10, scrollMargin: headerHeight })
  const columnVirtualizer = useVirtualizer({ horizontal: true, count: sheet.columns, getScrollElement: () => scrollRef.current, estimateSize: (index) => sheet.widths[index] * scale, overscan: 3, scrollMargin: gutter })
  const rows = rowVirtualizer.getVirtualItems()
  const columns = columnVirtualizer.getVirtualItems()
  const firstRow = rows[0]?.index ?? 0
  const lastRow = rows[rows.length - 1]?.index ?? -1
  const firstColumn = columns[0]?.index ?? 0
  const lastColumn = columns[columns.length - 1]?.index ?? -1
  const merges = sheet.merges.filter(([r0, c0, r1, c1]) => r0 <= lastRow && r1 >= firstRow && c0 <= lastColumn && c1 >= firstColumn)

  const fontSize = 12 * scale
  const cell = 'absolute top-0 h-full overflow-hidden whitespace-nowrap border-b border-r border-[color:var(--cp-border)] px-1.5 text-[color:var(--cp-text)]'
  const label = 'flex items-center justify-center border-b border-r border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2-opaque)] text-[color:var(--cp-muted)]'
  return (
    <div ref={contentRef} className="relative select-text" style={{ width: gutter + width, height: headerHeight + sheet.rows.length * rowHeight, fontSize, lineHeight: `${rowHeight - 1}px` }}>
      <div className="sticky top-0 z-20" style={{ height: headerHeight, lineHeight: `${headerHeight - 1}px`, fontSize: 11 * scale }}>
        <div className={clsx(label, 'sticky left-0 z-30 h-full')} style={{ width: gutter }} />
        {columns.map((column) => (
          <div key={column.key} className={clsx(label, 'absolute top-0 h-full')} style={{ left: column.start, width: column.size }}>
            {columnLetter(column.index)}
          </div>
        ))}
      </div>
      <div className="relative" style={{ height: sheet.rows.length * rowHeight }}>
        {rows.map((row) => (
          <div key={row.key} className="absolute left-0 w-full" style={{ top: row.start - headerHeight, height: rowHeight }} data-testid="content-preview-sheet-row" data-row-index={row.index}>
            <div className={clsx(label, 'sticky left-0 z-10 h-full')} style={{ width: gutter, fontSize: 11 * scale }}>
              {row.index + 1}
            </div>
            {columns.map((column) => {
              if (covered[row.index * sheet.columns + column.index]) return null
              const text = sheet.rows[row.index][column.index]
              return (
                <div key={column.key} className={clsx(cell, text && NUMERIC.test(text) && 'text-right')} style={{ left: column.start, width: column.size }}>
                  {text}
                </div>
              )
            })}
          </div>
        ))}
        {merges.map(([r0, c0, r1, c1]) => (
          <div
            key={`${r0}:${c0}`}
            className="absolute z-[5] flex items-center overflow-hidden border-b border-r border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-1.5 text-[color:var(--cp-text)]"
            style={{ left: gutter + lefts[c0], top: r0 * rowHeight, width: lefts[c1] + sheet.widths[c1] * scale - lefts[c0], height: (r1 - r0 + 1) * rowHeight, lineHeight: 1.3 }}
            data-testid="content-preview-sheet-merge"
          >
            {sheet.rows[r0][c0]}
          </div>
        ))}
      </div>
    </div>
  )
}
