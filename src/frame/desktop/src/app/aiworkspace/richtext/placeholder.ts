/* Placeholders of an empty rich text (流式页交互改进 §3.1): a hint in an empty document, and in the empty line
 * holding the caret (shown only while the editor has the focus, by CSS on `.ProseMirror-focused`). */

import { Plugin } from 'prosemirror-state'
import { Decoration, DecorationSet } from 'prosemirror-view'

export const EMPTY_DOC_HINT = '输入文字，或输入“/”选择格式'

export function placeholderPlugin(): Plugin {
  return new Plugin({
    props: {
      decorations(state) {
        const doc = state.doc
        const decorations: Decoration[] = []
        const only = doc.childCount === 1 ? doc.firstChild : null
        if (only && only.isTextblock && only.content.size === 0 && only.type.name === 'paragraph') {
          decorations.push(Decoration.node(0, only.nodeSize, { class: 'aiws-empty-doc', 'data-placeholder': EMPTY_DOC_HINT }))
          return DecorationSet.create(doc, decorations)
        }
        const { $from, empty } = state.selection
        const parent = $from.parent
        if (empty && parent.isTextblock && parent.content.size === 0 && !parent.type.spec.code) {
          const hint = parent.type.name === 'heading' ? `标题 ${String(parent.attrs.level)}`
            : $from.depth > 1 && $from.node($from.depth - 1).type.name === 'list_item' ? '列表'
              : $from.depth > 1 && $from.node($from.depth - 1).type.name === 'blockquote' ? '引用'
                : '输入“/”选择格式'
          decorations.push(Decoration.node($from.before(), $from.after(), { class: 'aiws-caret-empty', 'data-placeholder': hint }))
        }
        return decorations.length ? DecorationSet.create(doc, decorations) : null
      },
    },
  })
}
