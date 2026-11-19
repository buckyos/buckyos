/* eslint-disable react-refresh/only-export-components -- a definition with its renderers, not a component module */
/* The `mindnode` Block (脑图方案 §4.2, R17): a text node of a mind map. Its text is the title (inline Markdown,
 * ≤ 256 characters), drawn as the lines the shared measurer computed — what the layout sized is what is shown.
 * Typed in place like a shape (Enter or leaving saves, Shift+Enter breaks the line, Esc keeps the old text); Tab saves
 * and adds a child. Outside a control it draws like a root. Its look follows its depth and branch colour. */

import { useCallback, useContext, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type Ref } from 'react'
import { ChevronsDownUp, CornerDownRight, ListPlus, LocateFixed, Palette, SquarePlus } from 'lucide-react'
import { MAX_TEXT_WIDTH, NODE_PAD, runFont, typography } from '@controls/measure'
import { isAuto, num } from '@controls/placement'
import { aiwsT } from '../../../i18n'
import { useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { BlockMetaContext, consumeIntent, useIntent } from '../../blocks/editorToolbar'
import type { BlockDefinition, RenderContext } from '../../blocks/registry'
import type { WorkspaceStore } from '../../../state/store'
import { controlCommands } from '../commands'
import { foldBook } from '../folds'
import { nodeMeasurer } from '../measure'
import { blockChildren, nodeConfigOps, nodeInfo, NODE_VIEW, resetLayoutOps } from './ops'

function useNodeLook(context: RenderContext) {
  const store = useStore()
  const id = context.cell.entity_id
  const info = nodeInfo(store.outline, id)
  const depth = info?.depth ?? 0
  const width = num(context.cell.placement?.w)
  const maxText = width !== undefined ? Math.max(1, width - NODE_PAD.x * 2) : MAX_TEXT_WIDTH
  return { info, depth, maxText }
}

function NodeFrame({ context, children, ref }: { context: RenderContext; children?: ReactNode; ref?: Ref<HTMLDivElement> }) {
  const { info, depth, maxText } = useNodeLook(context)
  const t = typography(depth)
  // while editing the node grows with its text up to the width the layout wraps at, so typing does not wrap early
  const style = { '--aiws-branch': info?.color ?? undefined, '--aiws-node-max-w': `${maxText + NODE_PAD.x * 2}px`, padding: `${NODE_PAD.y}px ${NODE_PAD.x}px`, lineHeight: `${t.lineHeight}px`, fontSize: t.size, fontWeight: t.weight } as CSSProperties
  return (
    <div ref={ref} className={`aiws-mindnode is-depth-${Math.min(depth, 2)}`} style={style} data-testid={`aiws-mindnode-${context.cell.entity_id}`} data-depth={depth}>
      {children ?? <NodeLines title={context.payload.title ?? ''} depth={depth} maxText={maxText} />}
    </div>
  )
}

/** The measured lines with their inline formatting. */
function NodeLines({ title, depth, maxText }: { title: string; depth: number; maxText: number }) {
  const { t } = useAiwsText()
  if (!title) return <span className="aiws-mindnode-empty">{t('aiworkspace.mindmap.emptyNode')}</span>
  const measured = nodeMeasurer.measure(title, depth, maxText)
  return (
    <>
      {measured.lines.map((line, i) => (
        <div key={i} className="aiws-mindnode-line">
          {line.runs.map((r, j) => {
            const style: CSSProperties = { font: runFont(r, depth), textDecoration: [r.strike ? 'line-through' : '', r.href ? 'underline' : ''].filter(Boolean).join(' ') || undefined }
            return r.href
              ? <a key={j} href={r.href} target="_blank" rel="noreferrer noopener" style={style} onPointerDown={(e) => e.stopPropagation()}>{r.text}</a>
              : <span key={j} className={r.code ? 'is-code' : undefined} style={style}>{r.text}</span>
          })}
        </div>
      ))}
    </>
  )
}

function NodeStatic(context: RenderContext) {
  return <NodeFrame context={context} />
}

/** Write the title (the Markdown source of the node's text). */
function saveTitle(context: RenderContext, store: ReturnType<typeof useStore>, next: string) {
  const before = context.payload.title ?? ''
  if (next === before) return
  const id = context.cell.entity_id
  const expect = { rev: context.keyRevs.title ?? 0 }
  const label = aiwsT('aiworkspace.blocks.edit.change', { label: aiwsT('aiworkspace.mindmap.text'), value: next || aiwsT('aiworkspace.blocks.edit.cleared') })
  void store.submit({ editId: `key:${id}:title`, label, operations: [next
    ? { op: 'entity.set_keys', entity_id: id, keys: [{ key: 'title', value: [...next].slice(0, 256).join(''), expect }] }
    : { op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'title', expect }] }] })
}

