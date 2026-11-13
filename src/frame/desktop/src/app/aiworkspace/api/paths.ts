/* The one path syntax (核心 API 调整 R5, R14; core `path.rs`) over the outline this tab holds: `/` is the root, each
 * step an entity's `name` (`/data/销售`, `/surfaces/看板/text-3`), and a path may start at an entity with `~<entity_id>`.
 * The outline holds only what the caller may read, so this resolves exactly as the service does for this caller: of
 * siblings sharing a name the shared one wins, then sibling order; the others are reached by `~<id>`. */

import type { EntityEnvelope } from './types'

export interface OutlineLike {
  get(id: string): EntityEnvelope | undefined
  childrenOf(parentId: string): readonly EntityEnvelope[]
}

const ROOT = 'root'

/** The child of `parent` a name step reaches. */
function step(outline: OutlineLike, parent: string, name: string): EntityEnvelope | undefined {
  const named = outline.childrenOf(parent).filter((c) => !c.deleted && c.name === name)
  return named.find((c) => c.scope === 'shared') ?? named[0]
}

/** The entity a path names, or null. Anything not starting with `/` or `~` is an entity id. */
export function resolvePath(outline: OutlineLike, path: string): EntityEnvelope | null {
  if (!path.startsWith('/') && !path.startsWith('~')) {
    const e = outline.get(path)
    return e && !e.deleted ? e : null
  }
  const segs = path.split('/').filter((s) => s.length > 0)
  let cur: EntityEnvelope | undefined
  if (segs[0]?.startsWith('~')) {
    cur = outline.get(segs.shift()!.slice(1))
    if (!cur || cur.deleted) return null
  } else {
    cur = outline.get(ROOT)
    if (!cur) return null
  }
  for (const s of segs) {
    if (s === '.') continue
    cur = step(outline, cur.entity_id, s)
    if (!cur) return null
  }
  return cur
}

/** The path of an entity as this caller prints it: from the root, or from the highest ancestor it can reach by names
 * as `~<id>/…`. */
export function pathOf(outline: OutlineLike, id: string): string {
  if (id === ROOT) return '/'
  const segs: string[] = []
  let cur = outline.get(id)
  for (let i = 0; cur && i < 4096; i++) {
    const parentId = cur.parent_id
    const parent = parentId ? outline.get(parentId) : undefined
    if (!parent || !cur.name || step(outline, parent.entity_id, cur.name)?.entity_id !== cur.entity_id) {
      return `~${cur.entity_id}${segs.length ? `/${segs.reverse().join('/')}` : ''}`
    }
    segs.push(cur.name)
    if (parent.entity_id === ROOT) break
    cur = parent
  }
  return `/${segs.reverse().join('/')}`
}
