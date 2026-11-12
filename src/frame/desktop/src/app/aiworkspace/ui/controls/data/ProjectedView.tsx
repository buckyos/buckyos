/* eslint-disable react-refresh/only-export-components -- the projected-Block renderers and their colour helper */
/* Projected Blocks drawn (控件机制 TODO G2, G7): label, shape (svg) and asset (media) through the same art the Blocks
 * draw with, but without a BlockHost each — a span or an svg per node, so a month of events or a long list stays
 * light. Colours may be theme tokens (`accent`, `muted`, `danger` …) as well as `#rrggbb`, so a control follows the
 * light / dark theme. Every node carries its projected id and role for the control's hit testing. */

import { memo, type CSSProperties, type ReactNode } from 'react'
import { readLook } from '@drawio/shapes'
import type { ProjectedNode } from '@controls/projection'
import type { Json } from '../../../api/types'
import { AssetBlobImage } from '../../extensions/AssetBlobImage'
import { formatLabelValue, LabelArt, type LabelLook } from '../../blocks/labelBlock'
import { ShapeArt } from '../../blocks/shapeBlock'

const TOKENS: Record<string, string> = {
  accent: 'var(--cp-accent)', 'accent-soft': 'var(--cp-accent-soft)', text: 'var(--cp-text)', muted: 'var(--cp-muted)', border: 'var(--cp-border)',
  surface: 'var(--cp-surface-opaque, var(--cp-surface))', 'surface-2': 'var(--cp-surface-2)', danger: 'var(--cp-danger)', warning: 'var(--cp-warning)',
  success: 'var(--cp-success)', selected: 'var(--aiws-selected-bg)', 'selected-border': 'var(--aiws-selected-border)', none: 'none',
}

/** A colour of a template: a theme token or `#rrggbb` (either optionally `/<alpha 0–1>`), or `mix:<a>:<b>:<percent of a>`. */
export function paint(value: unknown): string | undefined {
  if (typeof value !== 'string' || !value) return undefined
  if (TOKENS[value]) return TOKENS[value]
  const alpha = /^(#[0-9a-fA-F]{6}|[a-z][a-z0-9-]*)\/(0(?:\.\d+)?|1(?:\.0+)?)$/.exec(value)
  if (alpha) return `color-mix(in srgb, ${TOKENS[alpha[1]] ?? alpha[1]} ${Math.round(Number(alpha[2]) * 100)}%, transparent)`
  const mix = /^mix:([^:]+):([^:]+):(\d{1,3})$/.exec(value)
  if (mix) return `color-mix(in srgb, ${paint(mix[1]) ?? mix[1]} ${mix[3]}%, ${paint(mix[2]) ?? mix[2]})`
  if (/^#[0-9a-fA-F]{6}$/.test(value)) return value
  return undefined
}

/** The text a projected label shows: its value, formatted. */
export function labelText(config: Record<string, unknown>): string {
  if (config.text !== undefined && config.value === undefined) return String(config.text ?? '')
  return formatLabelValue(config.value as Json | undefined, config.format as string | undefined)
}

function ProjectedShape({ config, w, h }: { config: Record<string, unknown>; w: number; h: number }) {
  const look = readLook(config as Record<string, Json>, '')
  const fill = paint(config.fill)
  const stroke = paint(config.stroke)
  // tokens are not draw.io colours: the art takes the CSS value as it is
  const art = { ...look, fill: fill ?? look.fill, stroke: stroke ?? look.stroke, text: '' }
  return <ShapeArt look={art} w={w} h={h} opacity={look.opacity} />
}

function lookOf(config: Record<string, unknown>): LabelLook {
  const look = config as LabelLook
  const color = paint(config.font_color)
  return color ? { ...look, font_color: color } : look
}

export interface ProjectedViewProps {
  node: ProjectedNode
  /** Added to the node's frame (selected, dragging, hovered …). */
  className?: string
  style?: CSSProperties
  children?: ReactNode
}

/** One projected Block, absolutely placed in its control's content coordinates. */
export const ProjectedView = memo(function ProjectedView({ node, className, style, children }: ProjectedViewProps) {
  const { rect, config } = node
  let body: ReactNode = null
  if (node.view === 'label') body = <LabelArt text={labelText(config)} look={lookOf(config)} title={typeof config.tooltip === 'string' ? config.tooltip : undefined} />
  else if (node.view === 'shape') body = <ProjectedShape config={config} w={rect.w} h={rect.h} />
  else if (node.view === 'asset' && typeof config.object_id === 'string') {
    body = <AssetBlobImage objectId={config.object_id} mediaType={typeof config.media_type === 'string' ? config.media_type : 'image/png'} alt={typeof config.alt === 'string' ? config.alt : ''} fit={config.fit === 'contain' ? 'contain' : 'cover'} />
  }
  return (
    <div className={`aiws-proj is-${node.view}${className ? ` ${className}` : ''}`} data-proj={node.id} data-item={node.item} data-node={node.node} data-proj-role={node.role}
      style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h, ...style }}>
      {body}
      {children}
    </div>
  )
})

/** The projected node a pointer is on (from the event target upward), with its gesture role (`body` when it has none). */
export function projectedAt(target: EventTarget | null, root: HTMLElement | null): { id: string; item: string; node: string; role: string } | null {
  let el = target as HTMLElement | null
  while (el && el !== root) {
    if (el.dataset?.proj) return { id: el.dataset.proj, item: el.dataset.item ?? '', node: el.dataset.node ?? '', role: el.dataset.projRole ?? 'body' }
    el = el.parentElement
  }
  return null
}
