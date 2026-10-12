/* ProseMirror editor bound to the working Loro document of a RichTextCollab. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Bold, Code, CodeXml, FileCode, Heading, Heading1, Heading2, Heading3, Image as ImageIcon, Italic, Link2, List, ListOrdered, ListTree, MessageSquarePlus, Minus, Pilcrow, Sparkles, Strikethrough, Table, TextQuote, Type } from 'lucide-react'
import { UndoManager } from 'loro-crdt'
import { createNodeFromLoroObj, LoroSyncPlugin, loroUndoPluginKey, type LoroDocType, type LoroNodeMapping } from 'loro-prosemirror'
import { baseKeymap, chainCommands, exitCode, toggleMark, wrapIn } from 'prosemirror-commands'
import { keymap } from 'prosemirror-keymap'
import { DOMSerializer, Fragment, Node as PMNode, Slice, type MarkType, type NodeType, type Schema } from 'prosemirror-model'
import { liftListItem, sinkListItem, splitListItem, wrapInList } from 'prosemirror-schema-list'
import { EditorState, Plugin, Selection, TextSelection, type Command } from 'prosemirror-state'
import { EditorView } from 'prosemirror-view'
import { describeError } from '../api/session'
import { testHooks } from '../api/testHooks'
import type { AstNode, EntityEnvelope, Reference, RichTextContent } from '../api/types'
import { useLoad, useStore, useVersion, type AnnotationMark } from '../state/hooks'
import { Emitter } from '../state/emitter'
import type { WorkspaceStore } from '../state/store'
import type { CapturedAnchor } from '../anchors/registry'
import { annotationsPlugin, captureAnchor, makeHost, richTextAnchors, selectionOf, setAnnotations, type Placement } from '../anchors/richtext'
import { AnnotationGutter } from '../ui/annotations'
import { consumeIntent, useEditorToolbar, useIntent } from '../ui/blocks/editorToolbar'
import { useFlowBlock } from '../ui/blocks/flowContext'
import type { ToolbarItem } from '../ui/blocks/registry'
import { registryEntries } from '../ui/canvas/catalog'
import { blockIdAt, blockIdPlugin } from './blockId'
import { RichTextCollab } from './collab'
import { astPlainText, deleteDraft, listDrafts, type RichTextDraft } from './drafts'
import { FormatBubble, MarkdownSourceEditor, SlashMenu, type SlashItem } from './EditorMenus'
import { codeFenceOnEnter, inputRulesPlugin, plainOnBackspace, undoInputRule } from './inputRules'
import { markdownCodec, type MdCodec } from './markdownSource'
import { placeholderPlugin } from './placeholder'
import { closeSlash, slashKey, slashPlugin, type SlashKey, type SlashState } from './slash'

export interface RichTextEditorProps {
  entityId: string
  /** False while the entity may not be edited (no capability, write lock not held…). */
  editable: boolean
  entities: EntityEnvelope[]
  renderEmbed: (reference: Reference) => ReactNode
  onOpenEntity: (entityId: string) => void
  /** Annotations of this rich text, shown where they resolve. */
  annotations?: AnnotationMark[]
  activeAnnotation?: string | null
  onActivateAnnotation?: (entityId: string | null) => void
  /** Start an annotation on the selection (absent without the `comment` capability). */
  onAnnotate?: (anchor: CapturedAnchor) => void
  /** Called with the collab once it is open (the host uses it to resume after a lock was re-acquired). */
  onCollab?: (collab: RichTextCollab | null) => void
  /** A canvas Block's id: where the canvas sends "type this" / "put the caret here" when it opens the editor. */
  intentKey?: string
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
  // the text is shown as it is while the collaborative document opens: no flash of a loading line
  if (!opened.collab) return <div className="aiws-richtext-loading" data-testid="aiws-richtext-loading"><StaticRichText entityId={entityId} renderEmbed={props.renderEmbed} /></div>
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

function markActive(state: EditorState, type: MarkType): boolean {
  const { from, to, empty, $from } = state.selection
  if (empty) return Boolean(type.isInSet(state.storedMarks ?? $from.marks()))
  return state.doc.rangeHasMark(from, to, type)
}

