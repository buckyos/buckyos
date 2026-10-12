/* The `/` command menu (流式页交互改进 §3.3): typing `/` at the start of a line or after a space opens a menu at
 * the caret; what follows the slash filters it; ↑ ↓ choose, Enter / Tab run, Esc or moving away closes it and
 * leaves the typed text as it is. The plugin only tracks where the command is; the menu is drawn by the editor. */

import { Plugin, PluginKey, type EditorState } from 'prosemirror-state'

export interface SlashState { from: number; query: string }

export const slashKey = new PluginKey<SlashState | null>('aiws-slash')

type Meta = { open: number } | { close: true }

/** Keys the open menu takes; `true` when it used the key. */
export type SlashKey = 'up' | 'down' | 'enter' | 'escape'

function check(state: EditorState, from: number): SlashState | null {
  const { selection, doc } = state
  if (!selection.empty || from < 0 || from >= doc.content.size) return null
  const head = selection.head
  if (head <= from || head - from > 40) return null
  const $from = doc.resolve(from)
  const $head = doc.resolve(head)
  if ($from.parent !== $head.parent) return null
  const text = doc.textBetween(from, head, undefined, '￼')
  if (!text.startsWith('/') || text.includes('￼') || /\s{2}/.test(text)) return null
  return { from, query: text.slice(1) }
}

export function slashPlugin(onKey: (key: SlashKey) => boolean): Plugin {
  return new Plugin<SlashState | null>({
    key: slashKey,
    state: {
      init: () => null,
      apply(tr, previous, _old, next) {
        const meta = tr.getMeta(slashKey) as Meta | undefined
        if (meta && 'close' in meta) return null
        if (meta && 'open' in meta) return check(next, meta.open)
        if (!previous) return null
        if (!tr.docChanged && !tr.selectionSet) return previous
        return check(next, tr.mapping.map(previous.from, -1))
      },
    },
    props: {
      handleTextInput(view, from, to, text) {
        if (text !== '/' || view.composing) return false
        const state = view.state
        const $from = state.doc.resolve(from)
        if (!$from.parent.isTextblock || $from.parent.type.spec.code) return false
        const before = $from.parent.textBetween(Math.max(0, $from.parentOffset - 1), $from.parentOffset, undefined, '￼')
        // after a Latin letter, a digit or URL punctuation a slash is text (a/b, http://); elsewhere (start, space, CJK) it is a command
        if (/[A-Za-z0-9/:.]/.test(before)) return false
        view.dispatch(state.tr.insertText('/', from, to).setMeta(slashKey, { open: from }))
        return true
      },
      handleKeyDown(view, event) {
        if (!slashKey.getState(view.state)) return false
        const key: SlashKey | null = event.key === 'ArrowDown' ? 'down' : event.key === 'ArrowUp' ? 'up'
          : event.key === 'Enter' || event.key === 'Tab' ? 'enter' : event.key === 'Escape' ? 'escape' : null
        if (!key || event.isComposing) return false
        if (key === 'escape') { view.dispatch(view.state.tr.setMeta(slashKey, { close: true })); return true }
        return onKey(key)
      },
      handleDOMEvents: {
        blur: (view) => {
          // leaving the editor (a click elsewhere) closes the menu; picking an item keeps the focus in the editor
          if (slashKey.getState(view.state)) window.setTimeout(() => { if (!view.isDestroyed && !view.hasFocus() && slashKey.getState(view.state)) view.dispatch(view.state.tr.setMeta(slashKey, { close: true })) }, 150)
          return false
        },
      },
    },
  })
}

/** Close the menu (after an item ran, or from outside). */
export function closeSlash(state: EditorState) {
  return state.tr.setMeta(slashKey, { close: true })
}
