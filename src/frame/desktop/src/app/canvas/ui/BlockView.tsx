/* ── Block chrome: position, header, selection, resize handles; delegates body to type views ── */

import clsx from 'clsx'
import { Frame, Lock, PencilLine } from 'lucide-react'
import { memo, useState } from 'react'
import type { CanvasBlock } from '../domain/types'
import { generatedStatus } from '../domain/selectors'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { Badge } from './primitives'
import { STATUS_META, TYPE_ICON, TYPE_LABEL } from './meta'
import { ChartBlockView } from './blocks/ChartBlock'
import { GroupHeader } from './blocks/GroupBlock'
import { ImageBlockView } from './blocks/ImageBlock'
import { MetricBlockView } from './blocks/MetricBlock'
import { TableBlockView } from './blocks/TableBlock'
import { TextBlockView } from './blocks/TextBlock'
import { VideoBlockView } from './blocks/VideoBlock'
import { WishBlockView } from './blocks/WishBlock'

const HANDLES: Array<{ k: string; style: React.CSSProperties; cursor: string }> = [
  { k: 'nw', style: { left: -5, top: -5 }, cursor: 'nwse-resize' },
  { k: 'n', style: { left: 'calc(50% - 5px)', top: -5 }, cursor: 'ns-resize' },
  { k: 'ne', style: { right: -5, top: -5 }, cursor: 'nesw-resize' },
  { k: 'e', style: { right: -5, top: 'calc(50% - 5px)' }, cursor: 'ew-resize' },
  { k: 'se', style: { right: -5, bottom: -5 }, cursor: 'nwse-resize' },
  { k: 's', style: { left: 'calc(50% - 5px)', bottom: -5 }, cursor: 'ns-resize' },
  { k: 'sw', style: { left: -5, bottom: -5 }, cursor: 'nesw-resize' },
  { k: 'w', style: { left: -5, top: 'calc(50% - 5px)' }, cursor: 'ew-resize' },
]

function Handles({ id }: { id: string }) {
  return (
    <>
      {HANDLES.map((h) => (
        <div key={h.k} data-resize={h.k} data-block-id={id} className="aic-handle" style={{ ...h.style, cursor: h.cursor }} />
      ))}
    </>
  )
}

function Title({ block }: { block: CanvasBlock }) {
  const { store } = useCanvasEditor()
  const [edit, setEdit] = useState<string | null>(null)
  if (edit !== null) {
    return (
      <input
        autoFocus
        data-no-drag
        className="min-w-0 flex-1 bg-transparent text-xs font-semibold outline-none"
        value={edit}
        onChange={(e) => setEdit(e.target.value)}
        onBlur={() => {
          if (edit.trim() && edit !== block.title) store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { title: edit.trim() }, userEdit: true })
          setEdit(null)
        }}
        onKeyDown={(e) => {
          e.stopPropagation()
          if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur()
          if (e.key === 'Escape') setEdit(null)
        }}
      />
    )
  }
  return (
    <span className="min-w-0 flex-1 truncate" onDoubleClick={(e) => { e.stopPropagation(); if (!block.locked) setEdit(block.title ?? '') }} title="双击重命名">
      {block.title ?? TYPE_LABEL[block.type]}
    </span>
  )
}

