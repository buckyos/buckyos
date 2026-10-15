/* Shared hover affordances (标准对象的交互改进 §4.2): the name of the data a Block shows, with its freshness. */

import { Database, Link2, type LucideIcon } from 'lucide-react'
import type { Shape } from '../canvas/connectors/anchors'
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

export function shapeOf(context: RenderContext): Shape {
  const shape = context.definition.shape
  return typeof shape === 'function' ? shape(context.payload) : shape ?? 'rect'
}

/** A Block's link (`config.link`, draw.io 导入 TODO §2.6): a web address, or `#<entity id>` for an object in this
 * Workspace. */
export function openLink(context: RenderContext, link: string) {
  const local = /^#([a-z0-9][a-z0-9_-]{0,63})$/.exec(link)
  if (local) { context.openEntity(local[1]); return }
  if (/^(https?:|mailto:)/i.test(link)) window.open(link, '_blank', 'noopener,noreferrer')
}

/** The hover button of a Block with a link, in edit, view and show mode. */
export function linkAffordance(context: RenderContext): HoverAffordance[] {
  const link = context.payload.config?.link
  if (typeof link !== 'string' || !link) return []
  return [{ id: 'link', kind: 'button', at: 'top-right-in', icon: Link2, label: `打开链接：${link}`, modes: ['edit', 'view', 'show'], run: (c) => openLink(c, link) }]
}
