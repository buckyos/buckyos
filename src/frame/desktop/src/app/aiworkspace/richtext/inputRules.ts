/* Markdown shortcuts while typing (流式页交互改进 §3.2): what Notion does with the same keys.
 *
 *   `# ` `## ` `### `  heading            `- ` `* ` `+ `  bullet list       `1. ` `1) `  ordered list
 *   `> `                quote              ``` `` + space / Enter → code block  `---`  divider
 *   `**a**` `__a__` strong   `*a*` `_a_` em   `~~a~~` strike   `` `a` `` code
 *
 * A rule fires on the character that completes it; Backspace right after undoes it and leaves the typed text
 * (the same contract as prosemirror-inputrules, written here to keep the dependency list as it is). Rules never
 * fire inside code. */

import type { MarkType, Node as PMNode, NodeType, Schema } from 'prosemirror-model'
import { wrapRangeInList } from 'prosemirror-schema-list'
import { Plugin, PluginKey, TextSelection, type Command, type EditorState, type Transaction } from 'prosemirror-state'
import { liftTarget } from 'prosemirror-transform'
import type { EditorView } from 'prosemirror-view'

interface Undoable { transform: Transaction; from: number; to: number; text: string }

const key = new PluginKey<Undoable | null>('aiws-input-rules')

/** The text of the textblock before the caret, atoms as one placeholder character each (offsets stay aligned). */
function textBefore(state: EditorState, pos: number): string {
  const $pos = state.doc.resolve(pos)
  return $pos.parent.textBetween(0, $pos.parentOffset, undefined, '￼')
}

function inCode(state: EditorState, pos: number): boolean {
  const $pos = state.doc.resolve(pos)
  if ($pos.parent.type.spec.code) return true
  const code = state.schema.marks.code
  return Boolean(code && code.isInSet($pos.marks()))
}

/** Replace the typed prefix of the textblock and turn the block into `type` (or wrap it). */
function blockRule(view: EditorView, from: number, to: number, typed: string, apply: (tr: Transaction, start: number) => Transaction | null): boolean {
  const state = view.state
  const $from = state.doc.resolve(from)
  const start = from - $from.parentOffset
  const tr = state.tr.delete(start, to)
  const result = apply(tr, start)
  if (!result) return false
  view.dispatch(result.setMeta(key, { transform: result, from, to, text: typed }).scrollIntoView())
  return true
}

function setTextblock(type: NodeType, attrs: Record<string, unknown>) {
  return (tr: Transaction, start: number): Transaction | null => {
    const $start = tr.doc.resolve(start)
    const node = $start.parent
    if (!node.isTextblock || !$start.node(-1).canReplaceWith($start.index(-1), $start.indexAfter(-1), type)) return null
    return tr.setBlockType(start, start, type, { ...attrs, block_id: node.attrs.block_id })
  }
}

function wrapList(type: NodeType, attrs: Record<string, unknown> = {}) {
  return (tr: Transaction, start: number): Transaction | null => {
    const range = tr.doc.resolve(start).blockRange()
    return range && wrapRangeInList(tr, range, type, attrs) ? tr : null
  }
}

function wrapQuote(type: NodeType) {
  return (tr: Transaction, start: number): Transaction | null => {
    const $start = tr.doc.resolve(start)
    const range = $start.blockRange()
    if (!range) return null
    const wrapping = range.$from.node(range.depth).canReplaceWith(range.startIndex, range.endIndex, type) ? [{ type }] : null
    return wrapping ? tr.wrap(range, wrapping) : null
  }
}

/** A divider replaces the paragraph; the caret goes to a new empty paragraph after it. */
function divider(schema: Schema) {
  return (tr: Transaction, start: number): Transaction | null => {
    const $start = tr.doc.resolve(start)
    if ($start.parent.type !== schema.nodes.paragraph || $start.parent.content.size !== 0 || $start.depth !== 1) return null
    const before = $start.before()
    const after = $start.after()
    tr.replaceWith(before, after, [schema.nodes.horizontal_rule.create(), schema.nodes.paragraph.create()])
    return tr.setSelection(TextSelection.create(tr.doc, before + 2))
  }
}

/** `(prefix)(open)(inner)(close)` at the end of the text before the caret, `close` just typed → `mark` on inner. */
function markRule(view: EditorView, from: number, to: number, typed: string, before: string, pattern: RegExp, mark: MarkType): boolean {
  const match = pattern.exec(before + typed)
  if (!match) return false
  const [whole, lead, open, inner] = match
  if (!inner || inner.trim() !== inner) return false
  const state = view.state
  const $from = state.doc.resolve(from)
  const blockStart = from - $from.parentOffset
  const matchStart = blockStart + match.index + lead.length
  const innerStart = matchStart + open.length
  const innerEnd = innerStart + inner.length
  // the closing delimiter minus the typed character is already in the document
  const closeInDoc = whole.length - lead.length - open.length - inner.length - typed.length
  if (closeInDoc < 0) return false
  const tr = state.tr
  if (to > from) tr.delete(from, to)
  tr.delete(innerEnd, innerEnd + closeInDoc)
  tr.delete(matchStart, innerStart)
  tr.addMark(matchStart, matchStart + inner.length, mark.create())
  tr.removeStoredMark(mark)
  view.dispatch(tr.setMeta(key, { transform: tr, from, to, text: typed }))
  return true
}

