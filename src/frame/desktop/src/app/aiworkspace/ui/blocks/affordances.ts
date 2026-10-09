/* Shared hover affordances (标准对象的交互改进 §4.2): the name of the data a Block shows, with its freshness. */

import { Database, type LucideIcon } from 'lucide-react'
import type { HoverAffordance, RenderContext } from './registry'

/** `top-left-out`: type icon + the data's name (+ freshness of a generated result), in edit and view mode. */
export function sourceLabel(context: RenderContext, icon: LucideIcon = Database): HoverAffordance[] {
  const source = context.source
  if (!source) return []
  return [{
    id: 'name', kind: 'label', at: 'top-left-out', icon, modes: ['edit', 'view'],
    label: source.title ?? source.name ?? context.payload.title ?? context.definition.title,
    ...(source.derived ? { freshness: source.entity_id } : {}),
  }]
}

export function shapeOf(context: RenderContext): 'rect' | 'ellipse' {
  const shape = context.definition.shape
  return typeof shape === 'function' ? shape(context.payload) : shape ?? 'rect'
}
