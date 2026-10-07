/* eslint-disable react-refresh/only-export-components -- a registry of definitions, not a component module */
/* Built-in Block definitions (phase two §10.1): table, rich text, record, asset, note (content
 * Blocks pairing with a data entity) and frame / shape (pure UI Blocks). Each declares what it
 * accepts, its static rendering, its editor (mounted only after activation), view-mode behaviour,
 * actions and inspector, and — 标准对象的交互改进 §4–§5 — its own look (the host frame draws none),
 * what hovering it offers and the type section of the near toolbar. They are registered like any
 * extension would be. */

import { useCallback, useEffect, useState, type CSSProperties, type ReactNode } from 'react'
import { ALargeSmall, Baseline, Circle, Columns3, Eye, Image as ImageIcon, ListFilter, ListTree, Pencil, Plus, Replace, Reply, Scan, Square, Table, Type } from 'lucide-react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { AnnotationContent, Json, Reference, RichTextContent } from '../../api/types'
import { StaticRichText } from '../../richtext/RichTextEditor'
import { useEdit, useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { TableViewCell } from '../TableViewCell'
import { sourceLabel } from './affordances'
import { consumeIntent, requestIntent, useIntent, useOnFreeCanvas } from './editorToolbar'
import { AssetCell, LockScope, NoteEditor, NoteReplies, RecordCell, RichTextCell, TableEditor } from './editors'
import { cellOp, createOp, MAX_EMBED_DEPTH, replaceAsset, setCellConfig, setCellKey } from './ops'
import { blockRegistry, type BlockDefinition, type RenderContext, type ToolbarItem } from './registry'

const TEXT_SIZES = [{ value: 's', label: '小' }, { value: 'm', label: '中' }, { value: 'l', label: '大' }, { value: 'xl', label: '特大' }]

/** Open the Block's editor with `intent` waiting for it (a panel to open, text typed into it). */
function openWith(context: RenderContext, intent: string) {
  requestIntent(context.cell.entity_id, intent)
  context.activateEditor()
}

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

const canAppend = (context: RenderContext) => context.readOnlyReason === null && Boolean(context.source?.capabilities.some((c) => c === 'append' || c === 'update'))

const table: BlockDefinition = {
  type: 'table', version: 1, title: '表格视图', accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 640, h: 320 }, cost: { editor: true, html: false },
  Static: TableStatic, Simplified: TableSimplified, Editor: TableEditorView,
  catalog: { group: 'data', description: '行与字段组成的数据表；新建时带“标题”“完成”两个字段，可在表格中继续加字段和记录。', needs: 'none', standard: true },
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => [
    ...sourceLabel(context, Table),
    ...(canAppend(context) ? [{ id: 'add-row', kind: 'button' as const, at: 'bottom-out' as const, icon: Plus, label: '新增一行', run: (c: RenderContext) => openWith(c, 'add') }] : []),
  ],
  // while editing, the table's own tools come from its editor (they also show which panel is open)
  toolbar: (context) => context.editorActive ? [] : [
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: '筛选 / 排序', run: () => openWith(context, 'filter') },
    ...(context.readOnlyReason === null && context.source?.capabilities.includes('structure') ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: '字段', run: () => openWith(context, 'fields') }] : []),
    ...(canAppend(context) ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: '新增记录', run: () => openWith(context, 'add') }] : []),
  ],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'table', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.table-source', args.contentFolderId, args.dataOrderKey, {
      title_field_id: 'title', fields: [{ field_id: 'title', name: '标题', type: 'text', required: true }, { field_id: 'done', name: '完成', type: 'boolean' }],
    }, args.title || undefined),
    cellOp(args, 'table', args.dataId),
  ]),
}

// ---- rich text

/** The Block-wide text size and colour of a rich text (Cell `config.text_size` / `config.text_color`). */
function textStyle(context: RenderContext): { size: string; style: CSSProperties } {
  const config = context.payload.config ?? {}
  return { size: typeof config.text_size === 'string' ? config.text_size : 'm', style: typeof config.text_color === 'string' ? { color: config.text_color } : {} }
}

