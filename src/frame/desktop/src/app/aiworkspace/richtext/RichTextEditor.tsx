/* ProseMirror editor bound to the working Loro document of a RichTextCollab. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { UndoManager } from 'loro-crdt'
import { createNodeFromLoroObj, LoroSyncPlugin, loroUndoPluginKey, type LoroDocType, type LoroNodeMapping } from 'loro-prosemirror'
import { baseKeymap, chainCommands, exitCode, toggleMark } from 'prosemirror-commands'
import { keymap } from 'prosemirror-keymap'
import { DOMSerializer, Node as PMNode, type NodeType, type Schema } from 'prosemirror-model'
import { liftListItem, sinkListItem, splitListItem, wrapInList } from 'prosemirror-schema-list'
import { EditorState, Plugin, type Command } from 'prosemirror-state'
import { EditorView } from 'prosemirror-view'
import { describeError } from '../api/session'
import { testHooks } from '../api/testHooks'
import type { AstNode, EntityEnvelope, Reference, RichTextContent } from '../api/types'
import { useLoad, useStore, useVersion } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { blockIdAt, blockIdPlugin } from './blockId'
import { RichTextCollab } from './collab'
import { astPlainText, deleteDraft, listDrafts, type RichTextDraft } from './drafts'

export interface RichTextEditorProps {
  entityId: string
  /** False while the entity may not be edited (no capability, write lock not held…). */
  editable: boolean
  entities: EntityEnvelope[]
  renderEmbed: (reference: Reference) => ReactNode
  onOpenEntity: (entityId: string) => void
  onAnnotateBlock?: (blockId: string) => void
  /** Called with the collab once it is open (the host uses it to resume after a lock was re-acquired). */
  onCollab?: (collab: RichTextCollab | null) => void
}

export function RichTextEditor(props: RichTextEditorProps) {
  const store = useStore()
  const { entityId, onCollab } = props
  const [opened, setOpened] = useState<{ collab: RichTextCollab | null; error: string | null }>({ collab: null, error: null })
  useEffect(() => {
    let live = true
    let mine: RichTextCollab | null = null
    RichTextCollab.open(store, entityId).then(
      (collab) => {
        if (!live) { void collab.close(); return }
        mine = collab
        setOpened({ collab, error: null })
        onCollab?.(collab)
      },
      (error: unknown) => { if (live) setOpened({ collab: null, error: describeError(error) }) },
    )
    return () => {
      live = false
      onCollab?.(null)
      if (mine) void mine.close()
    }
  }, [store, entityId, onCollab])
  if (opened.error) return <div className="aiws-error" role="alert">无法打开富文本：{opened.error}</div>
  if (!opened.collab) return <div className="aiws-muted">正在载入协作文档…</div>
  return <CollabSurface {...props} collab={opened.collab} />
}

function CollabSurface(props: RichTextEditorProps & { collab: RichTextCollab }) {
  const { collab } = props
  useSyncExternalStore(collab.subscribe, collab.snapshot)
  // A refused update replaces the working doc: the view is rebuilt on the new one.
  return <EditorSurface key={collab.generation} {...props} />
}

type Embeds = ReadonlyMap<HTMLElement, Reference>

function setBlock(type: NodeType, attrs: Record<string, unknown>): Command {
  return (state, dispatch) => {
    const { from, to } = state.selection
    const tr = state.tr
    let applicable = false
    state.doc.nodesBetween(from, to, (node, pos) => {
      if (!node.isTextblock) return true
      if (node.type !== type || Object.entries(attrs).some(([key, value]) => node.attrs[key] !== value)) {
        applicable = true
        // keep the block_id: changing a paragraph into a heading is the same block
        tr.setNodeMarkup(pos, type, { ...attrs, block_id: node.attrs.block_id })
      }
      return false
    })
    if (!applicable) return false
    if (dispatch) dispatch(tr.scrollIntoView())
    return true
  }
}

function insertEmbed(schema: Schema, entityId: string): Command {
  return (state, dispatch) => {
    const { $from } = state.selection
    const pos = $from.depth >= 1 ? $from.after(1) : state.doc.content.size
    if (dispatch) dispatch(state.tr.insert(pos, schema.nodes.object_embed.create({ ref: { entity_id: entityId } })).scrollIntoView())
    return true
  }
}

function insertLink(schema: Schema, entityId: string, label: string): Command {
  return (state, dispatch) => {
    if (!state.selection.$from.parent.isTextblock) return false
    if (dispatch) dispatch(state.tr.replaceSelectionWith(schema.nodes.object_link.create({ ref: { entity_id: entityId }, label })).scrollIntoView())
    return true
  }
}

