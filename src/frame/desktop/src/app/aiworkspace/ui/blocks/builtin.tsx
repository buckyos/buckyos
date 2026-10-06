/* eslint-disable react-refresh/only-export-components -- a registry of definitions, not a component module */
/* Built-in Block definitions (phase two §10.1): table, rich text, record, asset, note (content
 * Blocks pairing with a data entity) and frame / shape (pure UI Blocks). Each declares what it
 * accepts, its static rendering, its editor (mounted only after activation), view-mode behaviour,
 * actions and inspector. They are registered like any extension would be. */

import { useCallback, useState, type CSSProperties, type ReactNode } from 'react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { AnnotationContent, Reference, RichTextContent } from '../../api/types'
import { StaticRichText } from '../../richtext/RichTextEditor'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { TableViewCell } from '../TableViewCell'
import { AssetCell, LockScope, NoteEditor, RecordCell, RichTextCell, TableEditor } from './editors'
import { cellOp, createOp, MAX_EMBED_DEPTH } from './ops'
import { blockRegistry, type BlockDefinition, type RenderContext } from './registry'

/** Rich text `object_embed` inside a Block: rendered read-only through the shell's renderer. */
function useEmbedRenderer(depth: number) {
  const ui = useWorkspaceUi()
  return useCallback((reference: Reference): ReactNode => {
    if (depth >= MAX_EMBED_DEPTH) return <div className="aiws-muted">嵌入层级过深，未展开（{reference.entity_id}）</div>
    return ui.renderCell(reference.entity_id, depth + 1)
  }, [ui, depth])
}

// ---- table

function TableStatic(context: RenderContext) {
  const ui = useWorkspaceUi()
  const source = context.source
  if (!source) return null
  return (
    <div className="aiws-block-static">
      <TableViewCell cellId={context.cell.entity_id} readOnly compact annotations={ui.annotations} onActivateAnnotation={ui.setActiveAnnotation} />
    </div>
  )
}

function TableEditorView(context: RenderContext) {
  return <TableEditor cellId={context.cell.entity_id} sourceId={context.source?.entity_id ?? ''} source={context.source} readOnly={context.readOnlyReason !== null} />
}

function TableSimplified(context: RenderContext) {
  return <div className="aiws-block-simplified">表格 · {context.source?.name ?? context.source?.title ?? context.payload.title ?? ''}</div>
}

const table: BlockDefinition = {
  type: 'table', version: 1, title: '表格视图', accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 640, h: 320 }, cost: { editor: true, html: false },
  Static: TableStatic, Simplified: TableSimplified, Editor: TableEditorView,
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'table', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.table-source', args.contentFolderId, args.dataOrderKey, {
      title_field_id: 'title', fields: [{ field_id: 'title', name: '标题', type: 'text', required: true }, { field_id: 'done', name: '完成', type: 'boolean' }],
    }, args.title || undefined),
    cellOp(args, 'table', args.dataId),
  ]),
}

// ---- rich text

function RichTextStatic(context: RenderContext) {
  const renderEmbed = useEmbedRenderer(0)
  if (!context.source) return null
  return <div className="aiws-block-static"><StaticRichText entityId={context.source.entity_id} renderEmbed={renderEmbed} /></div>
}

function RichTextEditorView(context: RenderContext) {
  const renderEmbed = useEmbedRenderer(0)
  if (!context.source) return null
  return <RichTextCell key={context.source.entity_id} entity={context.source} entityId={context.source.entity_id} renderEmbed={renderEmbed} />
}

function RichTextSimplified(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<RichTextContent>(id), [store, id])
  const read = useLoad<ReadOk<RichTextContent>>(load, version)
  const text = read.data ? firstText(read.data.content.content) : ''
  return <div className="aiws-block-simplified">{text || '富文本'}</div>
}

function firstText(node: { text?: string; content?: unknown[] } | undefined, limit = 80): string {
  if (!node) return ''
  if (node.text) return node.text.slice(0, limit)
  for (const child of (node.content ?? []) as { text?: string; content?: unknown[] }[]) {
    const t = firstText(child, limit)
    if (t) return t
  }
  return ''
}