export function inputRulesPlugin(schema: Schema): Plugin {
  const { heading, bullet_list: bullet, ordered_list: ordered, blockquote, code_block: code, paragraph } = schema.nodes
  const marks = schema.marks
  return new Plugin<Undoable | null>({
    key,
    state: {
      init: () => null,
      apply: (tr, previous) => {
        const meta = tr.getMeta(key) as Undoable | undefined
        if (meta) return meta
        // block ids given to the nodes a rule created arrive as an appended transaction: the rule stays undoable
        if (tr.getMeta('appendedTransaction')) return previous
        return tr.selectionSet || tr.docChanged ? null : previous
      },
    },
    props: {
      handleTextInput(view, from, to, text) {
        if (view.composing || inCode(view.state, from)) return false
        const state = view.state
        const $from = state.doc.resolve(from)
        if (!$from.parent.isTextblock) return false
        const before = textBefore(state, from)
        const line = before + text
        // block shortcuts: only at the very start of a paragraph
        if ($from.parent.type === paragraph && to === from) {
          const rule = (apply: (tr: Transaction, start: number) => Transaction | null) => blockRule(view, from, to, text, apply)
          const heading_ = /^(#{1,3}) $/.exec(line)
          if (heading_) return rule(setTextblock(heading, { level: heading_[1].length }))
          if (/^[-*+] $/.test(line)) return rule(wrapList(bullet))
          const ordered_ = /^(\d{1,4})[.)] $/.exec(line)
          if (ordered_) return rule(wrapList(ordered, { start: Math.max(1, Number(ordered_[1])) }))
          if (/^> $/.test(line)) return rule(wrapQuote(blockquote))
          const fence = /^```([\w+-]*) $/.exec(line)
          if (fence) return rule(setTextblock(code, { language: fence[1] }))
          if (/^(---|\*\*\*|___)$/.test(line) && $from.parentOffset === $from.parent.content.size) return rule(divider(schema))
        }
        if (text.length !== 1) return false
        if (text === '*' && markRule(view, from, to, text, before, /(^|[^*])(\*\*)([^*]+)\*\*$/, marks.strong)) return true
        if (text === '_' && markRule(view, from, to, text, before, /(^|[^_\w])(__)([^_]+)__$/, marks.strong)) return true
        if (text === '*' && markRule(view, from, to, text, before, /(^|[^*])(\*)([^*]+)\*$/, marks.em)) return true
        if (text === '_' && markRule(view, from, to, text, before, /(^|[^_\w])(_)([^_]+)_$/, marks.em)) return true
        if (text === '~' && markRule(view, from, to, text, before, /(^|[^~])(~~)([^~]+)~~$/, marks.strike)) return true
        if (text === '`' && markRule(view, from, to, text, before, /(^|[^`])(`)([^`]+)`$/, marks.code)) return true
        return false
      },
    },
  })
}

/** Enter on a paragraph that reads ```lang: a code block (the form people type before pressing Enter). */
export function codeFenceOnEnter(schema: Schema): Command {
  return (state, dispatch) => {
    const { $from, empty } = state.selection
    if (!empty || $from.parent.type !== schema.nodes.paragraph) return false
    const m = /^```([\w+-]*)$/.exec($from.parent.textContent)
    if (!m || $from.parentOffset !== $from.parent.content.size) return false
    if (!$from.node(-1).canReplaceWith($from.index(-1), $from.indexAfter(-1), schema.nodes.code_block)) return false
    if (dispatch) {
      const start = $from.start()
      const tr = state.tr.delete(start, $from.end())
      tr.setBlockType(start, start, schema.nodes.code_block, { language: m[1], block_id: $from.parent.attrs.block_id })
      dispatch(tr.scrollIntoView())
    }
    return true
  }
}

/** Backspace right after a rule: undo it and keep what was typed. */
export const undoInputRule: Command = (state, dispatch) => {
  const undoable = key.getState(state)
  if (!undoable) return false
  if (dispatch) {
    const tr = state.tr
    const toUndo = undoable.transform
    for (let i = toUndo.steps.length - 1; i >= 0; i--) tr.step(toUndo.steps[i].invert(toUndo.docs[i]))
    if (undoable.text) {
      const marks = tr.doc.resolve(undoable.from).marks()
      tr.replaceWith(undoable.from, undoable.to, state.schema.text(undoable.text, marks))
    } else {
      tr.delete(undoable.from, undoable.to)
    }
    dispatch(tr)
  }
  return true
}

/** Backspace at the start of a heading, quote, list item or code block makes it plain text first (Notion). */
export function plainOnBackspace(schema: Schema, lift: Command): Command {
  return (state, dispatch, view) => {
    const { $from, empty } = state.selection
    if (!empty || $from.parentOffset !== 0 || !$from.parent.isTextblock) return false
    const parent = $from.parent
    if (parent.type === schema.nodes.heading || parent.type === schema.nodes.code_block) {
      if (dispatch) dispatch(state.tr.setBlockType($from.pos, $from.pos, schema.nodes.paragraph, { block_id: parent.attrs.block_id }))
      return true
    }
    for (let depth = $from.depth - 1; depth > 0; depth--) {
      const node: PMNode = $from.node(depth)
      // only the first textblock of the wrapper: further in, Backspace joins as usual
      if ($from.index(depth) !== 0) return false
      if (node.type === schema.nodes.list_item) return lift(state, dispatch, view)
      if (node.type === schema.nodes.blockquote) {
        const range = $from.blockRange()
        const target = range ? liftTarget(range) : null
        if (!range || target === null) return false
        if (dispatch) dispatch(state.tr.lift(range, target))
        return true
      }
    }
    return false
  }
}
