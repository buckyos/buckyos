/* Placeholders of an empty rich text (流式页交互改进 §3.1): a hint in an empty document, and in the empty line
 * holding the caret (shown only while the editor has the focus, by CSS on `.ProseMirror-focused`). */

import { Plugin } from 'prosemirror-state'
import { Decoration, DecorationSet } from 'prosemirror-view'
import { aiwsT } from '../i18n'

export function placeholderPlugin(): Plugin {
  return new Plugin({
    props: {
      decorations(state) {
        const doc = state.doc
        const decorations: Decoration[] = []
        const only = doc.childCount === 1 ? doc.firstChild : null
        if (only && only.isTextblock && only.content.size === 0 && only.type.name === 'paragraph') {
          decorations.push(Decoration.node(0, only.nodeSize, { class: 'aiws-empty-doc', 'data-placeholder': aiwsT('aiworkspace.richtext.placeholder.emptyDoc') }))
          return DecorationSet.create(doc, decorations)
        }
        const { $from, empty } = state.selection
        const parent = $from.parent
        if (empty && parent.isTextblock && parent.content.size === 0 && !parent.type.spec.code) {
          const hint = parent.type.name === 'heading' ? aiwsT('aiworkspace.richtext.block.heading', { level: String(parent.attrs.level) })
            : $from.depth > 1 && $from.node($from.depth - 1).type.name === 'list_item' ? aiwsT('aiworkspace.richtext.block.list')
              : $from.depth > 1 && $from.node($from.depth - 1).type.name === 'blockquote' ? aiwsT('aiworkspace.richtext.block.quote')
                : aiwsT('aiworkspace.richtext.placeholder.emptyLine')
          decorations.push(Decoration.node($from.before(), $from.after(), { class: 'aiws-caret-empty', 'data-placeholder': hint }))
        }
        return decorations.length ? DecorationSet.create(doc, decorations) : null
      },
    },
  })
}
