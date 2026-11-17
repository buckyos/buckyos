/* A flow canvas printed (打印 TODO §3.2, T2, T9, Q18): the page's Blocks in reading order, laid out at the width of the
 * page's content area, measured here in Chromium, broken into pages by `paginate` (@print/paginate.ts) and drawn page by
 * page. Legal breaks: between Blocks, between the paragraphs and lines of a rich text (never right after a heading),
 * between the rows of a table (whose header repeats on every page it continues on). A heading — the page title, a
 * group's title, a text ending in one — stays with what follows. A group prints as its title and its Blocks; a PDF Frame
 * on the page is not content. A part of a Block on a page is the Block drawn whole behind a window onto that part; a
 * table draws only the rows of its part. */

import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { contentArea, PX_PER_MM, type PageContext } from '@print/page.ts'
import type { PrintJob } from '@print/package.ts'
import { paginate, type FlowItem, type FlowPage, type Fragment } from '@print/paginate.ts'
import type { EntityEnvelope } from '../../api/types'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { BlockHost } from '../../ui/blocks/BlockHost'
import { blockRegistry } from '../../ui/blocks/registry'
import { flowChildren } from '../../ui/canvas/flowOps'
import { controlRegistry } from '../../ui/controls/registry'
import { isPdfFrame, type PdfFrameInfo } from '../../ui/controls/pdfframe/api'
import { PrintFragmentContext, type PrintFragment } from './context'
import { PrintPage } from './PrintPage'
import { settled } from './ready'

/** Space between two Blocks (dropped where a page ends). */
const GAP = 10
/** A group's Blocks are indented this much per level, with the group's line on their left. */
const INDENT = 14
const EPS = 0.5

interface Entry {
  id: string
  kind: 'title' | 'group' | 'cell' | 'control'
  entity: EntityEnvelope | null
  depth: number
  text: string
}

/** The page in reading order, groups opened up into their title and their Blocks. */
function entriesOf(store: WorkspaceStore, surface: EntityEnvelope, untitled: string): Entry[] {
  const out: Entry[] = [{ id: `title:${surface.entity_id}`, kind: 'title', entity: surface, depth: 0, text: surface.title ?? surface.name ?? untitled }]
  const walk = (containerId: string, depth: number) => {
    for (const entity of flowChildren(store, containerId)) {
      if (isPdfFrame(entity)) continue
      if (entity.type_id === 'buckyos.cell') {
        const source = entity.source_id ? store.outline.get(entity.source_id) : undefined
        out.push({ id: entity.entity_id, kind: 'cell', entity, depth, text: entity.title ?? (source ? source.title ?? source.name ?? '' : '') })
      } else if (entity.kind === 'group' || entity.kind === 'composite') {
        const title = entity.title ?? entity.name ?? (entity.kind === 'composite' ? store.outline.get(entity.def_id ?? '')?.title : null) ?? ''
        if (title) out.push({ id: `group:${entity.entity_id}`, kind: 'group', entity, depth, text: title })
        walk(entity.entity_id, depth + 1)
      } else {
        out.push({ id: entity.entity_id, kind: 'control', entity, depth, text: entity.title || controlRegistry.get(entity.kind)?.title || entity.kind || '' })
      }
    }
  }
  walk(surface.entity_id, 0)
  return out
}

function indentStyle(depth: number): CSSProperties | undefined {
  return depth > 0 ? { marginLeft: (depth - 1) * INDENT, paddingLeft: INDENT, borderLeft: '2px solid color-mix(in srgb, var(--cp-accent) 30%, var(--cp-border))' } : undefined
}

