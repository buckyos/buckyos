/* One connector on the canvas (连接线实现方案 §9.4; 标准对象的交互改进 §6.3): an absolutely positioned frame in the
 * world layer, in the shared paint order, that takes no pointer events (lines are hit geometrically). It reads
 * the Cell for the look (`config` is not in the outline) and draws the routed geometry the layout gave it; a
 * running gesture repaints the frame through the registry instead of rendering. Editing a line means typing
 * its label in place; the label's size and colour go to the near toolbar. */

import { memo, useCallback, useLayoutEffect, useRef, useState } from 'react'
import { ALargeSmall } from 'lucide-react'
import type { CellPayload, Json, KeyedContent } from '../../../api/types'
import { useLoad, useStore, useVersion } from '../../../state/hooks'
import { useEditorToolbar } from '../../blocks/editorToolbar'
import type { CanvasMode, ToolbarItem } from '../../blocks/registry'
import type { ConnectorGeometry } from './geometry'
import { connectorStyle, DEFAULT_STYLE, LABEL_SIZES } from './model'
import { connectorMarkup, frameBox, labelFont, type PaintOptions } from './paint'
import { LINE_SIMPLIFIED_ZOOM, type LineRegistry } from './registry'

interface Props {
  id: string
  geom: ConnectorGeometry
  geomKey: string
  title: string | null
  zIndex: number
  hidden: boolean
  zoom: number
  mode: CanvasMode
  editing: boolean
  registry: LineRegistry
  onDone: () => void
}

function ConnectorFrameImpl({ id, geom, title, zIndex, hidden, zoom, mode, editing, registry, onDone }: Props) {
  const store = useStore()
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<KeyedContent<CellPayload>>(id), [store, id])
  const read = useLoad(load, version)
  const content = read.data?.entity_id === id ? read.data.content : undefined
  const style = content ? connectorStyle(content.payload.config) : DEFAULT_STYLE
  const options: PaintOptions = { zoom, simplified: zoom < LINE_SIMPLIFIED_ZOOM, showBroken: mode === 'edit', id, title }
  const box = frameBox(geom, style, options)
  const markup = connectorMarkup(geom, style, options)
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    if (ref.current) registry.set(id, { el: ref.current, style, options })
  })
  useLayoutEffect(() => () => registry.set(id, null), [registry, id])
  const font = labelFont(style)
  const labelStyle = { left: geom.label.point.x - box.x, top: geom.label.point.y - box.y, fontSize: font, ...(style.labelColor ? { color: style.labelColor } : {}), ...(style.labelFill ? { background: style.labelFill } : {}) }
  return (
    <div ref={ref} className={`aiws-frame-block is-connector${editing ? ' is-editing' : ''}`} data-block-id={id} data-testid={`aiws-canvas-block-${id}`} data-renderer="connector"
      data-route={geom.route} data-start={geom.start.state} data-end={geom.end.state}
      style={{ left: box.x, top: box.y, width: box.w, height: box.h, zIndex, display: hidden ? 'none' : undefined }}>
      <svg className="aiws-connector-svg" viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`} dangerouslySetInnerHTML={{ __html: markup }} />
      {editing
        ? <LabelEditor id={id} title={title} style={labelStyle} keyRevs={content?.key_revs ?? {}} config={content?.payload.config} onDone={onDone} />
        : title && !options.simplified && <div className="aiws-connector-label" data-testid={`aiws-connector-label-${id}`} style={labelStyle}>{title}</div>}
    </div>
  )
}

export const ConnectorFrame = memo(ConnectorFrameImpl, (a, b) => a.geomKey === b.geomKey && a.title === b.title && a.zIndex === b.zIndex && a.hidden === b.hidden
  && a.zoom === b.zoom && a.mode === b.mode && a.editing === b.editing && a.registry === b.registry && a.onDone === b.onDone)

/** The label typed where it is shown: Enter (without Shift) or leaving saves, Esc keeps the old one. */
function LabelEditor({ id, title, style, keyRevs, config, onDone }: { id: string; title: string | null; style: React.CSSProperties; keyRevs: Record<string, number>; config: Record<string, Json> | undefined; onDone: () => void }) {
  const store = useStore()
  const [text, setText] = useState(title ?? '')
  const closed = useRef(false)
  const done = (save: boolean) => {
    if (closed.current) return
    closed.current = true
    const next = text.trim().slice(0, 256)
    if (save && next !== (title ?? '')) {
      const expect = { rev: keyRevs.title ?? 0 }
      void store.submit({ editId: `key:${id}:title`, label: `连接线标签 → ${next || '（清除）'}`, operations: [next
        ? { op: 'entity.set_keys', entity_id: id, keys: [{ key: 'title', value: next, expect }] }
        : { op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'title', expect }] }] })
    }
    onDone()
  }
  const setConfig = (patch: Record<string, Json | null>, label: string) => {
    const next: Record<string, Json> = { ...(config ?? {}) }
    for (const [key, value] of Object.entries(patch)) { if (value === null) delete next[key]; else next[key] = value }
    void store.submit({ editId: `config:${id}`, label, operations: [{ op: 'entity.set_keys', entity_id: id, keys: [{ key: 'config', value: next, expect: { rev: keyRevs.config ?? 0 } }] }] })
  }
  const size = typeof config?.label_size === 'string' ? config.label_size : 'm'
  const tools: ToolbarItem[] = [
    { kind: 'menu', id: 'line-label-size', icon: ALargeSmall, label: '标签字号', value: size, items: LABEL_SIZES, onPick: (value) => setConfig({ label_size: value === 'm' ? null : value }, '标签字号') },
    { kind: 'color', id: 'line-label-color', label: '标签颜色', palette: 'ink', value: typeof config?.label_color === 'string' ? config.label_color : '', onPick: (value) => setConfig({ label_color: value || null }, '标签颜色') },
  ]
  useEditorToolbar(`connector-label:${id}`, tools)
  return (
    <textarea className="aiws-connector-label aiws-connector-label-edit" data-role="editor" data-testid={`aiws-connector-label-edit-${id}`} aria-label="连接线标签" autoFocus rows={1}
      value={text} placeholder="标签" style={style} onChange={(event) => setText(event.target.value)} onBlur={() => done(true)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); done(true) }
        else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) }
      }} />
  )
}
