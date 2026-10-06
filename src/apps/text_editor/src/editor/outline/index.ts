import { syntaxTree } from '@codemirror/language'
import type { EditorState } from '@codemirror/state'
export interface OutlineItem { title: string; level: number; from: number }
export function extractOutline(state: EditorState): OutlineItem[] {
  const result: OutlineItem[] = []
  syntaxTree(state).iterate({ enter(node) {
    const heading = /^(?:ATX|Setext)Heading([1-6])$/.exec(node.name)
    if (heading) {
      const raw = state.doc.sliceString(node.from, node.to)
      result.push({ title: raw.split('\n')[0].replace(/^#{1,6}\s+|\s+#+$/g, '').trim(), level: Number(heading[1]), from: node.from })
      return false
    }
    if (node.name === 'Element') {
      const opening = node.node.getChild('OpenTag')
      if (!opening) return
      const tag = state.doc.sliceString(opening.from, opening.to)
      const match = /^<h([1-6])(?:\s|>)/i.exec(tag)
      if (!match) return
      const closing = node.node.getChild('CloseTag')
      const title = state.doc.sliceString(opening.to, closing?.from ?? node.to).replace(/<[^>]*>/g, '').trim()
      result.push({ title, level: Number(match[1]), from: node.from }); return false
    }
  } })
  return result
}