function buildView(
  host: HTMLElement, store: WorkspaceStore, collab: RichTextCollab, isEditable: () => boolean,
  setEmbeds: (update: (previous: Embeds) => Embeds) => void, onOpenEntity: (entityId: string) => void,
): { view: EditorView; dispose: () => void } {
  const schema = store.pmSchema
  const working = collab.working
  const mapping: LoroNodeMapping = new Map()
  const doc = createNodeFromLoroObj(schema, (working as LoroDocType).getMap('doc'), mapping)

  // Local undo: Loro UndoManager on the working doc; remote imports are never part of a step.
  // Its steps are announced to the UndoCoordinator, which alone decides when one is undone.
  // No merging inside Loro: every local commit is one step and the coordinator groups them into entries,
  // so the single undo stack and the editor's history can never disagree about what "one step" is.
  const undoManager = new UndoManager(working, { mergeInterval: 0, maxUndoSteps: 5000, excludeOriginPrefixes: ['sys:'] })
  const isUndoing = { current: false }
  undoManager.setOnPush((isUndo) => {
    if (isUndo) store.undo.pushEditorStep(collab.entityId, collab.editorId)
    return { value: null, cursors: [] }
  })
  const replay = (run: () => boolean) => {
    isUndoing.current = true
    window.setTimeout(() => { isUndoing.current = false }, 0)
    return run()
  }
  const unregister = store.undo.registerEditor(collab.editorId, {
    undo: () => replay(() => undoManager.undo()),
    redo: () => replay(() => undoManager.redo()),
  })
  // Same key and state shape as loro-prosemirror's undo plugin: its sync plugin reads `isUndoing` from it.
  const undoState = new Plugin({
    key: loroUndoPluginKey,
    state: { init: () => ({ undoManager, canUndo: false, canRedo: false, isUndoing }), apply: (_tr, value) => value },
  })

  const coordinatorUndo: Command = () => { void store.undo.undo(); return true }
  const coordinatorRedo: Command = () => { void store.undo.redo(); return true }
  const { list_item: listItem, hard_break: hardBreak } = schema.nodes
  const hardBreakCommand: Command = chainCommands(exitCode, (state, dispatch) => {
    if (dispatch) dispatch(state.tr.replaceSelectionWith(hardBreak.create()).scrollIntoView())
    return true
  })

  const view = new EditorView(host, {
    state: EditorState.create({
      schema,
      doc,
      plugins: [
        LoroSyncPlugin({ doc: working as LoroDocType, mapping }),
        undoState,
        blockIdPlugin(),
        keymap({
          // The editor has no undo history of its own: these keys go to the UndoCoordinator (design §2.7).
          'Mod-z': coordinatorUndo,
          'Shift-Mod-z': coordinatorRedo,
          'Mod-y': coordinatorRedo,
          'Mod-b': toggleMark(schema.marks.strong),
          'Mod-i': toggleMark(schema.marks.em),
          'Mod-`': toggleMark(schema.marks.code),
          'Shift-Enter': hardBreakCommand,
          Enter: splitListItem(listItem),
          Tab: sinkListItem(listItem),
          'Shift-Tab': liftListItem(listItem),
        }),
        keymap(baseKeymap),
      ],
    }),
    editable: isEditable,
    attributes: { class: 'aiws-prose', spellcheck: 'false', 'data-testid': `aiws-richtext-${collab.entityId}` },
    nodeViews: {
      object_embed: (node) => {
        const dom = document.createElement('div')
        dom.className = 'aiws-embed'
        dom.contentEditable = 'false'
        if (node.attrs.block_id) dom.setAttribute('data-block-id', String(node.attrs.block_id))
        const put = (reference: Reference) => setEmbeds((previous) => new Map(previous).set(dom, reference))
        put(node.attrs.ref as Reference)
        return {
          dom,
          update: (next) => {
            if (next.type !== node.type) return false
            if (next.attrs.block_id) dom.setAttribute('data-block-id', String(next.attrs.block_id))
            put(next.attrs.ref as Reference)
            return true
          },
          destroy: () => setEmbeds((previous) => { const next = new Map(previous); next.delete(dom); return next }),
          stopEvent: () => true,
          ignoreMutation: () => true,
        }
      },
    },
    handleClickOn: (_view, _pos, node) => {
      if (node.type.name !== 'object_link') return false
      onOpenEntity(String((node.attrs.ref as Reference).entity_id))
      return true
    },
    handleDOMEvents: {
      compositionstart: () => { collab.setComposing(true); return false },
      compositionend: () => { window.setTimeout(() => collab.setComposing(false), 0); return false },
      blur: () => { void collab.flush(); return false },
      beforeinput: (_view, event) => {
        // "Undo" from the browser's menu must not reach contenteditable's own history either.
        if (event.inputType === 'historyUndo') { event.preventDefault(); void store.undo.undo(); return true }
        if (event.inputType === 'historyRedo') { event.preventDefault(); void store.undo.redo(); return true }
        return false
      },
    },
  })
  collab.editorJson = () => view.state.doc.toJSON()
  const hooks = testHooks()
  if (hooks) {
    hooks.editors[collab.entityId] = () => view.state.doc.toJSON()
    hooks.canonicalize = (ast) => JSON.parse(store.core.richtext_canonicalize(JSON.stringify(ast))) as unknown
  }
  return {
    view,
    dispose: () => {
      undoManager.setOnPush(undefined)
      unregister()
      if (hooks && hooks.editors[collab.entityId]) delete hooks.editors[collab.entityId]
      collab.editorJson = null
      view.destroy()
      undoManager.free()
    },
  }
}

