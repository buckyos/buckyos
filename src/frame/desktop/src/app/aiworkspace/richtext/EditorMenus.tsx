/* Floating parts of the rich text editor (流式页交互改进 §3): the `/` command menu at the caret, the format
 * bubble over a text selection, and the Markdown source editor of the "notepad" input mode (§4). Nothing of
 * this sits permanently over the text: the page reads like a document until the user asks for a tool. */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import type { LucideIcon } from 'lucide-react'
import type { EditorView } from 'prosemirror-view'
import type { ToolbarItem } from '../ui/blocks/registry'
import { InlineTools } from '../ui/canvas/tools'
import { applyEdit, buildSource, changedRange, offsetOfBlock, rebase, segmentAt, sourceText, type MdCodec, type MdSource } from './markdownSource'
import type { SlashKey, SlashState } from './slash'

// ---- position helpers: the floating parts are drawn in the application root (absolute), so a canvas Block or a
// scrolling page never clips them; they are placed again when anything scrolls.

/** Re-render on any scroll or resize (the caret moved on screen). */
function useFollowScroll() {
  const [, setTick] = useState(0)
  useEffect(() => {
    const bump = () => setTick((tick) => tick + 1)
    window.addEventListener('scroll', bump, true)
    window.addEventListener('resize', bump)
    return () => { window.removeEventListener('scroll', bump, true); window.removeEventListener('resize', bump) }
  }, [])
}

function caretBox(view: EditorView, pos: number, container: HTMLElement): { left: number; top: number; bottom: number } | null {
  try {
    const at = view.coordsAtPos(pos)
    const box = container.getBoundingClientRect()
    return { left: at.left - box.left, top: at.top - box.top, bottom: at.bottom - box.top }
  } catch {
    return null
  }
}

// ---- `/` menu

export interface SlashItem {
  id: string
  label: string
  icon: LucideIcon
  /** Words the query matches besides the label (pinyin initials, English, Markdown syntax). */
  keywords: string
  group: string
  hint?: string
  run: () => void
}

function matches(item: SlashItem, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return `${item.label} ${item.keywords}`.toLowerCase().split(/\s+/).some((word) => word.startsWith(q)) || item.label.toLowerCase().includes(q)
}

export function SlashMenu({ view, state, items, container, bindKeys }: {
  view: EditorView
  state: SlashState
  items: SlashItem[]
  container: HTMLElement
  /** Hands the menu's key handler to the editor plugin (↑ ↓ Enter while the caret stays in the text). */
  bindKeys: (handler: ((key: SlashKey) => boolean) | null) => void
}) {
  const shown = items.filter((item) => matches(item, state.query))
  const [active, setActive] = useState(0)
  const [query, setQuery] = useState(state.query)
  if (query !== state.query) { setQuery(state.query); setActive(0) }
  const index = Math.min(active, Math.max(0, shown.length - 1))
  const listRef = useRef<HTMLDivElement>(null)
  useFollowScroll()
  useEffect(() => {
    bindKeys((key) => {
      if (key === 'down') { setActive((value) => (shown.length ? (Math.min(value, shown.length - 1) + 1) % shown.length : 0)); return true }
      if (key === 'up') { setActive((value) => (shown.length ? (Math.min(value, shown.length - 1) - 1 + shown.length) % shown.length : 0)); return true }
      if (key === 'enter') {
        const item = shown[index]
        if (!item) return false
        item.run()
        return true
      }
      return false
    })
    return () => bindKeys(null)
  })
  useLayoutEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [index])
  const box = caretBox(view, state.from, container)
  if (!box) return null
  let group = ''
  const rows: ReactNode[] = []
  shown.forEach((item, i) => {
    if (item.group !== group) {
      group = item.group
      rows.push(<div key={`g-${group}`} className="aiws-slash-group">{group}</div>)
    }
    rows.push(
      <button key={item.id} type="button" role="option" aria-selected={i === index} className="aiws-slash-item" data-testid={`aiws-slash-${item.id}`}
        onMouseDown={(event) => event.preventDefault()} onMouseEnter={() => setActive(i)} onClick={() => item.run()}>
        <span className="aiws-slash-icon"><item.icon size={16} aria-hidden="true" /></span>
        <span className="aiws-slash-label">{item.label}</span>
        {item.hint && <span className="aiws-slash-hint">{item.hint}</span>}
      </button>,
    )
  })
  // near the bottom of the window the menu opens above the line
  const above = box.bottom + 330 > container.clientHeight && box.top > 330
  const left = Math.max(4, Math.min(box.left - 4, container.clientWidth - 264))
  return (
    <div ref={listRef} className="aiws-slash" role="listbox" aria-label="插入或转换" data-testid="aiws-slash-menu" style={above ? { left, bottom: container.clientHeight - box.top + 4 } : { left, top: box.bottom + 4 }}
      onPointerDown={(event) => event.stopPropagation()}>
      {rows.length > 0 ? rows : <div className="aiws-slash-empty">没有匹配的命令</div>}
    </div>
  )
}