function NodeEditor(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const ref = useRef<HTMLDivElement>(null)
  const metaSink = useContext(BlockMetaContext)
  const id = context.cell.entity_id
  const observeFrame = useCallback((frame: HTMLDivElement | null) => {
    if (frame) return metaSink?.observeEditorSize(id, frame)
  }, [metaSink, id])
  const closed = useRef(false)
  const intent = useIntent(context.cell.entity_id)
  const [started] = useState(() => (intent?.value.startsWith('type:') ? intent.value.slice(5) : null))
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    // typing on a selected node replaces its text; "edit" keeps it with the caret at the end
    el.textContent = started ?? context.payload.title ?? ''
    el.focus()
    const range = document.createRange()
    range.selectNodeContents(el)
    range.collapse(false)
    const selection = window.getSelection()
    selection?.removeAllRanges()
    selection?.addRange(range)
    if (intent) consumeIntent(context.cell.entity_id, intent)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  const done = (save: boolean, then?: 'add-child' | 'add-sibling') => {
    if (closed.current) return
    closed.current = true
    if (save && ref.current) saveTitle(context, store, (ref.current.innerText ?? '').replace(/\n$/, ''))
    context.deactivateEditor()
    if (then) controlCommands.send({ kind: then, nodeId: context.cell.entity_id })
  }
  return (
    <NodeFrame context={context} ref={observeFrame}>
      <div ref={ref} className="aiws-mindnode-edit" contentEditable="plaintext-only" suppressContentEditableWarning role="textbox" aria-multiline="true" aria-label={t('aiworkspace.mindmap.text')}
        data-role="editor" data-testid={`aiws-mindnode-edit-${context.cell.entity_id}`} data-placeholder={t('aiworkspace.mindmap.emptyNode')}
        onBlur={() => done(true)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return
          if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); done(true) }
          else if (event.key === 'Tab' && !event.shiftKey) { event.preventDefault(); done(true, 'add-child') }
          else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) }
        }} />
    </NodeFrame>
  )
}