function EditorSurface({ collab, editable, entities, renderEmbed, onOpenEntity, onAnnotateBlock }: RichTextEditorProps & { collab: RichTextCollab }) {
  const store = useStore()
  const hostRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const editableRef = useRef(editable)
  const openEntityRef = useRef(onOpenEntity)
  const [embeds, setEmbeds] = useState<Embeds>(new Map())
  const [pick, setPick] = useState<'embed' | 'link' | null>(null)
  const [showDrafts, setShowDrafts] = useState(false)

  useEffect(() => {
    editableRef.current = editable
    openEntityRef.current = onOpenEntity
    viewRef.current?.setProps({ editable: () => editable })
  }, [editable, onOpenEntity])

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const built = buildView(host, store, collab, () => editableRef.current, setEmbeds, (id) => openEntityRef.current(id))
    viewRef.current = built.view
    return () => {
      viewRef.current = null
      built.dispose()
    }
  }, [store, collab])

  const run = (command: Command) => {
    const view = viewRef.current
    if (!view) return
    command(view.state, view.dispatch, view)
    view.focus()
  }
  const schema = store.pmSchema
  const cells = entities.filter((entity) => entity.type_id === 'buckyos.cell' && !entity.deleted)
  const linkable = entities.filter((entity) => entity.type_id !== 'buckyos.container' && !entity.deleted)
  const nameOf = (entity: EntityEnvelope) => entity.name ?? entity.title ?? entity.entity_id

  return (
    <div className="aiws-richtext" data-editable={editable ? 'true' : 'false'}>
      {editable && (
        <div className="aiws-toolbar" role="toolbar" aria-label="富文本格式">
          <button type="button" title="加粗 (Ctrl+B)" onMouseDown={(event) => event.preventDefault()} onClick={() => run(toggleMark(schema.marks.strong))}><b>B</b></button>
          <button type="button" title="斜体 (Ctrl+I)" onMouseDown={(event) => event.preventDefault()} onClick={() => run(toggleMark(schema.marks.em))}><i>I</i></button>
          <button type="button" title="删除线" onMouseDown={(event) => event.preventDefault()} onClick={() => run(toggleMark(schema.marks.strike))}><s>S</s></button>
          <button type="button" title="行内代码" onMouseDown={(event) => event.preventDefault()} onClick={() => run(toggleMark(schema.marks.code))}>{'</>'}</button>
          <span className="aiws-toolbar-sep" />
          <button type="button" title="正文" onMouseDown={(event) => event.preventDefault()} onClick={() => run(setBlock(schema.nodes.paragraph, {}))}>正文</button>
          {[1, 2, 3].map((level) => (
            <button key={level} type="button" title={`标题 ${level}`} onMouseDown={(event) => event.preventDefault()} onClick={() => run(setBlock(schema.nodes.heading, { level }))}>H{level}</button>
          ))}
          <button type="button" title="无序列表" onMouseDown={(event) => event.preventDefault()} onClick={() => run(wrapInList(schema.nodes.bullet_list))}>• 列表</button>
          <button type="button" title="有序列表" onMouseDown={(event) => event.preventDefault()} onClick={() => run(wrapInList(schema.nodes.ordered_list))}>1. 列表</button>
          <button type="button" title="移出列表" onMouseDown={(event) => event.preventDefault()} onClick={() => run(liftListItem(schema.nodes.list_item))}>⇤</button>
          <span className="aiws-toolbar-sep" />
          <button type="button" onClick={() => setPick(pick === 'embed' ? null : 'embed')}>嵌入单元…</button>
          <button type="button" onClick={() => setPick(pick === 'link' ? null : 'link')}>对象链接…</button>
          {onAnnotateBlock && (
            <button
              type="button"
              data-testid="aiws-annotate-block"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                const view = viewRef.current
                const blockId = view ? blockIdAt(view.state.doc, view.state.selection.from) : null
                if (blockId) onAnnotateBlock(blockId)
                else store.notify('info', '请先把光标放在要批注的块里。')
              }}
            >批注当前块</button>
          )}
        </div>
      )}
      {pick && (
        <div className="aiws-inline-form">
          <span>{pick === 'embed' ? '嵌入哪个单元：' : '链接到哪个对象：'}</span>
          <select
            aria-label={pick === 'embed' ? '嵌入单元' : '对象链接'}
            defaultValue=""
            onChange={(event) => {
              const id = event.target.value
              const target = entities.find((entity) => entity.entity_id === id)
              if (!target) return
              run(pick === 'embed' ? insertEmbed(schema, id) : insertLink(schema, id, nameOf(target)))
              setPick(null)
            }}
          >
            <option value="" disabled>请选择…</option>
            {(pick === 'embed' ? cells : linkable).map((entity) => <option key={entity.entity_id} value={entity.entity_id}>{nameOf(entity)}（{entity.entity_id}）</option>)}
          </select>
          <button type="button" onClick={() => setPick(null)}>取消</button>
        </div>
      )}
      <div ref={hostRef} className="aiws-prose-host" />
      {[...embeds].map(([dom, reference]) => createPortal(renderEmbed(reference), dom))}
      <DraftsBar entityId={collab.entityId} open={showDrafts} onToggle={() => setShowDrafts((value) => !value)} generation={collab.generation} />
    </div>
  )
}

