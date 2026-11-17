/* A PDF Frame drawn at a given width, scaled as a whole (打印 TODO §3.3): its header and footer with their Blocks (each
 * through BlockHost, cut to its band), the content area left empty. Printing draws it on every page (`PdfFrameDecoration`,
 * with that page's context); a flow page shows it as a preview with the page and the bands outlined. It reads only the
 * store, so it runs wherever a store is provided (the print renderer included). */

import type { ReactNode } from 'react'
import { num } from '@controls/placement'
import type { PageContext } from '@print/page.ts'
import type { EntityEnvelope } from '../../../api/types'
import { useOutlineVersion, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { BlockHost } from '../../blocks/BlockHost'
import { blockRegistry, type CanvasMode } from '../../blocks/registry'
import { PageContextValue } from './context'
import { bandsOf, frameLayout, isFrameEntity } from './model'
import './pdfframe.css'

/** Blocks under `parentId` at their places (relative to it): Cells through BlockHost, groups with their Blocks. Lines and
 * controls are not drawn (they need the canvas). */
function Blocks({ parentId, mode }: { parentId: string; mode: CanvasMode }): ReactNode {
  const store = useStore()
  return store.outline.childrenOf(parentId).map((e) => <Placed key={e.entity_id} entity={e} mode={mode} />)
}

function Placed({ entity, mode }: { entity: EntityEnvelope; mode: CanvasMode }) {
  const p = entity.placement
  const style = { left: num(p?.x) ?? 0, top: num(p?.y) ?? 0, width: num(p?.w) ?? 0, height: num(p?.h) ?? 0, rotate: p?.rotation ? `${p.rotation}deg` : undefined }
  if (entity.type_id === 'buckyos.container') {
    if (entity.kind !== 'group') return null
    return <div className="aiws-pdfframe-group" style={style}><Blocks parentId={entity.entity_id} mode={mode} /></div>
  }
  if (entity.type_id !== 'buckyos.cell' || entity.view_type === 'connector') return null
  return (
    <div className="aiws-frame-block" data-block-id={entity.entity_id} data-view={entity.view_type ?? undefined}
      data-chrome={blockRegistry.get(entity.view_type ?? '', entity.view_version ?? undefined)?.chrome ?? 'clip'} style={style}>
      <BlockHost cellId={entity.entity_id} mode={mode} view="canvas" size={{ w: style.width, h: style.height }} />
    </div>
  )
}

export interface FrameArtProps {
  frameId: string
  widthPx: number
  /** Default: the frame's own shape at that width. */
  heightPx?: number
  mode: CanvasMode
  /** The page's values; null: the samples of an edited frame. */
  context: PageContext | null
  /** Outline the page, the bands and the content area (a preview). */
  outlined?: boolean
  testId?: string
}

export function FrameArt({ frameId, widthPx, heightPx, mode, context, outlined = false, testId }: FrameArtProps) {
  const store = useStore()
  useOutlineVersion()
  const frame = store.outline.get(frameId)
  if (!frame || !isFrameEntity(frame)) return null
  const { box, areas } = frameLayout(frame)
  const scale = widthPx / box.w
  const bands = bandsOf(store.outline, frameId)
  const at = (r: { x: number; y: number; w: number; h: number }) => ({ left: r.x, top: r.y, width: Math.max(0, r.w), height: Math.max(0, r.h) })
  return (
    <PageContextValue.Provider value={context}>
      <div className={`aiws-pdfframe-art${outlined ? ' is-outlined' : ''}`} data-testid={testId} data-frame={frameId}
        style={{ width: widthPx, height: heightPx ?? box.h * scale }}>
        <div className="aiws-pdfframe-art-page" style={{ width: box.w, height: box.h, transform: `scale(${scale})` }}>
          {outlined && <div className="aiws-pdfframe-content" style={at(areas.content)} />}
          {(['header', 'footer'] as const).map((band) => {
            const entity = bands[band]
            return entity ? (
              <div key={band} className={`aiws-pdfframe-art-band is-${band}`} data-band={band} style={at(areas[band])}>
                <Blocks parentId={entity.entity_id} mode={mode} />
              </div>
            ) : null
          })}
        </div>
      </div>
    </PageContextValue.Provider>
  )
}

/** On a flow page: a preview of the frame (edited on a free canvas). */
export function PdfFrameFlow({ containerId }: { containerId: string }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-pdfframe-flow" data-testid={`aiws-pdfframe-flow-${containerId}`}>
      <FrameArt frameId={containerId} widthPx={240} mode="view" context={null} outlined />
      <span className="aiws-muted">{t('aiworkspace.pdfframe.flowHint')}</span>
    </div>
  )
}