// ---- format bubble over a selection

export function FormatBubble({ view, items, container, version }: { view: EditorView; items: ToolbarItem[]; container: HTMLElement; version: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  useFollowScroll()
  useLayoutEffect(() => { if (ref.current) setWidth(ref.current.offsetWidth) }, [version, items.length])
  const { from, to } = view.state.selection
  const start = caretBox(view, from, container)
  const end = caretBox(view, to, container)
  if (!start || !end || items.length === 0) return null
  const top = Math.min(start.top, end.top) - 44
  const center = start.top === end.top ? (start.left + end.left) / 2 : start.left + 80
  const left = Math.max(4, Math.min(center - width / 2, container.clientWidth - width - 4))
  return (
    <div ref={ref} className="aiws-bubble" data-testid="aiws-format-bubble" style={{ left, top: top < 4 ? end.bottom + 8 : top }} onPointerDown={(event) => event.stopPropagation()}>
      <InlineTools items={items} label="文字格式" />
    </div>
  )
}

// ---- Markdown source ("notepad") mode

/** A plain text area holding the Markdown of the document. What is typed reaches the rich text a moment later
 * (or on leaving); changes from elsewhere come in without rewriting the text being typed. */
export function MarkdownSourceEditor({ view, codec, editable, startBlockId, onExit, subscribeDoc }: {
  view: EditorView
  codec: MdCodec
  editable: boolean
  /** The block the caret was in when the mode was entered. */
  startBlockId: string | null
  /** Leave source mode; the editor's caret goes to the block holding the text caret. */
  onExit: (blockId: string | null) => void
  subscribeDoc: (listener: () => void) => () => void
}) {
  const [initial] = useState(() => {
    const source = buildSource(view.state.doc, codec)
    return { source, text: sourceText(source), caret: offsetOfBlock(source, startBlockId) }
  })
  const areaRef = useRef<HTMLTextAreaElement>(null)
  const [live] = useState(() => new Map<'s', { source: MdSource; text: string; timer: number; composing: boolean; ownDoc: unknown; remote: boolean; failed: boolean }>())
  const [failed, setFailed] = useState(false)
  // what may change while the text area stays (the lock taken, names of linked objects): read when used
  const [options] = useState(() => new Map<'o', { codec: MdCodec; editable: boolean }>())
  useEffect(() => { options.set('o', { codec, editable }) })

  useEffect(() => {
    const area = areaRef.current
    if (!area) return
    live.set('s', { source: initial.source, text: initial.text, timer: 0, composing: false, ownDoc: view.state.doc, remote: false, failed: false })
    const fit = () => { area.style.height = 'auto'; area.style.height = `${area.scrollHeight + 2}px` }
    area.value = initial.text
    fit()
    area.focus()
    area.setSelectionRange(initial.caret, initial.caret)
    const s = () => live.get('s')
    const rebaseNow = () => {
      const state = s()
      if (!state) return
      if (state.composing) { state.remote = true; return }
      const next = rebase(view.state.doc, state.source, options.get('o')?.codec ?? codec)
      const text = sourceText(next)
      state.source = next
      state.ownDoc = view.state.doc
      if (text !== area.value) {
        const range = changedRange(area.value, text)
        area.setRangeText(text.slice(range.from, range.newTo), range.from, range.oldTo, 'preserve')
        fit()
      }
      state.text = text
    }
    const sync = () => {
      const state = s()
      if (!state || view.isDestroyed) return
      if (state.timer) { window.clearTimeout(state.timer); state.timer = 0 }
      if (state.composing) return
      const next = area.value
      const current = options.get('o')
      if (next !== state.text && current?.editable) {
        const applied = applyEdit(view, state.source, state.text, next, current.codec)
        if (applied) {
          state.source = applied
          state.text = next
          state.ownDoc = view.state.doc
          if (state.failed) { state.failed = false; setFailed(false) }
        } else if (!state.failed) { state.failed = true; setFailed(true) }
      }
      if (state.remote) { state.remote = false; rebaseNow() }
    }
    const schedule = () => {
      const state = s()
      if (!state) return
      if (state.timer) window.clearTimeout(state.timer)
      state.timer = window.setTimeout(sync, 350)
    }
    const onInput = () => { fit(); schedule() }
    const onCompositionStart = () => { const state = s(); if (state) state.composing = true }
    const onCompositionEnd = () => { const state = s(); if (state) state.composing = false; schedule() }
    const onBlur = () => sync()
    area.addEventListener('input', onInput)
    area.addEventListener('compositionstart', onCompositionStart)
    area.addEventListener('compositionend', onCompositionEnd)
    area.addEventListener('blur', onBlur)
    const off = subscribeDoc(() => {
      const state = s()
      if (!state || view.state.doc === state.ownDoc) return
      // typed text not handed over yet: it goes first, the outside change is merged right after
      if (state.timer || area.value !== state.text) { state.remote = true; return }
      rebaseNow()
    })
    const onFit = () => fit()
    window.addEventListener('resize', onFit)
    return () => {
      off()
      window.removeEventListener('resize', onFit)
      area.removeEventListener('input', onInput)
      area.removeEventListener('compositionstart', onCompositionStart)
      area.removeEventListener('compositionend', onCompositionEnd)
      area.removeEventListener('blur', onBlur)
      const state = s()
      if (state) state.composing = false
      sync()
      live.delete('s')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- codec and editable are read through `options`
  }, [view, initial, live, options, subscribeDoc])

  const leave = () => {
    const area = areaRef.current
    // the cleanup hands over what is still pending
    const state = live.get('s')
    onExit(area && state ? segmentAt(state.source, area.selectionStart)?.ids[0] ?? null : null)
  }

  return (
    <div className="aiws-md-source-wrap">
      <textarea
        ref={areaRef}
        className="aiws-md-source"
        data-testid="aiws-md-source"
        aria-label="Markdown 源码"
        spellCheck={false}
        readOnly={!editable}
        placeholder="直接输入 Markdown：# 标题、- 列表、**加粗**、> 引用、``` 代码…"
        onKeyDown={(event) => {
          const mod = event.ctrlKey || event.metaKey
          if (mod && event.key === '/') { event.preventDefault(); leave(); return }
          if (event.key === 'Tab' && !mod && !event.altKey && editable) {
            event.preventDefault()
            // keeps the browser's own undo for the text area
            if (!document.execCommand('insertText', false, '\t')) event.currentTarget.setRangeText('\t', event.currentTarget.selectionStart, event.currentTarget.selectionEnd, 'end')
          }
        }}
      />
      {failed && <div className="aiws-warning" role="status">这段 Markdown 暂时无法转换，继续输入后会再试。</div>}
    </div>
  )
}
