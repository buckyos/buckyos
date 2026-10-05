/* block_id assignment (design §3.3.2). The backend never repairs ids; this plugin is the only place
 * they are assigned:
 *   - a new block gets a new id;
 *   - a split keeps the id on the first half, the second half gets a new one;
 *   - pasted blocks always get new ids, whatever they carried;
 *   - a join keeps the first block's id (ProseMirror keeps the first node's attrs);
 *   - a missing, malformed or duplicate id is replaced (first occurrence in document order wins).
 * Documents rebuilt from the CRDT (remote imports, undo, initial load) are left alone: repairing them
 * here would answer remote operations with local ones. */

import { Fragment, Slice, type Node as PMNode } from 'prosemirror-model'
import { Plugin, PluginKey, type Transaction } from 'prosemirror-state'
import { loroSyncPluginKey } from 'loro-prosemirror'
import { randomId } from '../api/ids'
import { BLOCK_ID_PATTERN } from './schema'

export const blockIdPluginKey = new PluginKey('aiws-block-id')

export function newBlockId(): string {
  return randomId('b')
}

function stripIds(fragment: Fragment): Fragment {
  const out: PMNode[] = []
  fragment.forEach((node) => {
    const content = stripIds(node.content)
    out.push('block_id' in node.attrs ? node.type.create({ ...node.attrs, block_id: null }, content, node.marks) : node.copy(content))
  })
  return Fragment.fromArray(out)
}

export function blockIdPlugin(): Plugin {
  return new Plugin({
    key: blockIdPluginKey,
    props: {
      transformPasted: (slice) => new Slice(stripIds(slice.content), slice.openStart, slice.openEnd),
    },
    appendTransaction(transactions, _oldState, newState) {
      const changed = transactions.filter((tr) => tr.docChanged)
      if (changed.length === 0) return null
      if (changed.every((tr) => tr.getMeta(loroSyncPluginKey) !== undefined)) return null
      const seen = new Set<string>()
      let fix: Transaction | null = null
      newState.doc.descendants((node, pos) => {
        if ('block_id' in node.attrs) {
          const id: unknown = node.attrs.block_id
          if (typeof id === 'string' && BLOCK_ID_PATTERN.test(id) && !seen.has(id)) {
            seen.add(id)
          } else {
            const fresh = newBlockId()
            seen.add(fresh)
            fix = (fix ?? newState.tr).setNodeMarkup(pos, undefined, { ...node.attrs, block_id: fresh })
          }
        }
        return !node.isTextblock && !node.isAtom
      })
      return fix
    },
  })
}

/** The innermost block with a block_id around the selection head. */
export function blockIdAt(doc: PMNode, pos: number): string | null {
  const resolved = doc.resolve(Math.min(Math.max(pos, 0), doc.content.size))
  for (let depth = resolved.depth; depth > 0; depth--) {
    const id: unknown = resolved.node(depth).attrs.block_id
    if (typeof id === 'string') return id
  }
  const after = resolved.nodeAfter
  return after && typeof after.attrs.block_id === 'string' ? after.attrs.block_id : null
}