const richtext: BlockDefinition = {
  type: 'richtext', version: 1, title: '富文本', accepts: ['buckyos.richtext'], allowNoSource: false,
  defaultSize: { w: 420, h: 220 }, cost: { editor: true, html: false },
  Static: RichTextStatic, Simplified: RichTextSimplified, Editor: RichTextEditorView,
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'richtext', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.richtext', args.contentFolderId, args.dataOrderKey, {
      content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') }, content: args.title ? [{ type: 'text', text: args.title }] : [] }] },
    }, args.title || undefined),
    cellOp(args, 'richtext', args.dataId),
  ]),
}

// ---- record

function RecordStatic(context: RenderContext) {
  if (!context.source) return null
  return <div className="aiws-block-static"><RecordCell entityId={context.source.entity_id} entity={context.source} readOnly /></div>
}
function RecordEditorView(context: RenderContext) {
  if (!context.source) return null
  const readOnly = context.readOnlyReason !== null
  return (
    <LockScope entity={readOnly ? undefined : context.source}>
      <RecordCell entityId={context.source.entity_id} entity={context.source} readOnly={readOnly} />
    </LockScope>
  )
}

const record: BlockDefinition = {
  type: 'record', version: 1, title: '记录', accepts: ['buckyos.record'], allowNoSource: false,
  defaultSize: { w: 360, h: 180 }, cost: { editor: false, html: false },
  Static: RecordStatic, Editor: RecordEditorView,
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'record', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.record', args.contentFolderId, args.dataOrderKey, {
      schema: { properties: [{ key: 'name', name: '名称', type: 'text' }, { key: 'value', name: '值', type: 'text' }] }, props: { name: args.title ?? '' },
    }, args.title || undefined),
    cellOp(args, 'record', args.dataId),
  ]),
}

// ---- asset (image)

function AssetStatic(context: RenderContext) {
  if (!context.source) return null
  const fit = context.payload.options?.fit === 'cover' ? 'cover' : 'contain'
  return <div className="aiws-block-static aiws-block-asset"><AssetCell entityId={context.source.entity_id} fit={fit} readOnly /></div>
}
function AssetEditorView(context: RenderContext) {
  if (!context.source) return null
  const fit = context.payload.options?.fit === 'cover' ? 'cover' : 'contain'
  return <AssetCell entityId={context.source.entity_id} fit={fit} readOnly={context.readOnlyReason !== null} />
}
/** View mode: a reading interaction (the explicit media preview, §8.3) without any write. */
function AssetView(context: RenderContext) {
  const [preview, setPreview] = useState(false)
  if (!context.source) return null
  const fit = context.payload.options?.fit === 'cover' ? 'cover' : 'contain'
  return (
    <div className="aiws-block-static aiws-block-asset" data-testid="aiws-asset-view">
      <AssetCell entityId={context.source.entity_id} fit={fit} readOnly />
      <button type="button" className="aiws-link aiws-block-view-action" data-testid="aiws-asset-preview" onClick={() => setPreview(true)}>预览</button>
      {preview && (
        <div className="aiws-media-preview" role="dialog" aria-label="媒体预览" onClick={() => setPreview(false)}>
          <div className="aiws-media-preview-body" onClick={(event) => event.stopPropagation()}>
            <AssetCell entityId={context.source.entity_id} fit="contain" readOnly />
            <button type="button" onClick={() => setPreview(false)}>关闭预览</button>
          </div>
        </div>
      )}
    </div>
  )
}

const asset: BlockDefinition = {
  type: 'asset', version: 1, title: '图片 / 附件', accepts: ['buckyos.asset-ref'], allowNoSource: false,
  defaultSize: { w: 320, h: 240 }, cost: { editor: false, html: false },
  Static: AssetStatic, Editor: AssetEditorView, View: AssetView,
  actions: [{ id: 'replace', label: '替换文件', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor() }],
  configFields: [{ key: 'fit', label: '填充方式', kind: 'select', options: [{ value: 'contain', label: '完整显示' }, { value: 'cover', label: '填满' }] }],
}

