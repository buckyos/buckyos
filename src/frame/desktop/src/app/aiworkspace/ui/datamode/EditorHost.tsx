/* The editing area's one mounted editor (UI 框架 PRD §5.3): only the current tab's editor is mounted. Its scroll
 * positions are kept per tab in this window and put back when the tab is current again (the content may arrive
 * later: tried for a while); what was typed survives the switch through the editors' own commit-on-leave and drafts.
 * A change inside a preview tab fixes it. */

import { useEffect, useLayoutEffect, useRef, type ReactNode } from 'react'
import { useStore } from '../../state/hooks'
import { editNames, type DataTab, type DataTabs } from './tabs'

interface ScrollMark { path: number[]; top: number; left: number }
/** Scroll positions per tab, for this window. */
const scrolls = new Map<string, ScrollMark[]>()

function pathTo(root: Element, el: Element): number[] | null {
  const out: number[] = []
  let cur: Element | null = el
  while (cur && cur !== root) {
    const parent: Element | null = cur.parentElement
    if (!parent) return null
    out.unshift(Array.prototype.indexOf.call(parent.children, cur))
    cur = parent
  }
  return cur === root ? out : null
}

function follow(root: Element, path: number[]): Element | null {
  let cur: Element | null = root
  for (const i of path) { cur = cur?.children[i] ?? null; if (!cur) return null }
  return cur
}

export function EditorHost({ tab, tabs, children }: { tab: DataTab; tabs: DataTabs; children: ReactNode }) {
  const store = useStore()
  const ref = useRef<HTMLDivElement>(null)
  const id = tab.id
  // remember where each scrolling element of this tab is
  useEffect(() => {
    const root = ref.current
    if (!root) return
    const onScroll = (event: Event) => {
      const el = event.target instanceof Element ? event.target : null
      if (!el || !root.contains(el)) return
      const path = pathTo(root, el)
      if (!path) return
      const marks = (scrolls.get(id) ?? []).filter((m) => m.path.join('.') !== path.join('.'))
      if (el.scrollTop > 0 || el.scrollLeft > 0) marks.push({ path, top: el.scrollTop, left: el.scrollLeft })
      scrolls.set(id, marks)
    }
    root.addEventListener('scroll', onScroll, true)
    return () => root.removeEventListener('scroll', onScroll, true)
  }, [id])
  // put them back once the content is tall enough (it loads after mounting), for at most three seconds
  useLayoutEffect(() => {
    const root = ref.current
    const marks = scrolls.get(id)
    if (!root || !marks?.length) return
    let left = [...marks]
    let frame = 0
    const until = Date.now() + 3000
    const tryRestore = () => {
      left = left.filter((m) => {
        const el = follow(root, m.path)
        if (!el) return true
        el.scrollTop = m.top
        el.scrollLeft = m.left
        return Math.abs(el.scrollTop - m.top) > 2 || Math.abs(el.scrollLeft - m.left) > 2
      })
      if (left.length > 0 && Date.now() < until) frame = window.requestAnimationFrame(tryRestore)
    }
    tryRestore()
    // the person scrolls: stop putting things back
    const stop = () => { left = [] }
    root.addEventListener('wheel', stop, { passive: true })
    root.addEventListener('touchmove', stop, { passive: true })
    return () => { window.cancelAnimationFrame(frame); root.removeEventListener('wheel', stop); root.removeEventListener('touchmove', stop) }
  }, [id])
  // a change in a preview tab fixes it: typing in it, or an edit of its object made while it is open
  useEffect(() => {
    if (!tab.preview) return
    const root = ref.current
    const since = tab.at / 1000
    // after the event is handled: a render in the middle of it would put a controlled input's old value back
    let timer: number | null = null
    const pinSoon = () => { if (timer === null) timer = window.setTimeout(() => { timer = null; tabs.pin(id) }, 0) }
    root?.addEventListener('input', pinSoon, true)
    const off = store.edits.subscribe(() => {
      for (const entry of store.edits.snapshot().values()) if (entry.at >= since && editNames(entry.id, id)) { pinSoon(); return }
    })
    return () => { root?.removeEventListener('input', pinSoon, true); off(); if (timer !== null) window.clearTimeout(timer) }
  }, [tab.preview, tab.at, id, tabs, store])
  return <div ref={ref} className="aiws-dm-editor" data-testid="aiws-dm-editor" data-tab-id={id}>{children}</div>
}