/** One entry as it prints: the same classes as the flow page on screen (its look), without its tools. */
function EntryView({ entry, showTitle = true }: { entry: Entry; showTitle?: boolean }) {
  const indent = indentStyle(entry.depth)
  if (entry.kind === 'title') return <h1 className="aiws-page-title">{entry.text}</h1>
  if (entry.kind === 'group') return <div className="aiws-fgroup-title" style={indent}>{entry.text}</div>
  const entity = entry.entity!
  if (entry.kind === 'control') {
    const Flow = controlRegistry.get(entity.kind)?.Flow
    return (
      <section className="aiws-cell aiws-fblock aiws-fcontrol" style={indent} data-kind={entity.kind ?? undefined}>
        {showTitle && entry.text && <div className="aiws-fblock-title">{entry.text}</div>}
        {Flow && <Flow containerId={entity.entity_id} mode="print" />}
      </section>
    )
  }
  const titled = !blockRegistry.get(entity.view_type ?? '')?.traits?.flowUntitled
  return (
    <section className="aiws-cell aiws-fblock" style={indent} data-view={entity.view_type ?? undefined} data-cell-id={entity.entity_id}>
      {showTitle && titled && entry.text && <div className="aiws-fblock-title">{entry.text}</div>}
      <div className="aiws-cell-body"><BlockHost cellId={entity.entity_id} mode="print" view="canvas" /></div>
    </section>
  )
}

const HEADING = /^H[1-6]$/
const BLOCK_DISPLAY = new Set(['block', 'list-item', 'flex', 'grid', 'table', 'flow-root'])

/** The bottoms of the lines of `el` (a line is the rectangles that overlap vertically). */
function lineBottoms(el: Element, top: number, out: number[]) {
  const range = document.createRange()
  range.selectNodeContents(el)
  const rects = [...range.getClientRects()].filter((r) => r.height > 0).sort((a, b) => a.top - b.top)
  let line: { top: number; bottom: number } | null = null
  for (const r of rects) {
    if (line && r.top < line.bottom - 1) { line.bottom = Math.max(line.bottom, r.bottom); continue }
    if (line) out.push(line.bottom - top)
    line = { top: r.top, bottom: r.bottom }
  }
  if (line) out.push(line.bottom - top)
}

/** Breaks inside a rich text: after each block (never after a heading), inside a block between its lines, between the
 * rows of a table; an embedded Block is not broken. */
function textBreaks(container: Element, top: number, out: number[]) {
  for (const child of container.children) {
    if (HEADING.test(child.tagName)) continue
    const bottom = child.getBoundingClientRect().bottom - top
    if (child.tagName === 'TABLE') {
      for (const row of child.querySelectorAll('tr')) out.push(row.getBoundingClientRect().bottom - top)
    } else if (!child.classList.contains('aiws-embed')) {
      if ([...child.children].some((c) => BLOCK_DISPLAY.has(getComputedStyle(c).display))) textBreaks(child, top, out)
      else lineBottoms(child, top, out)
    }
    out.push(bottom)
  }
}

interface Measured {
  item: FlowItem
  /** A table: the bottom of each row (from the entry's top). */
  rows: number[] | null
}

function measureEntry(el: HTMLElement, entry: Entry): Measured {
  const box = el.getBoundingClientRect()
  const top = box.top
  const base: FlowItem = { id: entry.id, height: box.height, gapAfter: GAP }
  if (entry.kind === 'title' || entry.kind === 'group') return { item: { ...base, keepWithNext: true }, rows: null }
  if (entry.kind !== 'cell') return { item: base, rows: null }
  const table = el.querySelector('table.aiws-print-table')
  if (table) {
    const rows = [...table.querySelectorAll('tr[data-print-row]')].map((row) => row.getBoundingClientRect().bottom - top)
    const head = table.querySelector('thead')?.getBoundingClientRect().height ?? 0
    return { item: { ...base, breaks: rows, header: head }, rows }
  }
  const prose = el.querySelector('.aiws-prose-static')
  if (!prose) return { item: base, rows: null }
  const breaks: number[] = []
  textBreaks(prose, top, breaks)
  const last = prose.lastElementChild
  const sorted = [...new Set(breaks.map((b) => Math.round(b * 10) / 10))].sort((a, b) => a - b)
  return { item: { ...base, breaks: sorted, keepWithNext: Boolean(last && HEADING.test(last.tagName)) }, rows: null }
}