function RichTextStatic(context: RenderContext) {
  const renderEmbed = useEmbedRenderer(context.depth)
  if (!context.source) return null
  const { size, style } = textStyle(context)
  return <div className="aiws-block-static" data-text-size={size} style={style}><StaticRichText entityId={context.source.entity_id} renderEmbed={renderEmbed} /></div>
}

function RichTextEditorView(context: RenderContext) {
  const renderEmbed = useEmbedRenderer(context.depth)
  const onCanvas = useOnFreeCanvas()
  if (!context.source) return null
  const { size, style } = textStyle(context)
  return (
    <div className="aiws-block-static" data-text-size={size} style={style}>
      <RichTextCell key={context.source.entity_id} entity={context.source} entityId={context.source.entity_id} renderEmbed={renderEmbed} intentKey={onCanvas ? context.cell.entity_id : undefined} />
    </div>
  )
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

/** Text size and colour of the whole Block: Cell configuration (§5.2), in the toolbar while selected or editing. */
function textTools(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  const { size } = textStyle(context)
  const color = context.payload.config?.text_color
  // a Block's own configuration: the Cell, not the data, is written
  const reason = context.capabilities.includes('update') && context.mode === 'edit' ? false : '没有修改此 Block 的权限'
  return [
    { kind: 'menu', id: 'text-size', icon: ALargeSmall, label: '字号', value: size, items: TEXT_SIZES, disabled: reason, onPick: (value) => setCellConfig(context, store, { text_size: value === 'm' ? null : value }, `字号 → ${TEXT_SIZES.find((t) => t.value === value)?.label ?? value}`) },
    { kind: 'color', id: 'text-color', label: '文字颜色', palette: 'ink', value: typeof color === 'string' ? color : '', disabled: reason, onPick: (value) => setCellConfig(context, store, { text_color: value || null }, '文字颜色') },
  ]
}

const richtext: BlockDefinition = {
  type: 'richtext', version: 1, title: '富文本', accepts: ['buckyos.richtext'], allowNoSource: false,
  defaultSize: { w: 420, h: 220 }, cost: { editor: true, html: false },
  Static: RichTextStatic, Simplified: RichTextSimplified, Editor: RichTextEditorView,
  catalog: { group: 'text', description: '段落、标题、列表等格式化文本，支持多人同时编辑和按文字范围批注。', needs: 'none', editAfterInsert: true, standard: true },
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => sourceLabel(context, Type),
  toolbar: textTools,
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
  catalog: { group: 'data', description: '一组带名称的属性（名称、值），适合记录项目信息、参数等结构化内容。', needs: 'none', editAfterInsert: true },
  actions: [{ id: 'edit', label: '编辑内容', modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => sourceLabel(context, ListTree),
  create: (args) => (args.existingSourceId ? [cellOp(args, 'record', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.record', args.contentFolderId, args.dataOrderKey, {
      schema: { properties: [{ key: 'name', name: '名称', type: 'text' }, { key: 'value', name: '值', type: 'text' }] }, props: { name: args.title ?? '' },
    }, args.title || undefined),
    cellOp(args, 'record', args.dataId),
  ]),
}

// ---- asset (image): the picture itself; no editor — replace, fit and preview are toolbar actions (§4.4)

function fitOf(context: RenderContext): 'contain' | 'cover' {
  return (context.payload.config?.fit ?? context.payload.options?.fit) === 'cover' ? 'cover' : 'contain'
}

function AssetPreview({ entityId, onClose }: { entityId: string; onClose: () => void }) {
  return (
    <div className="aiws-media-preview" role="dialog" aria-label="媒体预览" data-testid="aiws-media-preview" onClick={onClose}>
      <div className="aiws-media-preview-body" onClick={(event) => event.stopPropagation()}>
        <AssetCell entityId={entityId} fit="contain" readOnly />
        <button type="button" onClick={onClose}>关闭预览</button>
      </div>
    </div>
  )
}

/** "Preview" from the toolbar: the Block opens its preview (also in edit mode, where there is no editor). */
function usePreviewIntent(cellId: string): [boolean, (open: boolean) => void] {
  const [preview, setPreview] = useState(false)
  const intent = useIntent(cellId)
  const [seen, setSeen] = useState(0)
  if (intent?.value === 'preview' && intent.seq !== seen) { setSeen(intent.seq); setPreview(true) }
  useEffect(() => { if (intent?.value === 'preview') consumeIntent(cellId, intent) }, [intent, cellId])
  return [preview, setPreview]
}

function AssetStatic(context: RenderContext) {
  const [preview, setPreview] = usePreviewIntent(context.cell.entity_id)
  // a flow page has no near toolbar: the file facts and "replace" stay under the picture there
  const bare = useOnFreeCanvas()
  if (!context.source) return null
  return (
    <div className="aiws-block-static aiws-block-asset">
      <AssetCell entityId={context.source.entity_id} fit={fitOf(context)} readOnly={bare || context.readOnlyReason !== null || context.mode !== 'edit'} bare={bare} />
      {preview && <AssetPreview entityId={context.source.entity_id} onClose={() => setPreview(false)} />}
    </div>
  )
}
/** View mode: a reading interaction (the explicit media preview, §8.3) without any write. */
function AssetView(context: RenderContext) {
  const [preview, setPreview] = usePreviewIntent(context.cell.entity_id)
  const bare = useOnFreeCanvas()
  if (!context.source) return null
  return (
    <div className="aiws-block-static aiws-block-asset" data-testid="aiws-asset-view">
      <AssetCell entityId={context.source.entity_id} fit={fitOf(context)} readOnly bare={bare} />
      <button type="button" className="aiws-link aiws-block-view-action" data-testid="aiws-asset-preview" onClick={() => setPreview(true)}>预览</button>
      {preview && <AssetPreview entityId={context.source.entity_id} onClose={() => setPreview(false)} />}
    </div>
  )
}

/** Pick a file and make it the asset's content. */
function pickReplacement(context: RenderContext, store: WorkspaceStore) {
  const sourceId = context.source?.entity_id
  if (!sourceId) return
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = 'image/*'
  input.onchange = () => {
    const file = input.files?.[0]
    if (file) void replaceAsset(store, sourceId, file).then((problem) => { if (problem) store.notify('error', problem) })
  }
  input.click()
}

const asset: BlockDefinition = {
  type: 'asset', version: 1, title: '图片 / 附件', accepts: ['buckyos.asset-ref'], allowNoSource: false,
  defaultSize: { w: 320, h: 240 }, cost: { editor: false, html: false },
  Static: AssetStatic, View: AssetView,
  resize: { aspect: 'locked' },
  catalog: { group: 'data', description: '上传图片或文件，作为工作区里的资产显示；文件先上传，再放到画布上。', needs: 'file', standard: true },
  actions: [{ id: 'replace', label: '替换文件', modes: ['edit'], needs: ['update'], onSource: true, run: pickReplacement }],
  hover: (context) => sourceLabel(context, ImageIcon),
  toolbar: (context, store) => [
    ...(context.mode === 'edit' && context.source?.capabilities.includes('update') && context.readOnlyReason === null ? [{ kind: 'button' as const, id: 'asset-replace', icon: Replace, label: '替换', run: () => pickReplacement(context, store) }] : []),
    { kind: 'menu', id: 'asset-fit', icon: Scan, label: '适应方式', value: fitOf(context), items: [{ value: 'contain', label: '完整显示' }, { value: 'cover', label: '填满' }],
      disabled: context.capabilities.includes('update') && context.mode === 'edit' ? false : '没有修改此 Block 的权限', onPick: (value) => setCellConfig(context, store, { fit: value === 'contain' ? null : value }, `适应方式 → ${value === 'cover' ? '填满' : '完整显示'}`) },
    { kind: 'button', id: 'asset-preview', icon: Eye, label: '预览', run: () => requestIntent(context.cell.entity_id, 'preview') },
  ],
  configFields: [{ key: 'fit', label: '填充方式', kind: 'select', options: [{ value: 'contain', label: '完整显示' }, { value: 'cover', label: '填满' }] }],
  create: (args) => {
    if (args.existingSourceId) return [cellOp(args, 'asset', args.existingSourceId)]
    const file = args.config?.file as { object_id: string; file_name: string } | undefined
    if (!file) throw new Error('图片 / 附件需要先选择文件')
    const { file: _file, ...rest } = args.config ?? {}
    void _file
    return [
      createOp(args.dataId, 'buckyos.asset-ref', args.contentFolderId, args.dataOrderKey, { object_id: file.object_id, file_name: file.file_name }, file.file_name),
      cellOp({ ...args, config: Object.keys(rest).length > 0 ? rest : undefined }, 'asset', args.dataId),
    ]
  },
}

// ---- note (a free or attached annotation shown on the canvas): a coloured sheet; replies are annotations on it (§7.3)

const NOTE_SIZES = TEXT_SIZES

function NoteStatic(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<AnnotationContent>(id), [store, id])
  const read = useLoad<ReadOk<AnnotationContent>>(load, version)
  const entry = useEdit(`key:${id}:body`)
  const payload = read.data?.content.payload
  const style: CSSProperties = { background: typeof payload?.style?.color === 'string' ? payload.style.color : undefined }
  return (
    <div className="aiws-note aiws-block-static" style={style} data-size={typeof payload?.style?.size === 'string' ? payload.style.size : 'm'} data-testid={`aiws-note-${id}`}>
      <div className="aiws-note-body">{payload?.body ?? '…'}</div>
      <NoteReplies noteId={id} />
      {payload?.author && <span className="aiws-note-author">{payload.author}</span>}
      {entry && entry.state !== 'committed' && <span className="aiws-note-dot" data-testid="aiws-edit-state" data-state={entry.state} title={entry.detail ?? entry.state} />}
    </div>
  )
}
function NoteEditorView(context: RenderContext) {
  const onCanvas = useOnFreeCanvas()
  if (!context.source) return null
  return <NoteEditor entityId={context.source.entity_id} entity={context.source} readOnly={context.readOnlyReason !== null} startEditing={context.view === 'canvas'} intentKey={onCanvas ? context.cell.entity_id : undefined} />
}

/** Change the note's look (`style` of the annotation: the author's, or anyone's with `manage`). */
async function setNoteStyle(store: WorkspaceStore, noteId: string, patch: Record<string, Json>, label: string) {
  const read = await store.session.read<AnnotationContent>(noteId)
  const style = { ...(read.content.payload.style ?? {}), ...patch }
  const outcome = await store.submit({ editId: `key:${noteId}:style`, label, operations: [{ op: 'entity.set_keys', entity_id: noteId, keys: [{ key: 'style', value: style, expect: { rev: read.content.key_revs.style ?? 0 } }] }] })
  if (outcome.status === 'rejected') store.notify('error', `没有修改便签：${outcome.code}（只有作者或有管理权限的人可以修改）`)
}

/** Reply to a note: a new annotation whose target is the note (§7.3). */
function replyTo(context: RenderContext) {
  const id = context.source?.entity_id
  if (!id || !context.annotate) return
  context.annotate({ target: { entity_id: id }, label: '回复便签' })
  context.showAnnotations(id)
}

const note: BlockDefinition = {
  type: 'note', version: 1, title: '便签', accepts: ['buckyos.annotation'], allowNoSource: false,
  defaultSize: { w: 220, h: 160 }, cost: { editor: false, html: false },
  Static: NoteStatic, Editor: NoteEditorView,
  chrome: 'none', resize: { aspect: 'locked' },
  catalog: { group: 'text', description: '画布上的自由便签，插入后直接输入内容。', needs: 'none', editAfterInsert: true, standard: true },
  actions: [{ id: 'edit', label: '编辑便签', modes: ['edit'], needs: ['comment'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => (context.annotate ? [{ id: 'reply', kind: 'button', at: 'bottom-right-in', icon: Reply, label: '回复', modes: ['edit', 'view'], run: replyTo }] : []),
  toolbar: (context, store) => {
    const noteId = context.source?.entity_id
    if (!noteId) return []
    const reason = context.mode === 'edit' && context.source?.capabilities.some((c) => c === 'comment' || c === 'manage') ? false : '没有修改此便签的权限'
    return [
      { kind: 'color', id: 'note-color', label: '便签颜色', palette: 'fill', value: undefined, disabled: reason, onPick: (value) => { if (value) void setNoteStyle(store, noteId, { color: value }, '便签颜色') } },
      { kind: 'menu', id: 'note-size', icon: ALargeSmall, label: '字号', items: NOTE_SIZES, disabled: reason, onPick: (value) => { void setNoteStyle(store, noteId, { size: value }, '便签字号') } },
      ...(context.annotate ? [{ kind: 'button' as const, id: 'note-reply', icon: Reply, label: '回复', run: () => replyTo(context) }] : []),
    ]
  },
  create: (args) => (args.existingSourceId ? [cellOp(args, 'note', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.annotation', args.contentFolderId, args.dataOrderKey, { kind: 'note', body: args.title ?? '', style: { color: '#fff2cc' } }),
    cellOp(args, 'note', args.dataId),
  ]),
}

// ---- frame & shape (pure UI Blocks, D3 / D10): the title or label is edited in place (§4.4)

/** A one-line title typed where it is shown; Enter or leaving saves, Esc keeps the old one. */
function InPlaceTitle({ context, className, placeholder }: { context: RenderContext; className: string; placeholder: string }) {
  const store = useStore()
  const [text, setText] = useState(context.payload.title ?? '')
  const done = (save: boolean) => {
    const next = text.trim()
    if (save && next !== (context.payload.title ?? '')) setCellKey(context, store, 'title', next || null, `标题 → ${next || '（清除）'}`)
    context.deactivateEditor()
  }
  return (
    <input className={className} aria-label="标题" data-testid={`aiws-title-edit-${context.cell.entity_id}`} autoFocus value={text} placeholder={placeholder} onChange={(event) => setText(event.target.value)}
      onBlur={() => done(true)} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); done(true) } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) } }} />
  )
}

