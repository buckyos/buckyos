/* eslint-disable react-refresh/only-export-components -- a definition with its renderers, not a component module */
/* The `mindnode` Block (脑图方案 §4.2, R17): a text node of a mind map. Its text is the title (inline Markdown,
 * ≤ 256 characters), drawn as the lines the shared measurer computed — what the layout sized is what is shown.
 * Typed in place like a shape (Enter or leaving saves, Shift+Enter breaks the line, Esc keeps the old text); Tab saves
 * and adds a child. Outside a control it draws like a root. Its look follows its depth and branch colour. */

import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { CornerDownRight, ListPlus, LocateFixed, Minus, Plus, SquarePlus } from 'lucide-react'
import { MAX_TEXT_WIDTH, NODE_PAD, runFont, typography } from '@controls/measure'
import { isAuto, num } from '@controls/placement'
import { aiwsT } from '../../../i18n'
import { useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { consumeIntent, useIntent } from '../../blocks/editorToolbar'
import type { BlockDefinition, RenderContext, ToolbarItem } from '../../blocks/registry'
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

function NodeFrame({ context, children }: { context: RenderContext; children?: ReactNode }) {
  const { info, depth, maxText } = useNodeLook(context)
  const t = typography(depth)
  const style = { '--aiws-branch': info?.color ?? undefined, padding: `${NODE_PAD.y}px ${NODE_PAD.x}px`, lineHeight: `${t.lineHeight}px`, fontSize: t.size, fontWeight: t.weight } as CSSProperties
  return (
    <div className={`aiws-mindnode is-depth-${Math.min(depth, 2)}`} style={style} data-testid={`aiws-mindnode-${context.cell.entity_id}`} data-depth={depth}>
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
    <NodeFrame context={context}>
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

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.blocks.noPermission'))

/** The node's tools (§7): add a child / sibling, insert any object as a child, fold, branch colour, side under a
 * two-sided root, back to the layout. */
function nodeToolbar(context: RenderContext, store: ReturnType<typeof useStore>): ToolbarItem[] {
  const id = context.cell.entity_id
  const info = nodeInfo(store.outline, id)
  if (!info) return []
  const items: ToolbarItem[] = []
  const structure = context.cell.capabilities.includes('structure') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.blocks.noPermission')
  items.push({ kind: 'button', id: 'mindmap-add-child', icon: CornerDownRight, label: aiwsT('aiworkspace.mindmap.addChild'), key: 'Tab', disabled: structure, run: () => controlCommands.send({ kind: 'add-child', nodeId: id }) })
  if (info.depth > 0) items.push({ kind: 'button', id: 'mindmap-add-sibling', icon: ListPlus, label: aiwsT('aiworkspace.mindmap.addSibling'), key: 'Enter', disabled: structure, run: () => controlCommands.send({ kind: 'add-sibling', nodeId: id }) })
  items.push({ kind: 'button', id: 'mindmap-insert', icon: SquarePlus, label: aiwsT('aiworkspace.mindmap.insertObject'), disabled: structure, run: () => controlCommands.send({ kind: 'insert', nodeId: id }) })
  if (blockChildren(store.outline, id).length > 0) {
    const folded = foldBook(store).isFolded(info.control.entity_id, id)
    items.push({ kind: 'button', id: 'mindmap-fold', icon: folded ? Plus : Minus, label: aiwsT(folded ? 'aiworkspace.mindmap.unfold' : 'aiworkspace.mindmap.fold'), key: 'Ctrl+/', run: () => foldBook(store).toggle(info.control.entity_id, id) })
  }
  const change = canChange(context)
  if (info.depth > 0) {
    items.push({ kind: 'color', id: 'mindmap-branch', label: aiwsT('aiworkspace.mindmap.branchColor'), value: context.cell.branch_color ?? info.color ?? undefined, palette: 'ink',
      extras: [{ value: 'auto', label: aiwsT('aiworkspace.mindmap.branchAuto') }], disabled: change,
      onPick: (hex) => { void nodeConfigOps(store, id, { branch_color: hex === 'auto' ? null : hex }).then((operations) => store.submit({ editId: `key:${id}:config`, label: aiwsT('aiworkspace.mindmap.branchColor'), operations })) } })
  }
  if (info.depth === 1 && info.config.direction === 'both') {
    items.push({ kind: 'menu', id: 'mindmap-side', label: aiwsT('aiworkspace.mindmap.side'), value: context.cell.side ?? 'auto', disabled: change,
      items: [{ value: 'auto', label: aiwsT('aiworkspace.mindmap.sideAuto') }, { value: 'left', label: aiwsT('aiworkspace.mindmap.sideLeft') }, { value: 'right', label: aiwsT('aiworkspace.mindmap.sideRight') }],
      onPick: (value) => { void nodeConfigOps(store, id, { side: value === 'auto' ? null : value }).then((operations) => store.submit({ editId: `key:${id}:config`, label: aiwsT('aiworkspace.mindmap.side'), operations })) } })
  }
  const p = context.cell.placement
  if (p && (!isAuto(p.x) || !isAuto(p.y))) {
    items.push({ kind: 'button', id: 'mindmap-auto', icon: LocateFixed, label: aiwsT('aiworkspace.mindmap.backToLayout'), disabled: structure,
      run: () => { void store.submit({ editId: `layout:${id}`, label: aiwsT('aiworkspace.mindmap.backToLayout'), operations: resetLayoutOps(store, [id]) }) } })
  }
  return items
}

export const mindnodeDefinition: BlockDefinition = {
  type: NODE_VIEW, version: 1, get title() { return aiwsT('aiworkspace.mindmap.node') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 120, h: 40 }, cost: { editor: false, html: false }, chrome: 'none',
  Static: NodeStatic, Editor: NodeEditor,
  shape: 'rect',
  // a node's look is its depth and branch: no hover label, the text is already there
  hover: () => [],
  toolbar: nodeToolbar,
}