// ---- note (a free or attached annotation shown on the canvas)

function NoteStatic(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<AnnotationContent>(id), [store, id])
  const read = useLoad<ReadOk<AnnotationContent>>(load, version)
  const payload = read.data?.content.payload
  const style: CSSProperties = { background: typeof payload?.style?.color === 'string' ? payload.style.color : undefined }
  return (
    <div className="aiws-note aiws-block-static" style={style} data-testid={`aiws-note-${id}`}>
      <div className="aiws-note-body">{payload?.body ?? '…'}</div>
      {payload?.author && <div className="aiws-note-meta"><span>{payload.author}</span></div>}
    </div>
  )
}
function NoteEditorView(context: RenderContext) {
  if (!context.source) return null
  return <NoteEditor entityId={context.source.entity_id} entity={context.source} readOnly={context.readOnlyReason !== null} />
}

const note: BlockDefinition = {
  type: 'note', version: 1, title: '便签', accepts: ['buckyos.annotation'], allowNoSource: false,
  defaultSize: { w: 220, h: 160 }, cost: { editor: false, html: false },
  Static: NoteStatic, Editor: NoteEditorView,
  actions: [{ id: 'edit', label: '编辑便签', modes: ['edit'], needs: ['comment'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'note', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.annotation', args.contentFolderId, args.dataOrderKey, { kind: 'note', body: args.title ?? '', style: { color: '#fff2cc' } }),
    cellOp(args, 'note', args.dataId),
  ]),
}

// ---- frame & shape (pure UI Blocks, D3 / D10)

function FrameStatic(context: RenderContext) {
  const color = typeof context.payload.config?.color === 'string' ? context.payload.config.color : 'var(--cp-accent)'
  return (
    <div className="aiws-frame" style={{ borderColor: color }} data-testid={`aiws-frame-${context.cell.entity_id}`}>
      <div className="aiws-frame-title" style={{ background: color }}>{context.payload.title ?? '框'}</div>
    </div>
  )
}

const frame: BlockDefinition = {
  type: 'frame', version: 1, title: '框', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 600, h: 400 }, cost: { editor: false, html: false },
  Static: FrameStatic,
  configFields: [{ key: 'color', label: '颜色', kind: 'color' }],
  create: (args) => [cellOp(args, 'frame', undefined, { config: { color: (args.config?.color as string) ?? '#4f8df7' } })],
}

function ShapeStatic(context: RenderContext) {
  const config = context.payload.config ?? {}
  const kind = config.shape === 'ellipse' ? 'ellipse' : 'rect'
  const fill = typeof config.fill === 'string' ? config.fill : '#e8f0fe'
  const stroke = typeof config.stroke === 'string' ? config.stroke : '#4f8df7'
  return (
    <div className={`aiws-shape aiws-shape-${kind}`} style={{ background: fill, borderColor: stroke }} data-testid={`aiws-shape-${context.cell.entity_id}`}>
      {context.payload.title && <span className="aiws-shape-label">{context.payload.title}</span>}
    </div>
  )
}

const shape: BlockDefinition = {
  type: 'shape', version: 1, title: '形状', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 160, h: 120 }, cost: { editor: false, html: false },
  Static: ShapeStatic,
  configFields: [
    { key: 'shape', label: '形状', kind: 'select', options: [{ value: 'rect', label: '矩形' }, { value: 'ellipse', label: '椭圆' }] },
    { key: 'fill', label: '填充', kind: 'color' }, { key: 'stroke', label: '边框', kind: 'color' },
  ],
  create: (args) => [cellOp(args, 'shape', undefined, { config: { shape: (args.config?.shape as string) ?? 'rect', fill: '#e8f0fe', stroke: '#4f8df7' } })],
}

export const BUILTIN_BLOCKS: BlockDefinition[] = [table, richtext, record, asset, note, frame, shape]

let registered = false
/** Register the built-ins once (idempotent). Extensions register themselves the same way. */
export function registerBuiltinBlocks() {
  if (registered) return
  registered = true
  for (const def of BUILTIN_BLOCKS) blockRegistry.register(def)
}