function DraftsBar({ entityId, open, onToggle, generation }: { entityId: string; open: boolean; onToggle: () => void; generation: number }) {
  const store = useStore()
  const [tick, setTick] = useState(0)
  const [drafts, setDrafts] = useState<RichTextDraft[]>([])
  useEffect(() => {
    const id = window.setTimeout(() => setDrafts(listDrafts(store.session.workspaceId, entityId)), 0)
    return () => window.clearTimeout(id)
  }, [store, entityId, tick, generation])
  if (drafts.length === 0) return null
  return (
    <div className="aiws-drafts" data-testid="aiws-drafts">
      <button type="button" onClick={onToggle}>未被接受的修改已保留为草稿（{drafts.length}）{open ? ' ▾' : ' ▸'}</button>
      {open && drafts.map((draft) => (
        <div key={draft.draft_id} className="aiws-draft">
          <div className="aiws-muted">{new Date(draft.saved_at).toLocaleString()} · {draft.reason}</div>
          <pre>{astPlainText(draft.ast) || JSON.stringify(draft.ast)}</pre>
          <details><summary>导出 JSON</summary><textarea readOnly rows={6} value={JSON.stringify(draft.ast, null, 2)} /></details>
          <button type="button" onClick={() => { deleteDraft(draft.draft_id); setTick((value) => value + 1) }}>删除草稿</button>
        </div>
      ))}
    </div>
  )
}

/** Read-only rendering from the backend's AST projection (used for embedded rich text cells). */
export function StaticRichText({ entityId, renderEmbed }: { entityId: string; renderEmbed: (reference: Reference) => ReactNode }) {
  const store = useStore()
  const version = useVersion(`e:${entityId}`)
  const hostRef = useRef<HTMLDivElement>(null)
  const [embeds, setEmbeds] = useState<Embeds>(new Map())
  const load = useCallback(() => store.session.read<RichTextContent>(entityId), [store, entityId])
  const { data, error } = useLoad(load, version)
  const ast: AstNode | undefined = data?.content.content
  useEffect(() => {
    const host = hostRef.current
    if (!host || !ast) return
    let found = new Map<HTMLElement, Reference>()
    try {
      const node = PMNode.fromJSON(store.pmSchema, ast)
      host.replaceChildren(DOMSerializer.fromSchema(store.pmSchema).serializeFragment(node.content))
      host.querySelectorAll<HTMLElement>('.aiws-embed[data-ref]').forEach((element) => {
        try { found.set(element, JSON.parse(element.getAttribute('data-ref') ?? '') as Reference) } catch { /* shown as an empty box */ }
      })
    } catch (failure) {
      host.textContent = `无法渲染：${describeError(failure)}`
      found = new Map()
    }
    const id = window.setTimeout(() => setEmbeds(found), 0)
    return () => window.clearTimeout(id)
  }, [ast, store])
  if (error) return <div className="aiws-error">{error}</div>
  return (
    <div className="aiws-richtext" data-editable="false">
      <div ref={hostRef} className="aiws-prose aiws-prose-static" />
      {[...embeds].map(([dom, reference]) => createPortal(renderEmbed(reference), dom))}
    </div>
  )
}
