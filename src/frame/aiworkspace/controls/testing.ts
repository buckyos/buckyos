/* Test support: a commit sequence (fixtures/mindmap/commits.json) replayed into an outline a control reads. Only the
 * operations the fixtures use (entity.create, tree.place, tree.move) and the outline fields the core projects. */

import type { ControlEntity, ControlSource } from './solve.ts'

interface Entity extends ControlEntity { parent_id: string; order_key: string }

export function replay(commits: { operations: Record<string, unknown>[] }[]): ControlSource & { all(): Entity[] } {
  const by = new Map<string, Entity>()
  for (const c of commits) for (const op of c.operations) {
    const id = op.entity_id as string
    if (op.op === 'entity.create') {
      const p = (op.payload ?? {}) as Record<string, unknown>
      const config = p.config as Record<string, unknown> | undefined
      by.set(id, {
        entity_id: id, type_id: op.type_id as string, parent_id: op.parent_id as string, order_key: op.order_key as string,
        kind: (p.kind as string) ?? null, view_type: (p.view as { type?: string } | undefined)?.type ?? null, title: (p.title as string) ?? null,
        placement: (op.placement as ControlEntity['placement']) ?? null,
        side: typeof config?.side === 'string' ? config.side : null, branch_color: typeof config?.branch_color === 'string' ? config.branch_color : null,
        config: op.type_id === 'buckyos.container' ? config : undefined,
      })
    } else if (op.op === 'tree.place' || op.op === 'tree.move') {
      const e = by.get(id)
      if (!e) continue
      if ('placement' in op) e.placement = op.placement as ControlEntity['placement']
      if (typeof op.order_key === 'string') e.order_key = op.order_key
      if (typeof op.new_parent_id === 'string') e.parent_id = op.new_parent_id
    }
  }
  const all = () => [...by.values()]
  return {
    all,
    get: (id) => by.get(id),
    childrenOf: (id) => all().filter((e) => e.parent_id === id).sort((a, b) => (a.order_key < b.order_key ? -1 : a.order_key > b.order_key ? 1 : a.entity_id < b.entity_id ? -1 : 1)),
  }
}
