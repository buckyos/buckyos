/* eslint-disable react-refresh/only-export-components -- a registry of definitions, not a component module */
/* Built-in Block definitions (phase two §10.1): table, rich text, record, asset, note (content
 * Blocks pairing with a data entity) and frame / shape (pure UI Blocks; the shape in shapeBlock.tsx). Each declares what it
 * accepts, its static rendering, its editor (mounted only after activation), view-mode behaviour,
 * actions and inspector, and — 标准对象的交互改进 §4–§5 — its own look (the host frame draws none),
 * what hovering it offers and the type section of the near toolbar. They are registered like any
 * extension would be. */

import { useCallback, useEffect, useState, type CSSProperties, type ReactNode } from 'react'
import { ALargeSmall, Columns3, Eye, Image as ImageIcon, ListFilter, ListTree, Plus, Replace, Reply, Scan, Table, Type } from 'lucide-react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { AnnotationContent, Json, Reference, RichTextContent } from '../../api/types'
import { StaticRichText } from '../../richtext/RichTextEditor'
import { isGuestPrincipal } from '../../api/transport'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { authorLabel, readGuestName } from '../../state/guest'
import { useEdit, useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { TableViewCell } from '../TableViewCell'
import { sourceLabel } from './affordances'
import { consumeIntent, requestIntent, useIntent, useOnFreeCanvas } from './editorToolbar'
import { AssetCell, LockScope, NoteEditor, NoteReplies, RecordCell, RichTextCell, TableEditor } from './editors'
import { cellOp, createOp, MAX_EMBED_DEPTH, replaceAsset, setCellConfig } from './ops'
import { blockRegistry, type BlockDefinition, type RenderContext, type ToolbarItem } from './registry'
import { frameDefinition } from './frameBlock'
import { shapeDefinition } from './shapeBlock'

const textSizes = () => [
  { value: 's', label: aiwsT('aiworkspace.blocks.size.s') }, { value: 'm', label: aiwsT('aiworkspace.blocks.size.m') },
  { value: 'l', label: aiwsT('aiworkspace.blocks.size.l') }, { value: 'xl', label: aiwsT('aiworkspace.blocks.size.xl') },
]

/** Open the Block's editor with `intent` waiting for it (a panel to open, text typed into it). */
function openWith(context: RenderContext, intent: string) {
  requestIntent(context.cell.entity_id, intent)
  context.activateEditor()
}

/** Rich text `object_embed` inside a Block: rendered read-only through the shell's renderer. */
function useEmbedRenderer(depth: number) {
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  return useCallback((reference: Reference): ReactNode => {
    if (depth >= MAX_EMBED_DEPTH) return <div className="aiws-muted">{t('aiworkspace.blocks.embedTooDeep', { id: reference.entity_id })}</div>
    return ui.renderCell(reference.entity_id, depth + 1)
  }, [ui, depth, t])
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
  const { t } = useAiwsText()
  return <div className="aiws-block-simplified">{t('aiworkspace.blocks.table.simplified', { name: context.source?.name ?? context.source?.title ?? context.payload.title ?? '' })}</div>
}

const canAppend = (context: RenderContext) => context.readOnlyReason === null && Boolean(context.source?.capabilities.some((c) => c === 'append' || c === 'update'))

const table: BlockDefinition = {
  type: 'table', version: 1, get title() { return aiwsT('aiworkspace.blocks.table.title') }, accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 640, h: 320 }, cost: { editor: true, html: false },
  Static: TableStatic, Simplified: TableSimplified, Editor: TableEditorView,
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.blocks.table.description') }, needs: 'none', standard: true },
  actions: [{ id: 'edit', get label() { return aiwsT('aiworkspace.blocks.action.editContent') }, modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => [
    ...sourceLabel(context, Table),
    ...(canAppend(context) ? [{ id: 'add-row', kind: 'button' as const, at: 'bottom-out' as const, icon: Plus, label: aiwsT('aiworkspace.blocks.table.addRow'), run: (c: RenderContext) => openWith(c, 'add') }] : []),
  ],
  // while editing, the table's own tools come from its editor (they also show which panel is open)
  toolbar: (context) => context.editorActive ? [] : [
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: aiwsT('aiworkspace.blocks.table.filter'), run: () => openWith(context, 'filter') },
    ...(context.readOnlyReason === null && context.source?.capabilities.includes('structure') ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: aiwsT('aiworkspace.blocks.table.fields'), run: () => openWith(context, 'fields') }] : []),
    ...(canAppend(context) ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: aiwsT('aiworkspace.blocks.table.addRecord'), run: () => openWith(context, 'add') }] : []),
  ],
  create: (args) => (args.existingSourceId ? [cellOp(args, 'table', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.table-source', args.contentFolderId, args.dataOrderKey, {
      title_field_id: 'title', fields: [{ field_id: 'title', name: aiwsT('aiworkspace.blocks.table.fieldTitle'), type: 'text', required: true }, { field_id: 'done', name: aiwsT('aiworkspace.blocks.table.fieldDone'), type: 'boolean' }],
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
  if (!context.source) return null
  const { size, style } = textStyle(context)
  return (
    <div className="aiws-block-static" data-text-size={size} style={style}>
      <RichTextCell key={context.source.entity_id} entity={context.source} entityId={context.source.entity_id} renderEmbed={renderEmbed} intentKey={context.cell.entity_id} />
    </div>
  )
}

function RichTextSimplified(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<RichTextContent>(id), [store, id])
  const read = useLoad<ReadOk<RichTextContent>>(load, version)
  const text = read.data ? firstText(read.data.content.content) : ''
  return <div className="aiws-block-simplified">{text || t('aiworkspace.blocks.richtext.title')}</div>
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
  const reason = context.capabilities.includes('update') && context.mode === 'edit' ? false : aiwsT('aiworkspace.blocks.noPermission')
  const sizes = textSizes()
  return [
    { kind: 'menu', id: 'text-size', icon: ALargeSmall, label: aiwsT('aiworkspace.blocks.fontSize'), value: size, items: sizes, disabled: reason,
      onPick: (value) => setCellConfig(context, store, { text_size: value === 'm' ? null : value }, aiwsT('aiworkspace.blocks.edit.change', { label: aiwsT('aiworkspace.blocks.fontSize'), value: sizes.find((t) => t.value === value)?.label ?? value })) },
    { kind: 'color', id: 'text-color', label: aiwsT('aiworkspace.blocks.textColor'), palette: 'ink', value: typeof color === 'string' ? color : '', disabled: reason, onPick: (value) => setCellConfig(context, store, { text_color: value || null }, aiwsT('aiworkspace.blocks.textColor')) },
  ]
}

const richtext: BlockDefinition = {
  type: 'richtext', version: 1, get title() { return aiwsT('aiworkspace.blocks.richtext.title') }, accepts: ['buckyos.richtext'], allowNoSource: false,
  defaultSize: { w: 420, h: 220 }, cost: { editor: true, html: false },
  Static: RichTextStatic, Simplified: RichTextSimplified, Editor: RichTextEditorView,
  catalog: { group: 'text', get description() { return aiwsT('aiworkspace.blocks.richtext.description') }, needs: 'none', editAfterInsert: true, standard: true },
  actions: [{ id: 'edit', get label() { return aiwsT('aiworkspace.blocks.action.editContent') }, modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
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
  type: 'record', version: 1, get title() { return aiwsT('aiworkspace.blocks.record.title') }, accepts: ['buckyos.record'], allowNoSource: false,
  defaultSize: { w: 360, h: 180 }, cost: { editor: false, html: false },
  Static: RecordStatic, Editor: RecordEditorView,
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.blocks.record.description') }, needs: 'none', editAfterInsert: true },
  actions: [{ id: 'edit', get label() { return aiwsT('aiworkspace.blocks.action.editContent') }, modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => sourceLabel(context, ListTree),
  create: (args) => (args.existingSourceId ? [cellOp(args, 'record', args.existingSourceId)] : [
    createOp(args.dataId, 'buckyos.record', args.contentFolderId, args.dataOrderKey, {
      schema: { properties: [{ key: 'name', name: aiwsT('aiworkspace.blocks.record.propName'), type: 'text' }, { key: 'value', name: aiwsT('aiworkspace.blocks.record.propValue'), type: 'text' }] }, props: { name: args.title ?? '' },
    }, args.title || undefined),
    cellOp(args, 'record', args.dataId),
  ]),
}

// ---- asset (image): the picture itself; no editor — replace, fit and preview are toolbar actions (§4.4)

function fitOf(context: RenderContext): 'contain' | 'cover' {
  return (context.payload.config?.fit ?? context.payload.options?.fit) === 'cover' ? 'cover' : 'contain'
}

function AssetPreview({ entityId, onClose }: { entityId: string; onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-media-preview" role="dialog" aria-label={t('aiworkspace.blocks.asset.previewDialog')} data-testid="aiws-media-preview" onClick={onClose}>
      <div className="aiws-media-preview-body" onClick={(event) => event.stopPropagation()}>
        <AssetCell entityId={entityId} fit="contain" readOnly />
        <button type="button" onClick={onClose}>{t('aiworkspace.blocks.asset.closePreview')}</button>
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
  const { t } = useAiwsText()
  const [preview, setPreview] = usePreviewIntent(context.cell.entity_id)
  const bare = useOnFreeCanvas()
  if (!context.source) return null
  return (
    <div className="aiws-block-static aiws-block-asset" data-testid="aiws-asset-view">
      <AssetCell entityId={context.source.entity_id} fit={fitOf(context)} readOnly bare={bare} />
      <button type="button" className="aiws-link aiws-block-view-action" data-testid="aiws-asset-preview" onClick={() => setPreview(true)}>{t('aiworkspace.blocks.asset.preview')}</button>
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

const FIT_OPTIONS = [
  { value: 'contain', get label() { return aiwsT('aiworkspace.blocks.asset.fitContain') } },
  { value: 'cover', get label() { return aiwsT('aiworkspace.blocks.asset.fitCover') } },
]

const asset: BlockDefinition = {
  type: 'asset', version: 1, get title() { return aiwsT('aiworkspace.blocks.asset.title') }, accepts: ['buckyos.asset-ref'], allowNoSource: false,
  defaultSize: { w: 320, h: 240 }, cost: { editor: false, html: false },
  Static: AssetStatic, View: AssetView,
  resize: { aspect: 'locked' },
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.blocks.asset.description') }, needs: 'file', standard: true },
  actions: [{ id: 'replace', get label() { return aiwsT('aiworkspace.blocks.asset.replaceFile') }, modes: ['edit'], needs: ['update'], onSource: true, run: pickReplacement }],
  hover: (context) => sourceLabel(context, ImageIcon),
  toolbar: (context, store) => [
    ...(context.mode === 'edit' && context.source?.capabilities.includes('update') && context.readOnlyReason === null ? [{ kind: 'button' as const, id: 'asset-replace', icon: Replace, label: aiwsT('aiworkspace.blocks.asset.replace'), run: () => pickReplacement(context, store) }] : []),
    { kind: 'menu', id: 'asset-fit', icon: Scan, label: aiwsT('aiworkspace.blocks.asset.fit'), value: fitOf(context), items: FIT_OPTIONS,
      disabled: context.capabilities.includes('update') && context.mode === 'edit' ? false : aiwsT('aiworkspace.blocks.noPermission'),
      onPick: (value) => setCellConfig(context, store, { fit: value === 'contain' ? null : value }, aiwsT('aiworkspace.blocks.edit.change', { label: aiwsT('aiworkspace.blocks.asset.fit'), value: aiwsT(value === 'cover' ? 'aiworkspace.blocks.asset.fitCover' : 'aiworkspace.blocks.asset.fitContain') })) },
    { kind: 'button', id: 'asset-preview', icon: Eye, label: aiwsT('aiworkspace.blocks.asset.preview'), run: () => requestIntent(context.cell.entity_id, 'preview') },
  ],
  configFields: [{ key: 'fit', get label() { return aiwsT('aiworkspace.blocks.asset.fitField') }, kind: 'select', options: FIT_OPTIONS }],
  create: (args) => {
    if (args.existingSourceId) return [cellOp(args, 'asset', args.existingSourceId)]
    const file = args.config?.file as { object_id: string; file_name: string } | undefined
    if (!file) throw new Error(aiwsT('aiworkspace.blocks.asset.fileRequired'))
    const { file: _file, ...rest } = args.config ?? {}
    void _file
    return [
      createOp(args.dataId, 'buckyos.asset-ref', args.contentFolderId, args.dataOrderKey, { object_id: file.object_id, file_name: file.file_name }, file.file_name),
      cellOp({ ...args, config: Object.keys(rest).length > 0 ? rest : undefined }, 'asset', args.dataId),
    ]
  },
}

// ---- note (a free or attached annotation shown on the canvas): a coloured sheet; replies are annotations on it (§7.3)


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
      {payload && authorLabel(payload) && <span className="aiws-note-author">{authorLabel(payload)}</span>}
      {entry && entry.state !== 'committed' && <span className="aiws-note-dot" data-testid="aiws-edit-state" data-state={entry.state} title={entry.detail ?? entry.state} />}
    </div>
  )
}
function NoteEditorView(context: RenderContext) {
  const onCanvas = useOnFreeCanvas()
  if (!context.source) return null
  return <NoteEditor entityId={context.source.entity_id} entity={context.source} readOnly={context.readOnlyReason !== null} startEditing={context.view === 'canvas'} intentKey={onCanvas ? context.cell.entity_id : undefined} />
}

/** A guest signs its notes with the name it chose, as its annotations (the principal stays `author`). */
function guestAuthorName(store: WorkspaceStore): { author_name?: string } {
  const name = isGuestPrincipal(store.session.principal ?? '') ? readGuestName() : ''
  return name ? { author_name: name } : {}
}

/** Change the note's look (`style` of the annotation: the author's, or anyone's with `manage`). */
async function setNoteStyle(store: WorkspaceStore, noteId: string, patch: Record<string, Json>, label: string) {
  const read = await store.session.read<AnnotationContent>(noteId)
  const style = { ...(read.content.payload.style ?? {}), ...patch }
  const outcome = await store.submit({ editId: `key:${noteId}:style`, label, operations: [{ op: 'entity.set_keys', entity_id: noteId, keys: [{ key: 'style', value: style, expect: { rev: read.content.key_revs.style ?? 0 } }] }] })
  if (outcome.status === 'rejected') store.notify('error', aiwsT('aiworkspace.blocks.note.notChanged', { code: outcome.code }))
}

/** Reply to a note: a new annotation whose target is the note (§7.3). */
function replyTo(context: RenderContext) {
  const id = context.source?.entity_id
  if (!id || !context.annotate) return
  context.annotate({ target: { entity_id: id }, label: aiwsT('aiworkspace.blocks.note.replyAnchor') })
  context.showAnnotations(id)
}

const note: BlockDefinition = {
  type: 'note', version: 1, get title() { return aiwsT('aiworkspace.blocks.note.title') }, accepts: ['buckyos.annotation'], allowNoSource: false,
  defaultSize: { w: 220, h: 160 }, cost: { editor: false, html: false },
  Static: NoteStatic, Editor: NoteEditorView,
  chrome: 'none', resize: { aspect: 'locked' },
  catalog: { group: 'text', get description() { return aiwsT('aiworkspace.blocks.note.description') }, needs: 'none', editAfterInsert: true, standard: true },
  actions: [{ id: 'edit', get label() { return aiwsT('aiworkspace.blocks.note.edit') }, modes: ['edit'], needs: ['comment'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => (context.annotate ? [{ id: 'reply', kind: 'button', at: 'bottom-right-in', icon: Reply, label: aiwsT('aiworkspace.blocks.note.reply'), modes: ['edit', 'view'], run: replyTo }] : []),
  toolbar: (context, store) => {
    const noteId = context.source?.entity_id
    if (!noteId) return []
    const reason = context.mode === 'edit' && context.source?.capabilities.some((c) => c === 'comment' || c === 'manage') ? false : aiwsT('aiworkspace.blocks.note.noPermission')
    return [
      { kind: 'color', id: 'note-color', label: aiwsT('aiworkspace.blocks.note.color'), palette: 'fill', value: undefined, disabled: reason, onPick: (value) => { if (value) void setNoteStyle(store, noteId, { color: value }, aiwsT('aiworkspace.blocks.note.color')) } },
      { kind: 'menu', id: 'note-size', icon: ALargeSmall, label: aiwsT('aiworkspace.blocks.fontSize'), items: textSizes(), disabled: reason, onPick: (value) => { void setNoteStyle(store, noteId, { size: value }, aiwsT('aiworkspace.blocks.note.fontSize')) } },
      ...(context.annotate ? [{ kind: 'button' as const, id: 'note-reply', icon: Reply, label: aiwsT('aiworkspace.blocks.note.reply'), run: () => replyTo(context) }] : []),
    ]
  },
  create: (args) => (args.existingSourceId ? [cellOp(args, 'note', args.existingSourceId)] : [
    // a title given at insertion is the note's text (a note written first, then placed): the Block itself has none
    createOp(args.dataId, 'buckyos.annotation', args.contentFolderId, args.dataOrderKey, { kind: 'note', body: args.title ?? '', style: { color: '#fff2cc' }, ...guestAuthorName(args.store) }),
    cellOp({ ...args, title: undefined }, 'note', args.dataId),
  ]),
}

export const BUILTIN_BLOCKS: BlockDefinition[] = [table, richtext, record, asset, note, frameDefinition, shapeDefinition]

let registered = false
/** Register the built-ins once (idempotent). Extensions register themselves the same way. */
export function registerBuiltinBlocks() {
  if (registered) return
  registered = true
  for (const def of BUILTIN_BLOCKS) blockRegistry.register(def)
}