/** The text block kind at the caret: `p`, `h1`–`h3`. */
function blockKind(state: EditorState): string {
  const parent = state.selection.$from.parent
  return parent.type.name === 'heading' ? `h${String(parent.attrs.level)}` : 'p'
}

/** The innermost list around the caret: `bullet`, `ordered` or ''. */
function listKind(state: EditorState): string {
  const $from = state.selection.$from
  for (let depth = $from.depth; depth > 0; depth--) {
    const name = $from.node(depth).type.name
    if (name === 'bullet_list') return 'bullet'
    if (name === 'ordered_list') return 'ordered'
  }
  return ''
}

function insertLink(schema: Schema, entityId: string, label: string): Command {
  return (state, dispatch) => {
    if (!state.selection.$from.parent.isTextblock) return false
    if (dispatch) dispatch(state.tr.replaceSelectionWith(schema.nodes.object_link.create({ ref: { entity_id: entityId }, label })).scrollIntoView())
    return true
  }
}

/** Text pasted without HTML that reads as Markdown (headings, lists, quotes, fences, tables, emphasis, links). */
function looksLikeMarkdown(text: string): boolean {
  return /^\s{0,3}(#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?|```|~~~|\|.*\|\s*$)/m.test(text) || /\*\*[^*\n]+\*\*|`[^`\n]+`|\[[^\]\n]+\]\([^)\s]+\)/.test(text)
}

interface ViewExtras {
  /** Keys of the open `/` menu. */
  slashKeys: (key: SlashKey) => boolean
  toggleSource: () => void
  codec: () => MdCodec
  /** Focus moved in or out (the format bubble follows it). */
  onFocusChange: () => void
  /** Esc in the text: true when the page took it (it selects the Block). */
  onEscape: () => boolean
  /** Ctrl+Alt+M: annotate the selection, or the block around the caret. */
  onAnnotate: () => boolean
}

function buildView(
  host: HTMLElement, store: WorkspaceStore, collab: RichTextCollab, isEditable: () => boolean,
  setEmbeds: (update: (previous: Embeds) => Embeds) => void, onOpenEntity: (entityId: string) => void,
  annotations: { onPlaced: (placements: Placement[]) => void; onClick: (annotationId: string) => void },
  onUpdate: () => void, extras: ViewExtras,
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
  const toggleSource: Command = () => { extras.toggleSource(); return true }

  const view = new EditorView(host, {
    state: EditorState.create({
      schema,
      doc,
      plugins: [
        LoroSyncPlugin({ doc: working as LoroDocType, mapping }),
        undoState,
        blockIdPlugin(),
        slashPlugin((key) => extras.slashKeys(key)),
        inputRulesPlugin(schema),
        placeholderPlugin(),
        annotationsPlugin({ entityId: collab.entityId, loro: () => working, ...annotations }),
        // the format tools show what applies at the caret: they follow every state change
        new Plugin({ view: () => ({ update: onUpdate }) }),
        keymap({
          // The editor has no undo history of its own: these keys go to the UndoCoordinator (design §2.7).
          'Mod-z': coordinatorUndo,
          'Shift-Mod-z': coordinatorRedo,
          'Mod-y': coordinatorRedo,
          'Mod-b': toggleMark(schema.marks.strong),
          'Mod-i': toggleMark(schema.marks.em),
          'Mod-`': toggleMark(schema.marks.code),
          'Mod-e': toggleMark(schema.marks.code),
          'Mod-Shift-s': toggleMark(schema.marks.strike),
          'Mod-Shift-x': toggleMark(schema.marks.strike),
          'Mod-Alt-0': setBlock(schema.nodes.paragraph, {}),
          'Mod-Alt-1': setBlock(schema.nodes.heading, { level: 1 }),
          'Mod-Alt-2': setBlock(schema.nodes.heading, { level: 2 }),
          'Mod-Alt-3': setBlock(schema.nodes.heading, { level: 3 }),
          'Mod-Shift-7': wrapInList(schema.nodes.ordered_list),
          'Mod-Shift-8': wrapInList(schema.nodes.bullet_list),
          'Mod-/': toggleSource,
          Escape: () => extras.onEscape(),
          'Mod-Alt-m': () => extras.onAnnotate(),
          'Shift-Enter': hardBreakCommand,
          Enter: chainCommands(codeFenceOnEnter(schema), splitListItem(listItem)),
          Backspace: chainCommands(undoInputRule, plainOnBackspace(schema, liftListItem(listItem))),
          Tab: sinkListItem(listItem),
          'Shift-Tab': liftListItem(listItem),
        }),
        // Esc leaves the text (the page selects the Block, Notion's way) instead of selecting the parent node
        keymap(Object.fromEntries(Object.entries(baseKeymap).filter(([key]) => key !== 'Escape'))),
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
    handleClickOn: (view, _pos, node, _nodePos, event) => {
      if (node.type.name !== 'object_link') return false
      // while editing, a plain click places the caret; Ctrl/Cmd+click opens the linked object (read-only: a plain click opens it)
      if (view.editable && !(event.ctrlKey || event.metaKey)) return false
      onOpenEntity(String((node.attrs.ref as Reference).entity_id))
      return true
    },
    // Markdown pasted as plain text becomes formatted text (Shift+paste keeps it as typed)
    clipboardTextParser: (text, $context, plain) => {
      if (plain || $context.parent.type.spec.code || !looksLikeMarkdown(text)) return undefined as unknown as Slice
      try {
        const nodes = extras.codec().parse(text.replace(/\r\n?/g, '\n')).map((json) => PMNode.fromJSON(schema, json))
        if (nodes.length === 0) return undefined as unknown as Slice
        return Slice.maxOpen(Fragment.fromArray(nodes))
      } catch {
        return undefined as unknown as Slice
      }
    },
    handleDOMEvents: {
      compositionstart: () => { collab.setComposing(true); return false },
      compositionend: () => { window.setTimeout(() => collab.setComposing(false), 0); return false },
      focus: () => { extras.onFocusChange(); return false },
      blur: () => { void collab.flush(); extras.onFocusChange(); return false },
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
    hooks.richTextAnchors = richTextAnchors
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

const NO_ANNOTATIONS: AnnotationMark[] = []

// ---- Markdown source mode, remembered per rich text on this device (流式页交互改进 §4.3)

const SOURCE_KEY = 'aiworkspace.md-source'

function readSourcePref(entityId: string): boolean {
  try { return (JSON.parse(window.localStorage.getItem(SOURCE_KEY) ?? '[]') as string[]).includes(entityId) } catch { return false }
}

function writeSourcePref(entityId: string, on: boolean) {
  try {
    const list = (JSON.parse(window.localStorage.getItem(SOURCE_KEY) ?? '[]') as string[]).filter((id) => id !== entityId)
    if (on) list.push(entityId)
    window.localStorage.setItem(SOURCE_KEY, JSON.stringify(list.slice(-200)))
  } catch { /* a preference only */ }
}

/** Insert a divider after the caret's top-level block (or in place of an empty paragraph) and continue below it. */
function insertDivider(schema: Schema): Command {
  return (state, dispatch) => {
    const { $from } = state.selection
    if ($from.depth < 1) return false
    const top = $from.node(1)
    const empty = top.type === schema.nodes.paragraph && top.content.size === 0
    const from = empty ? $from.before(1) : $from.after(1)
    const to = empty ? $from.after(1) : from
    if (dispatch) {
      const tr = state.tr.replaceWith(from, to, [schema.nodes.horizontal_rule.create(), schema.nodes.paragraph.create()])
      dispatch(tr.setSelection(TextSelection.create(tr.doc, from + 2)).scrollIntoView())
    }
    return true
  }
}

/** Block types offered in a flow page's `/` menu besides text (data, media, AI); the canvas-only ones are left out. */
const FLOW_SLASH_SKIP = new Set(['richtext', 'note', 'shape', 'frame', 'connector'])
const SLASH_ICONS: Record<string, typeof Table> = { table: Table, asset: ImageIcon, record: ListTree, wish: Sparkles }

function EditorSurface(props: RichTextEditorProps & { collab: RichTextCollab }) {
  const { collab, editable, entities, renderEmbed, onOpenEntity, onAnnotate, intentKey } = props
  const annotations = props.annotations ?? NO_ANNOTATIONS
  const activeAnnotation = props.activeAnnotation ?? null
  const store = useStore()
  const flow = useFlowBlock()
  const hostRef = useRef<HTMLDivElement>(null)
  const [root, bindRoot] = useState<HTMLDivElement | null>(null)
  const viewRef = useRef<EditorView | null>(null)
  const editableRef = useRef(editable)
  const openEntityRef = useRef(onOpenEntity)
  const activateRef = useRef(props.onActivateAnnotation)
  const [embeds, setEmbeds] = useState<Embeds>(new Map())
  const [showDrafts, setShowDrafts] = useState(false)
  const [built, setBuilt] = useState<{ view: EditorView; host: HTMLElement } | null>(null)
  const [placements, setPlacements] = useState<Placement[]>([])
  /** What applies at the caret, for the format tools (recomputed on every editor state change). */
  const [at, setAt] = useState({ strong: false, em: false, strike: false, code: false, block: 'p', list: '' })
  /** Selection, focus and the `/` command: what the floating parts follow. */
  const [float, setFloat] = useState<{ focused: boolean; from: number; to: number; slash: SlashState | null }>({ focused: false, from: 0, to: 0, slash: null })
  const [source, setSource] = useState<{ on: boolean; blockId: string | null }>(() => ({ on: readSourcePref(collab.entityId), blockId: null }))
  const [linkAt, setLinkAt] = useState(false)
  // stable holders read by the editor plugins (created once per view)
  const [slashKeys] = useState(() => new Map<'h', (key: SlashKey) => boolean>())
  const [docEvents] = useState(() => new Emitter())
  const [escapeHolder] = useState(() => new Map<'f' | 'a', () => boolean>())
  const nameOfId = (id: string) => { const entity = entities.find((e) => e.entity_id === id); return entity?.title ?? entity?.name ?? id }
  const [names] = useState(() => new Map<'f', (id: string) => string>())
  const codec = useMemo(() => markdownCodec(store.core, (id) => names.get('f')?.(id) ?? id), [store, names])
  const toggleSourceRef = useRef<() => void>(() => undefined)

  useEffect(() => {
    openEntityRef.current = onOpenEntity
    activateRef.current = props.onActivateAnnotation
    names.set('f', nameOfId)
    // on a page, Esc leaves the text and selects its Block
    escapeHolder.set('f', () => {
      if (!flow) return false
      viewRef.current?.dom.blur()
      flow.select()
      return true
    })
  })
  useEffect(() => {
    editableRef.current = editable
    viewRef.current?.setProps({ editable: () => editable })
  }, [editable])

  // the canvas opened this editor by typing or by double-clicking a spot: the text goes in, the caret goes there
  const applyIntent = (view: EditorView, intent: string) => {
    if (!editableRef.current) return
    if (intent.startsWith('type:')) {
      const end = Selection.atEnd(view.state.doc)
      view.dispatch(view.state.tr.setSelection(end).insertText(intent.slice(5)))
    } else if (intent.startsWith('caret:')) {
      const [left, top] = intent.slice(6).split(',').map(Number)
      const pos = view.posAtCoords({ left, top })
      view.dispatch(view.state.tr.setSelection(pos ? TextSelection.near(view.state.doc.resolve(pos.pos)) : Selection.atEnd(view.state.doc)))
    } else if (intent === 'newline') {
      // "continue writing" below the text: an empty last line, or a new one
      const doc = view.state.doc
      const lastNode = doc.lastChild
      if (lastNode && lastNode.type.name === 'paragraph' && lastNode.content.size === 0) view.dispatch(view.state.tr.setSelection(Selection.atEnd(doc)))
      else {
        const tr = view.state.tr.insert(doc.content.size, view.state.schema.nodes.paragraph.create())
        view.dispatch(tr.setSelection(TextSelection.create(tr.doc, tr.doc.content.size - 1)).scrollIntoView())
      }
    } else view.dispatch(view.state.tr.setSelection(Selection.atEnd(view.state.doc)))
    view.focus()
  }
  const intent = useIntent(intentKey ?? '')

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const follow = () => {
      const view = viewRef.current
      if (!view) return
      const state = view.state
      setFloat((previous) => {
        const next = { focused: view.hasFocus(), from: state.selection.from, to: state.selection.to, slash: slashKey.getState(state) ?? null }
        return previous.focused === next.focused && previous.from === next.from && previous.to === next.to && previous.slash === next.slash ? previous : next
      })
    }
    const result = buildView(host, store, collab, () => editableRef.current, setEmbeds, (id) => openEntityRef.current(id), {
      onPlaced: setPlacements,
      onClick: (id) => activateRef.current?.(id),
    }, () => {
      const view = viewRef.current
      if (!view) return
      const state = view.state
      const marks = store.pmSchema.marks
      const next = { strong: markActive(state, marks.strong), em: markActive(state, marks.em), strike: markActive(state, marks.strike), code: markActive(state, marks.code), block: blockKind(state), list: listKind(state) }
      setAt((previous) => (Object.entries(next).every(([key, value]) => previous[key as keyof typeof previous] === value) ? previous : next))
      follow()
      docEvents.emit()
    }, {
      slashKeys: (key) => slashKeys.get('h')?.(key) ?? false,
      toggleSource: () => toggleSourceRef.current(),
      codec: () => codec,
      onFocusChange: () => window.setTimeout(follow, 0),
      onEscape: () => escapeHolder.get('f')?.() ?? false,
      onAnnotate: () => escapeHolder.get('a')?.() ?? false,
    })
    viewRef.current = result.view
    setBuilt({ view: result.view, host })
    return () => {
      viewRef.current = null
      setBuilt(null)
      result.dispose()
    }
  }, [store, collab, slashKeys, docEvents, codec, escapeHolder])

  useEffect(() => {
    if (!built || built.view.isDestroyed || !intent || !intentKey) return
    consumeIntent(intentKey, intent)
    if (source.on) {
      // the text area takes the caret at its end (typed characters go after it)
      const area = root?.querySelector<HTMLTextAreaElement>('textarea.aiws-md-source')
      if (!area) return
      area.focus()
      area.setSelectionRange(area.value.length, area.value.length)
      if (intent.value.startsWith('type:') && editableRef.current) document.execCommand('insertText', false, intent.value.slice(5))
      return
    }
    applyIntent(built.view, intent.value)
  }, [built, intent, intentKey, source.on, root])

  useEffect(() => {
    if (built && !built.view.isDestroyed) setAnnotations(built.view, annotations, activeAnnotation)
  }, [built, annotations, activeAnnotation])

  const toggleSource = () => {
    const view = built?.view
    if (!view || view.isDestroyed) return
    if (source.on) {
      setSource({ on: false, blockId: null })
      writeSourcePref(collab.entityId, false)
    } else {
      setSource({ on: true, blockId: blockIdAt(view.state.doc, view.state.selection.head) })
      writeSourcePref(collab.entityId, true)
    }
  }
  useEffect(() => { toggleSourceRef.current = toggleSource })
  /** Back from source mode: the caret goes to the end of the block that held the text caret. */
  const leaveSource = (blockId: string | null) => {
    setSource({ on: false, blockId: null })
    writeSourcePref(collab.entityId, false)
    const view = built?.view
    if (!view || view.isDestroyed) return
    window.setTimeout(() => {
      if (view.isDestroyed) return
      let target: Selection = Selection.atEnd(view.state.doc)
      if (blockId) {
        view.state.doc.forEach((node, pos) => {
          if (node.attrs.block_id === blockId) target = Selection.near(view.state.doc.resolve(pos + node.nodeSize), -1)
        })
      }
      view.dispatch(view.state.tr.setSelection(target))
      view.focus()
    }, 0)
  }

  const annotate = () => {
    const view = built?.view
    if (!view || view.isDestroyed || !onAnnotate) return
    const anchor = captureAnchor(makeHost(collab.entityId, view.state.doc, collab.working, selectionOf(view)))
    if (anchor) onAnnotate(anchor)
    else store.notify('info', '请先选中要批注的文字，或把光标放在要批注的块里。')
  }

  useEffect(() => { escapeHolder.set('a', () => { if (!onAnnotate) return false; annotate(); return true }) })

  const run = (command: Command) => {
    const view = built?.view
    if (!view || view.isDestroyed) return
    command(view.state, view.dispatch, view)
    view.focus()
  }
  const schema = store.pmSchema
  const cells = entities.filter((entity) => entity.type_id === 'buckyos.cell' && !entity.deleted)
  const linkable = entities.filter((entity) => entity.type_id !== 'buckyos.container' && !entity.deleted)
  const nameOf = (entity: EntityEnvelope) => entity.name ?? entity.title ?? entity.entity_id
  const pickLink = (kind: 'link' | 'embed', target: EntityEnvelope) => run(kind === 'embed' ? insertEmbed(schema, target.entity_id) : insertLink(schema, target.entity_id, nameOf(target)))
  const blockItems = [{ value: 'p', label: '正文' }, { value: 'h1', label: '标题 1' }, { value: 'h2', label: '标题 2' }, { value: 'h3', label: '标题 3' }]
  const toBlock = (value: string) => run(value === 'p' ? setBlock(schema.nodes.paragraph, {}) : setBlock(schema.nodes.heading, { level: Number(value.slice(1)) }))

  // the format tools (标准对象的交互改进 §5.2): in the near toolbar on a canvas; elsewhere a bubble over a text selection
  const tools: ToolbarItem[] = []
  if (editable) {
    tools.push(
      { kind: 'menu', id: 'block-type', icon: at.block === 'p' ? Pilcrow : Heading, label: '段落格式', value: at.block, items: blockItems, onPick: toBlock },
      { kind: 'separator', id: 'sep-type' },
      { kind: 'button', id: 'bold', icon: Bold, label: '加粗', key: 'Ctrl+B', active: at.strong, run: () => run(toggleMark(schema.marks.strong)) },
      { kind: 'button', id: 'italic', icon: Italic, label: '斜体', key: 'Ctrl+I', active: at.em, run: () => run(toggleMark(schema.marks.em)) },
      { kind: 'button', id: 'strike', icon: Strikethrough, label: '删除线', key: 'Ctrl+Shift+S', active: at.strike, run: () => run(toggleMark(schema.marks.strike)) },
      { kind: 'button', id: 'code', icon: Code, label: '行内代码', key: 'Ctrl+E', active: at.code, run: () => run(toggleMark(schema.marks.code)) },
      { kind: 'menu', id: 'list', icon: at.list === 'ordered' ? ListOrdered : List, label: '列表', value: at.list,
        items: [{ value: 'bullet', label: '无序列表', icon: List }, { value: 'ordered', label: '有序列表', icon: ListOrdered }, { value: 'lift', label: '移出列表' }],
        onPick: (value) => run(value === 'lift' ? liftListItem(schema.nodes.list_item) : wrapInList(value === 'ordered' ? schema.nodes.ordered_list : schema.nodes.bullet_list)) },
      { kind: 'panel', id: 'link', icon: Link2, label: '链接对象或嵌入单元', render: (close) => (
        <LinkPicker objects={linkable} cells={cells} nameOf={nameOf} onPick={(kind, target) => { close(); pickLink(kind, target) }} />
      ) },
    )
  }
  if (onAnnotate) tools.push(...(tools.length ? [{ kind: 'separator' as const, id: 'sep-annotate' }] : []), { kind: 'button', id: 'annotate-text', icon: MessageSquarePlus, label: '批注选中的文字（没有选中时批注光标所在的块）', key: 'Ctrl+Alt+M', run: annotate })
  // the input mode is a tool of its own on a canvas (on a page it sits in the text's corner)
  const nearTools: ToolbarItem[] = editable ? [...tools, { kind: 'separator', id: 'sep-source' }, { kind: 'button', id: 'markdown-source', icon: FileCode, label: 'Markdown 源码（心流输入）', key: 'Ctrl+/', active: source.on, run: toggleSource }] : tools
  const inNearToolbar = useEditorToolbar('richtext', nearTools)

  // ---- `/` menu: turn the line into another kind, or put another Block after this text (on a page)
  const slashRun = (command: Command | null, then?: () => void) => {
    const view = built?.view
    if (!view || view.isDestroyed) return
    const state = slashKey.getState(view.state)
    if (state) view.dispatch(view.state.tr.delete(state.from, view.state.selection.head).setMeta(slashKey, { close: true }))
    else view.dispatch(closeSlash(view.state))
    if (command) command(view.state, view.dispatch, view)
    view.focus()
    then?.()
  }
  const slashItems: SlashItem[] = editable ? [
    { id: 'text', label: '正文', icon: Type, keywords: 'text paragraph zw zhengwen p', group: '基本', run: () => slashRun(setBlock(schema.nodes.paragraph, {})) },
    { id: 'h1', label: '标题 1', icon: Heading1, keywords: 'h1 heading bt biaoti #', hint: '#', group: '基本', run: () => slashRun(setBlock(schema.nodes.heading, { level: 1 })) },
    { id: 'h2', label: '标题 2', icon: Heading2, keywords: 'h2 heading bt biaoti ##', hint: '##', group: '基本', run: () => slashRun(setBlock(schema.nodes.heading, { level: 2 })) },
    { id: 'h3', label: '标题 3', icon: Heading3, keywords: 'h3 heading bt biaoti ###', hint: '###', group: '基本', run: () => slashRun(setBlock(schema.nodes.heading, { level: 3 })) },
    { id: 'bullet', label: '无序列表', icon: List, keywords: 'bullet list ul lb liebiao -', hint: '-', group: '基本', run: () => slashRun(wrapInList(schema.nodes.bullet_list)) },
    { id: 'ordered', label: '有序列表', icon: ListOrdered, keywords: 'ordered number list ol bh bianhao 1.', hint: '1.', group: '基本', run: () => slashRun(wrapInList(schema.nodes.ordered_list)) },
    { id: 'quote', label: '引用', icon: TextQuote, keywords: 'quote blockquote yy yinyong >', hint: '>', group: '基本', run: () => slashRun(wrapIn(schema.nodes.blockquote)) },
    { id: 'code', label: '代码块', icon: CodeXml, keywords: 'code pre dm daima ```', hint: '```', group: '基本', run: () => slashRun(setBlock(schema.nodes.code_block, { language: '' })) },
    { id: 'divider', label: '分割线', icon: Minus, keywords: 'divider hr line fgx fengexian ---', hint: '---', group: '基本', run: () => slashRun(insertDivider(schema)) },
    { id: 'link', label: '链接对象 / 嵌入 Block…', icon: Link2, keywords: 'link embed object lj lianjie qr qianru', group: '基本', run: () => slashRun(null, () => setLinkAt(true)) },
    ...(flow ? registryEntries().filter((entry) => entry.catalog.standard && !FLOW_SLASH_SKIP.has(entry.definition.type)).map((entry): SlashItem => ({
      id: `block-${entry.definition.type}`, label: entry.title, icon: SLASH_ICONS[entry.definition.type] ?? Table,
      keywords: `${entry.definition.type} block ${entry.definition.type === 'table' ? 'bg biaoge database' : entry.definition.type === 'asset' ? 'tp tupian image picture file' : entry.definition.type === 'wish' ? 'xyg xuyuan ai wish' : ''}`,
      group: '在下方插入', run: () => slashRun(null, () => flow.insertAfter(entry)),
    })) : []),
    { id: 'markdown', label: 'Markdown 源码（心流输入）', icon: FileCode, keywords: 'markdown md source ym yuanma xl xinliu notepad', hint: 'Ctrl+/', group: '输入方式', run: () => slashRun(null, toggleSource) },
    ...(onAnnotate ? [{ id: 'annotate', label: '批注这一段', icon: MessageSquarePlus, keywords: 'comment annotate pz pizhu', hint: 'Ctrl+Alt+M', group: '输入方式', run: () => slashRun(null, annotate) }] : []),
  ] : []

  const showBubble = !inNearToolbar && !source.on && built !== null && float.focused && float.from !== float.to && tools.length > 0
  const overlay = root?.closest<HTMLElement>('.aiws-root') ?? null
  return (
    <div ref={bindRoot} className="aiws-richtext" data-editable={editable ? 'true' : 'false'} data-source={source.on ? 'true' : 'false'}>
      {!inNearToolbar && editable && (
        <button type="button" className={`aiws-md-toggle${source.on ? ' is-on' : ''}`} data-testid="aiws-md-toggle" aria-pressed={source.on}
          title={source.on ? '回到排版视图（Ctrl+/）' : 'Markdown 源码：像记事本一样直接输入（Ctrl+/）'} onMouseDown={(event) => event.preventDefault()} onClick={toggleSource}>
          {source.on ? '排版' : 'MD'}
        </button>
      )}
      <div className="aiws-richtext-body">
        <div ref={hostRef} className="aiws-prose-host" hidden={source.on} />
        {source.on && built && (
          <MarkdownSourceEditor key={collab.generation} view={built.view} codec={codec} editable={editable} startBlockId={source.blockId} onExit={leaveSource} subscribeDoc={docEvents.subscribe} />
        )}
        {!source.on && (
          <AnnotationGutter container={built?.host ?? null} marks={annotations} placements={placements} active={activeAnnotation}
            onActivate={(id) => props.onActivateAnnotation?.(id)} version={placements.length} />
        )}
      </div>
      {showBubble && overlay && createPortal(<FormatBubble view={built.view} items={tools} container={overlay} version={`${float.from}:${float.to}`} />, overlay)}
      {!source.on && built && overlay && float.slash && editable && createPortal(
        <SlashMenu view={built.view} state={float.slash} items={slashItems} container={overlay} bindKeys={(handler) => { if (handler) slashKeys.set('h', handler); else slashKeys.delete('h') }} />,
        overlay,
      )}
      {linkAt && editable && (
        <div className="aiws-link-float" onPointerDown={(event) => event.stopPropagation()}>
          <LinkPicker objects={linkable} cells={cells} nameOf={nameOf} onPick={(kind, target) => { setLinkAt(false); pickLink(kind, target) }} />
          <button type="button" className="aiws-link" onClick={() => { setLinkAt(false); built?.view.focus() }}>取消</button>
        </div>
      )}
      {[...embeds].map(([dom, reference]) => createPortal(renderEmbed(reference), dom))}
      <DraftsBar entityId={collab.entityId} open={showDrafts} onToggle={() => setShowDrafts((value) => !value)} generation={collab.generation} />
    </div>
  )
}

/** "Link an object" / "embed a Block": a searchable list instead of a native select (§4.4). */
function LinkPicker({ objects, cells, nameOf, onPick }: { objects: EntityEnvelope[]; cells: EntityEnvelope[]; nameOf: (entity: EntityEnvelope) => string; onPick: (kind: 'link' | 'embed', target: EntityEnvelope) => void }) {
  const [kind, setKind] = useState<'link' | 'embed'>('link')
  const [query, setQuery] = useState('')
  const list = (kind === 'embed' ? cells : objects).filter((entity) => !query || `${nameOf(entity)} ${entity.entity_id}`.toLowerCase().includes(query.toLowerCase())).slice(0, 80)
  return (
    <div className="aiws-near-panel" data-testid="aiws-link-picker">
      <div className="aiws-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={kind === 'link'} data-testid="aiws-link-kind-link" onClick={() => setKind('link')}>对象链接</button>
        <button type="button" role="tab" aria-selected={kind === 'embed'} data-testid="aiws-link-kind-embed" onClick={() => setKind('embed')}>嵌入单元</button>
      </div>
      <input type="search" aria-label="搜索对象" placeholder="搜索名称或 ID" autoFocus value={query} data-testid="aiws-link-search" onChange={(event) => setQuery(event.target.value)} />
      <div className="aiws-near-panel-list">
        {list.map((entity) => (
          <button key={entity.entity_id} type="button" className="aiws-popover-item" data-testid={`aiws-link-pick-${entity.entity_id}`} onClick={() => onPick(kind, entity)}>
            {nameOf(entity)} <span className="aiws-muted">{entity.type_id === 'buckyos.cell' ? entity.view_type : entity.type_id.replace('buckyos.', '')}</span>
          </button>
        ))}
        {list.length === 0 && <div className="aiws-muted">没有匹配的对象</div>}
      </div>
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
