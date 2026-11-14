/* The text format bar (UI 框架 PRD §4.4, §6.2, §7.5): while a text editor has the focus on a phone, its format tools — the
 * ones it hands its host (`useEditorToolbar` / `setEditorToolbar`) — stand in the context zone, over the soft keyboard,
 * scrolling sideways when they do not fit. A text editor on a phone's page (or a phone's full-screen editor) is wrapped
 * in a scope: its tools go there instead of being drawn over the text. The bar follows the scope that has the focus;
 * focus moving into the bar or one of its sheets keeps it. */

import { useEffect, useState, useSyncExternalStore, type ReactNode } from 'react'
import { Emitter } from '../../state/emitter'
import { useAiwsText } from '../../useAiwsText'
import { EditorToolbarContext, ToolbarSink } from '../blocks/editorToolbar'
import { ToolItem } from '../canvas/tools'
import { ContextZone } from '../framework/ContextZone'
import { insideSheet } from '../shell/popover'
import './phone.css'

const sinks = new Map<string, ToolbarSink>()
const scopes = new Emitter()
let scopesVersion = 0
const NO_SINK = { subscribe: () => () => undefined, snapshot: () => 0 }

/** The editor inside hands its tools to the format bar (on a phone; elsewhere `enabled` is false and nothing changes). */
export function FormatScope({ id, enabled = true, children }: { id: string; enabled?: boolean; children: ReactNode }) {
  const [sink] = useState(() => new ToolbarSink())
  useEffect(() => {
    if (!enabled) return
    sinks.set(id, sink)
    scopesVersion += 1
    scopes.emit()
    return () => {
      if (sinks.get(id) === sink) sinks.delete(id)
      scopesVersion += 1
      scopes.emit()
    }
  }, [id, sink, enabled])
  if (!enabled) return <>{children}</>
  return (
    <EditorToolbarContext.Provider value={sink}>
      <div className="aiws-format-scope" data-format-scope={id}>{children}</div>
    </EditorToolbarContext.Provider>
  )
}

const scopeOf = (target: EventTarget | null) => (target instanceof Element ? target.closest('[data-format-scope]')?.getAttribute('data-format-scope') ?? null : null)
const keeps = (target: EventTarget | null) => target instanceof Element && (target.closest('.aiws-format-bar') !== null || insideSheet(target))

/** The bar of the scope that has the focus. */
export function FormatBar() {
  const { t } = useAiwsText()
  const [active, setActive] = useState<string | null>(() => scopeOf(document.activeElement))
  useSyncExternalStore(scopes.subscribe, () => scopesVersion)
  const sink = active ? sinks.get(active) : undefined
  useSyncExternalStore(sink?.subscribe ?? NO_SINK.subscribe, sink?.snapshot ?? NO_SINK.snapshot)
  useEffect(() => {
    let timer = 0
    const onIn = (event: FocusEvent) => { if (!keeps(event.target)) setActive(scopeOf(event.target)) }
    // the focus left for nothing (a tap on the page outside any field): the bar goes with it
    const onOut = () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        const now = document.activeElement
        if (!now || now === document.body) setActive(null)
      }, 0)
    }
    document.addEventListener('focusin', onIn)
    document.addEventListener('focusout', onOut)
    return () => { window.clearTimeout(timer); document.removeEventListener('focusin', onIn); document.removeEventListener('focusout', onOut) }
  }, [])
  const items = sink?.items() ?? []
  if (!active || items.length === 0) return null
  return (
    <ContextZone testId="aiws-format-zone">
      <div className="aiws-panel aiws-context-bar aiws-format-bar" role="toolbar" aria-label={t('aiworkspace.phone.formatBar')} data-testid="aiws-format-bar" data-scope={active}
        onPointerDown={(event) => event.stopPropagation()}>
        {items.map((item) => <ToolItem key={item.id} item={item} />)}
      </div>
    </ContextZone>
  )
}