const info = (context: RenderContext, store: WorkspaceStore) => nodeInfo(store.outline, context.cell.entity_id)
/** Adding and moving nodes restructures the map: `structure` on the node, in edit mode. */
const structureReason = (context: RenderContext): string | false => (context.cell.capabilities.includes('structure') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.blocks.noPermission'))
/** A node's own config key (`auto` and empty remove it). */
const configOps = (key: string) => (context: RenderContext, store: WorkspaceStore, value: unknown) => nodeConfigOps(store, context.cell.entity_id, { [key]: value === 'auto' || !value ? null : String(value) })

/** The node's actions (§7): add a child / sibling, insert any object as a child, fold (the reader's own view, also in
 * view mode), back to the layout. Its branch colour and its side under a two-sided root are its properties. */
const nodeActions: BlockDefinition['actions'] = [
  { id: 'mindmap-add-child', icon: CornerDownRight, primary: true, get label() { return aiwsT('aiworkspace.mindmap.addChild') }, modes: ['edit'], key: 'Tab', disabled: structureReason,
    when: (context, store) => Boolean(info(context, store)), run: (context) => controlCommands.send({ kind: 'add-child', nodeId: context.cell.entity_id }) },
  // Enter adds a sibling here (the control's keys): it is not the node's "edit content"
  { id: 'mindmap-add-sibling', icon: ListPlus, primary: true, get label() { return aiwsT('aiworkspace.mindmap.addSibling') }, modes: ['edit'], key: 'Enter', group: 'type', disabled: structureReason,
    when: (context, store) => (info(context, store)?.depth ?? 0) > 0, run: (context) => controlCommands.send({ kind: 'add-sibling', nodeId: context.cell.entity_id }) },
  { id: 'mindmap-insert', icon: SquarePlus, primary: true, get label() { return aiwsT('aiworkspace.mindmap.insertObject') }, modes: ['edit'], disabled: structureReason,
    when: (context, store) => Boolean(info(context, store)), run: (context) => controlCommands.send({ kind: 'insert', nodeId: context.cell.entity_id }) },
  { id: 'mindmap-fold', icon: ChevronsDownUp, primary: true, get label() { return aiwsT('aiworkspace.mindmap.fold') }, modes: ['edit', 'view'], key: 'Ctrl+/',
    when: (context, store) => Boolean(info(context, store)) && blockChildren(store.outline, context.cell.entity_id).length > 0,
    labelFor: (context, store) => aiwsT(foldBook(store).isFolded(info(context, store)!.control.entity_id, context.cell.entity_id) ? 'aiworkspace.mindmap.unfold' : 'aiworkspace.mindmap.fold'),
    active: (context, store) => foldBook(store).isFolded(info(context, store)!.control.entity_id, context.cell.entity_id),
    run: (context, store) => foldBook(store).toggle(info(context, store)!.control.entity_id, context.cell.entity_id) },
  { id: 'mindmap-auto', icon: LocateFixed, primary: true, get label() { return aiwsT('aiworkspace.mindmap.backToLayout') }, modes: ['edit'], disabled: structureReason,
    when: (context) => { const p = context.cell.placement; return Boolean(p && (!isAuto(p.x) || !isAuto(p.y))) },
    run: (context, store) => { const id = context.cell.entity_id; void store.submit({ editId: `layout:${id}`, label: aiwsT('aiworkspace.mindmap.backToLayout'), operations: resetLayoutOps(store, [id]) }) } },
]

const nodeFields: BlockDefinition['configFields'] = [
  { key: 'branch_color', id: 'mindmap-branch', get label() { return aiwsT('aiworkspace.mindmap.branchColor') }, kind: 'color', quick: true, palette: 'ink', icon: Palette, default: 'auto',
    get extras() { return [{ value: 'auto', label: aiwsT('aiworkspace.mindmap.branchAuto') }] },
    when: (context, store) => (info(context, store)?.depth ?? 0) > 0, get: (context, store) => context.cell.branch_color ?? info(context, store)?.color ?? undefined,
    ops: configOps('branch_color'), editLabel: () => aiwsT('aiworkspace.mindmap.branchColor') },
  { key: 'side', id: 'mindmap-side', get label() { return aiwsT('aiworkspace.mindmap.side') }, kind: 'select', quick: true, default: 'auto',
    get options() { return [{ value: 'auto', label: aiwsT('aiworkspace.mindmap.sideAuto') }, { value: 'left', label: aiwsT('aiworkspace.mindmap.sideLeft') }, { value: 'right', label: aiwsT('aiworkspace.mindmap.sideRight') }] },
    when: (context, store) => { const i = info(context, store); return i?.depth === 1 && i.config.direction === 'both' }, get: (context) => context.cell.side ?? 'auto',
    ops: configOps('side'), editLabel: () => aiwsT('aiworkspace.mindmap.side') },
]

export const mindnodeDefinition: BlockDefinition = {
  type: NODE_VIEW, version: 1, get title() { return aiwsT('aiworkspace.mindmap.node') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 120, h: 40 }, cost: { editor: false, html: false }, chrome: 'none',
  // a selected node takes typing at once (UI 框架 PRD §10.1: the type declares it)
  traits: { typeToEdit: true },
  Static: NodeStatic, Editor: NodeEditor,
  shape: 'rect',
  // a node's look is its depth and branch: no hover label, the text is already there
  hover: () => [],
  actions: nodeActions,
  configFields: nodeFields,
}
