/* eslint-disable react-refresh/only-export-components -- a Block definition with its renderer */
/* The `ink` Block (手写笔 TODO D3): one segment of a pen stroke, drawn as SVG in its box. A pure UI Block: no data,
 * no editor, never a low-detail placeholder; the canvas mounts it wherever it crosses the view. The pieces and widths
 * are those of the live layer (model.ts), so the stroke does not change when its Block takes over; zoomed out, a long
 * stroke draws fewer points. Once drawn it tells the workspace's ink book (hit tests read its points there, and a
 * stroke waiting for its Block stops being drawn by the live layer). */

import { useLayoutEffect, useMemo } from 'react'
import { aiwsT } from '../i18n'
import { useAiwsText } from '../useAiwsText'
import { useStore } from '../state/hooks'
import type { BlockDefinition, RenderContext } from '../ui/blocks/registry'
import { inkBook } from './book'
import { boxPoints, displayPoints, INK_VIEW, pieces, readInk, runPath, runs } from './model'

function InkStatic(context: RenderContext) {
  const store = useStore()
  const { t } = useAiwsText()
  const book = inkBook(store)
  const config = context.payload.config
  const ink = useMemo(() => readInk(config), [config])
  const { w, h } = context.size
  // zoomed out, fewer points: one level per halving of the zoom
  const level = context.zoom >= 1 ? 1 : Math.pow(2, Math.floor(Math.log2(Math.max(context.zoom, 0.01))))
  const paths = useMemo(() => (ink ? runs(pieces(displayPoints(boxPoints(ink, { w, h }), level))).map((run) => ({ d: runPath(run), w: run.w })) : []), [ink, w, h, level])
  const id = context.cell.entity_id
  const rev = context.cell.content_rev
  useLayoutEffect(() => {
    if (!ink) return
    book.put(id, rev, ink)
    book.rendered(id)
  }, [book, id, rev, ink])
  if (!ink) return <div className="aiws-ink-unreadable" role="note" data-testid={`aiws-ink-unreadable-${id}`}>{t('aiworkspace.blocks.ink.unreadable')}</div>
  return (
    <svg className="aiws-ink" data-testid={`aiws-ink-${id}`} data-stroke={ink.stroke} data-seg={ink.seg} data-points={ink.points.length} data-paths={paths.length}
      viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden="true">
      {paths.map((p, i) => <path key={i} d={p.d} stroke={ink.style.color} strokeWidth={p.w} />)}
    </svg>
  )
}

export const inkDefinition: BlockDefinition = {
  type: INK_VIEW, version: 1, get title() { return aiwsT('aiworkspace.blocks.ink.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 120, h: 40 }, cost: { editor: false, html: false }, Static: InkStatic, chrome: 'none',
  // corner handles keep the handwriting's proportions (Shift frees them)
  resize: { aspect: 'locked' },
}