export const BlockView = memo(function BlockView({ block, selected, editing, highlight, presenting }: { block: CanvasBlock; selected: boolean; editing: boolean; highlight: boolean; presenting: boolean }) {
  const { doc } = useStoreState()
  const { rect } = block
  const base: React.CSSProperties = { left: rect.x, top: rect.y, width: rect.width, height: rect.height, zIndex: block.zIndex + (selected ? 1000 : 0) }
  const showHandles = selected && !block.locked && !presenting

  if (block.type === 'frame') {
    const color = block.content.color ?? 'var(--cp-accent)'
    return (
      <div className="aic-block" data-block-id={block.id} style={{ ...base, pointerEvents: 'none' }}>
        <div className={clsx('h-full w-full rounded-xl border-2', selected ? 'border-solid' : 'border-dashed')} style={{ borderColor: selected ? 'var(--cp-accent)' : `color-mix(in srgb, ${color} 60%, var(--cp-border-opaque))`, background: `color-mix(in srgb, ${color} 6%, transparent)` }}>
          <div className="aic-frame-title" data-drag-handle data-block-id={block.id} style={{ background: `color-mix(in srgb, ${color} 22%, var(--cp-surface-opaque))`, color: 'var(--cp-text)' }}>
            <span className="inline-flex items-center gap-1 [&>svg]:size-[12px]">
              <Frame />
              <Title block={block} />
              {block.locked ? <Lock /> : null}
            </span>
          </div>
        </div>
        {showHandles ? <div className="pointer-events-auto"><Handles id={block.id} /></div> : null}
      </div>
    )
  }

  if (block.type === 'group') {
    const active = block.generated && !block.generated.detached
    return (
      <div className={clsx('aic-block', highlight && 'aic-highlight')} data-block-id={block.id} style={{ ...base, pointerEvents: 'none' }}>
        <div className={clsx('h-full w-full rounded-[10px] border', selected ? 'border-solid border-[color:var(--cp-accent)]' : 'border-dashed')} style={{ borderColor: selected ? undefined : active ? 'color-mix(in srgb, var(--aic-ai) 45%, transparent)' : 'var(--cp-border-opaque)', background: active ? 'color-mix(in srgb, var(--aic-ai) 3%, transparent)' : 'transparent' }}>
          <div data-drag-handle data-block-id={block.id}>
            <GroupHeader block={block} />
          </div>
        </div>
      </div>
    )
  }

  const status = block.generated && !block.generated.detached ? generatedStatus(doc, block) : null
  const sm = status && status !== 'never_run' ? STATUS_META[status] : null
  const inGroup = Boolean(block.generated && !block.generated.detached && Object.values(doc.blocks).some((g) => g.type === 'group' && g.content.childBlockIds.includes(block.id)))

  return (
    <div className={clsx('aic-block', highlight && 'aic-highlight')} data-block-id={block.id} style={base}>
      <div className={clsx('aic-card', selected && 'is-selected', block.generated && !block.generated.detached && 'is-generated')}>
        <div className="aic-head" data-drag-handle>
          <span className={clsx('inline-flex [&>svg]:size-[13px]', block.type === 'wish' ? 'text-[color:var(--aic-ai)]' : 'text-[color:var(--cp-muted)]')}>{TYPE_ICON[block.type]}</span>
          <Title block={block} />
          {block.generated && !block.generated.detached ? (
            <>
              {!inGroup ? <Badge tone="ai">AI</Badge> : null}
              {sm && !inGroup ? <Badge tone={sm.tone}>{sm.glyph} {sm.label}</Badge> : null}
              {block.generated.userModified ? <span title="已手工修改" className="text-[color:var(--cp-muted)] [&>svg]:size-[12px]"><PencilLine /></span> : null}
            </>
          ) : null}
          {block.locked ? <Lock className="size-[12px] text-[color:var(--cp-muted)]" /> : null}
        </div>
        <div className="aic-body">
          {block.type === 'text' ? <TextBlockView block={block} editing={editing} /> : null}
          {block.type === 'table' ? <TableBlockView block={block} selected={selected} /> : null}
          {block.type === 'wish' ? <WishBlockView block={block} /> : null}
          {block.type === 'metric' ? <MetricBlockView block={block} /> : null}
          {block.type === 'chart' ? <ChartBlockView block={block} /> : null}
          {block.type === 'image' ? <ImageBlockView block={block} /> : null}
          {block.type === 'video' ? <VideoBlockView block={block} /> : null}
          {block.type === 'interactive' ? (
            <div className="flex h-full items-center justify-center p-3 text-center text-xs text-[color:var(--cp-muted)]">自定义交互块将在 P0.5 以沙箱方式渲染，当前版本仅保留数据。</div>
          ) : null}
        </div>
      </div>
      {showHandles ? <Handles id={block.id} /> : null}
    </div>
  )
})