function frameColor(context: RenderContext) {
  return typeof context.payload.config?.color === 'string' ? context.payload.config.color : 'var(--cp-accent)'
}

function FrameStatic(context: RenderContext) {
  const color = frameColor(context)
  return (
    <div className="aiws-frame" style={{ borderColor: color }} data-testid={`aiws-frame-${context.cell.entity_id}`}>
      <div className="aiws-frame-title" style={{ background: color }}>{context.payload.title ?? '框'}</div>
    </div>
  )
}
function FrameEditor(context: RenderContext) {
  const color = frameColor(context)
  return (
    <div className="aiws-frame" style={{ borderColor: color }} data-testid={`aiws-frame-${context.cell.entity_id}`}>
      <div className="aiws-frame-title is-editing" style={{ background: color }}><InPlaceTitle context={context} className="aiws-title-input" placeholder="框" /></div>
    </div>
  )
}

const canChangeLook = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' ? false : '没有修改此 Block 的权限')

const frame: BlockDefinition = {
  type: 'frame', version: 1, title: '框', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 600, h: 400 }, cost: { editor: false, html: false },
  Static: FrameStatic, Editor: FrameEditor, chrome: 'none',
  catalog: { group: 'layout', description: '带标题的区域框，用来在画布上圈出一组内容。', needs: 'none', standard: true },
  actions: [{ id: 'rename', label: '重命名', modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: (context, store) => [
    { kind: 'color', id: 'frame-color', label: '颜色', palette: 'ink', value: typeof context.payload.config?.color === 'string' ? context.payload.config.color : undefined, disabled: canChangeLook(context), onPick: (value) => setCellConfig(context, store, { color: value || null }, '框的颜色') },
    { kind: 'button', id: 'frame-rename', icon: Pencil, label: '重命名', disabled: canChangeLook(context), run: () => context.activateEditor() },
  ],
  configFields: [{ key: 'color', label: '颜色', kind: 'color' }],
  create: (args) => [cellOp(args, 'frame', undefined, { config: { color: (args.config?.color as string) ?? '#4f8df7' } })],
}

/** Text on a filled shape: dark on a light fill, light on a dark one (whatever the theme). */
function inkOn(fill: string): string {
  const m = /^#([0-9a-f]{6})$/i.exec(fill)
  if (!m) return 'inherit'
  const n = parseInt(m[1], 16)
  const luminance = (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255
  return luminance > 0.6 ? '#1f2328' : '#ffffff'
}

function shapeLook(context: RenderContext) {
  const config = context.payload.config ?? {}
  const fill = typeof config.fill === 'string' ? config.fill : '#e8f0fe'
  return {
    kind: config.shape === 'ellipse' ? 'ellipse' as const : 'rect' as const,
    fill,
    stroke: typeof config.stroke === 'string' ? config.stroke : '#4f8df7',
    ink: inkOn(fill),
  }
}

function ShapeStatic(context: RenderContext) {
  const { kind, fill, stroke, ink } = shapeLook(context)
  return (
    <div className={`aiws-shape aiws-shape-${kind}`} style={{ background: fill, borderColor: stroke, color: ink }} data-testid={`aiws-shape-${context.cell.entity_id}`}>
      {context.payload.title && <span className="aiws-shape-label">{context.payload.title}</span>}
    </div>
  )
}
function ShapeEditor(context: RenderContext) {
  const { kind, fill, stroke, ink } = shapeLook(context)
  return (
    <div className={`aiws-shape aiws-shape-${kind}`} style={{ background: fill, borderColor: stroke, color: ink }} data-testid={`aiws-shape-${context.cell.entity_id}`}>
      <InPlaceTitle context={context} className="aiws-title-input aiws-shape-label" placeholder="文字" />
    </div>
  )
}

const shape: BlockDefinition = {
  type: 'shape', version: 1, title: '形状', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 160, h: 120 }, cost: { editor: false, html: false },
  Static: ShapeStatic, Editor: ShapeEditor, chrome: 'none',
  shape: (context) => shapeLook(context).kind,
  catalog: { group: 'layout', description: '矩形或椭圆，可在属性中改颜色和形状。', needs: 'none', standard: true },
  actions: [{ id: 'label', label: '编辑文字', modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: (context, store) => {
    const look = shapeLook(context)
    const reason = canChangeLook(context)
    return [
      { kind: 'menu', id: 'shape-kind', icon: look.kind === 'ellipse' ? Circle : Square, label: '形状', value: look.kind, disabled: reason,
        items: [{ value: 'rect', label: '矩形', icon: Square }, { value: 'ellipse', label: '椭圆', icon: Circle }], onPick: (value) => setCellConfig(context, store, { shape: value }, `形状 → ${value === 'ellipse' ? '椭圆' : '矩形'}`) },
      { kind: 'color', id: 'shape-fill', label: '填充', palette: 'fill', value: look.fill, disabled: reason, onPick: (value) => setCellConfig(context, store, { fill: value || null }, '形状填充') },
      { kind: 'color', id: 'shape-stroke', label: '描边', palette: 'ink', value: look.stroke, disabled: reason, onPick: (value) => setCellConfig(context, store, { stroke: value || null }, '形状描边') },
      { kind: 'button', id: 'shape-text', icon: Baseline, label: '文字', disabled: reason, run: () => context.activateEditor() },
    ]
  },
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