function FragmentView({ fragment, entry, rows, width }: { fragment: Fragment; entry: Entry; rows: number[] | null; width: number }) {
  const scale = fragment.scale ?? 1
  const header = fragment.header ?? 0
  const height = (header + fragment.to - fragment.from) * scale
  let context: PrintFragment = { width, rows: null, measure: false }
  let offset = fragment.from
  if (rows) {
    // a table draws its rows of this part (and its header): the window starts at the top of what it draws
    const first = rows.findIndex((b) => b > fragment.from + EPS)
    let end = rows.length
    for (let i = 0; i < rows.length; i++) if (rows[i] > fragment.to + EPS) { end = i; break }
    context = { width, rows: [Math.max(0, first), Math.max(first, end)], measure: false }
    offset = 0
  }
  return (
    <div className="aiws-print-frag" style={{ top: fragment.top, height }} data-item={entry.id} data-from={Math.round(fragment.from)} data-to={Math.round(fragment.to)}>
      <div className="aiws-print-frag-body" style={{ width, transform: `translateY(${-offset * scale}px) scale(${scale})` }}>
        <PrintFragmentContext.Provider value={context}>
          <div className="aiws-print-item"><EntryView entry={entry} showTitle={fragment.from <= EPS} /></div>
        </PrintFragmentContext.Provider>
      </div>
    </div>
  )
}

interface Layout { pages: FlowPage[]; rows: Map<number, number[] | null> }

export function FlowPages({ job, frame, background, measureDeadline, deadline, onReady }: {
  job: PrintJob; frame: PdfFrameInfo | null; background: string; measureDeadline: number; deadline: number; onReady: () => void
}) {
  const store = useStore()
  const { t } = useAiwsText()
  const areaMm = contentArea(job.paper, 'flow', frame?.geometry)
  const width = areaMm.w * PX_PER_MM
  const height = areaMm.h * PX_PER_MM
  const surface = store.outline.get(job.surface_id)
  const entries = useMemo(() => (surface ? entriesOf(store, surface, t('aiworkspace.canvas.defaultName')) : []), [store, surface, t])
  const measureRef = useRef<HTMLDivElement>(null)
  const pagesRef = useRef<HTMLDivElement>(null)
  const [layout, setLayout] = useState<Layout | null>(null)

  // first pass: every entry at the content width, measured once it is all drawn
  useEffect(() => {
    const root = measureRef.current
    if (!root) return
    let live = true
    void settled(root, measureDeadline).then(() => {
      if (!live) return
      const measured = entries.map((entry) => {
        const el = root.querySelector<HTMLElement>(`[data-print-item="${CSS.escape(entry.id)}"]`)
        return el ? measureEntry(el, entry) : { item: { id: entry.id, height: 0 }, rows: null }
      })
      const result = paginate(measured.map((m) => m.item), height)
      setLayout({ pages: result.pages, rows: new Map(measured.map((m, i) => [i, m.rows])) })
    })
    return () => { live = false }
  }, [entries, height, measureDeadline])

  // second pass: the pages, done once they are drawn
  useEffect(() => {
    const root = pagesRef.current
    if (!layout || !root) return
    let live = true
    void settled(root, deadline).then(() => { if (live) onReady() })
    return () => { live = false }
  }, [layout, deadline, onReady])

  if (!layout) {
    return (
      <div ref={measureRef} className="aiws-print-measure aiws-flow" style={{ width }} aria-hidden="true">
        <PrintFragmentContext.Provider value={{ width, rows: null, measure: true }}>
          {entries.map((entry) => <div key={entry.id} className="aiws-print-item" data-print-item={entry.id}><EntryView entry={entry} /></div>)}
        </PrintFragmentContext.Provider>
      </div>
    )
  }
  const area = { left: areaMm.x * PX_PER_MM, top: areaMm.y * PX_PER_MM, width, height }
  return (
    <div ref={pagesRef}>
      {layout.pages.map((page, i) => {
        const context: PageContext = { page: i + 1, pages: layout.pages.length, title: job.title, date: job.date }
        return (
          <PrintPage key={i} paper={job.paper} frame={frame} context={context} background={background}>
            <div className="aiws-print-flow aiws-flow" style={area}>
              {page.fragments.map((fragment, j) => <FragmentView key={`${fragment.id}:${j}`} fragment={fragment} entry={entries[fragment.item]} rows={layout.rows.get(fragment.item) ?? null} width={width} />)}
            </div>
          </PrintPage>
        )
      })}
    </div>
  )
}
