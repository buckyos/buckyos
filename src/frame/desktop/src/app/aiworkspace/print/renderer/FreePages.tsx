/* A free canvas printed along its presentation path (打印 TODO §3.2, T1): one page per playable step, as a show plays it.
 * The step's rectangle is fitted into the page's content area (`fitStep`), centred, never cropped; around it the path's
 * background (a Frame step: its own background inside), Frames drawn without border or title, the step's hidden Blocks
 * left out, a turned Frame upright. Steps on other canvases print like the rest. Each page is one RenderHost in `print`
 * mode with a camera that never moves. */

import { useMemo, useState } from 'react'
import { contentArea, fitStep, PX_PER_MM, type PageContext, type Rect } from '@print/page.ts'
import type { PrintJob } from '@print/package.ts'
import { DEFAULT_BACKGROUND, type ResolvedStep } from '../../presentation/model'
import type { Laid } from '../../ui/canvas/layout'
import { Camera } from '../../ui/canvas/render/camera'
import { RenderHost } from '../../ui/canvas/render/RenderHost'
import { useSurfaceLayout } from '../../ui/canvas/useSurfaceLayout'
import { partOfPdfFrame, type PdfFrameInfo } from '../../ui/controls/pdfframe/api'
import type { OutlineModel } from '../../state/outline'
import { useStore } from '../../state/hooks'
import { PrintPage } from './PrintPage'

const NO_LAYOUT = () => false
const NONE = new Set<string>()

export interface FreeStep {
  step: ResolvedStep
  /** A Frame step's own background (`presentation.background`). */
  background: string | null
}

/** What a page leaves out on its Surface: Frames (editing aids, §4.2 of the show), PDF Frames with what is in them. */
function hiddenOf(laid: Map<string, Laid>, outline: OutlineModel): Set<string> {
  const out = new Set<string>()
  for (const [id, l] of laid) if (l.entity.view_type === 'frame' || partOfPdfFrame(outline, id)) out.add(id)
  return out
}

function FreePage({ item, job, frame, context, pathBackground }: { item: FreeStep; job: PrintJob; frame: PdfFrameInfo | null; context: PageContext; pathBackground: string }) {
  const { step } = item
  const store = useStore()
  const rect = step.rect!
  const surfaceId = step.surfaceId!
  const { laid, index } = useSurfaceLayout(surfaceId)
  const areaMm = contentArea(job.paper, 'free', frame?.geometry)
  const area: Rect = { x: areaMm.x * PX_PER_MM, y: areaMm.y * PX_PER_MM, w: areaMm.w * PX_PER_MM, h: areaMm.h * PX_PER_MM }
  const fit = fitStep(rect, area)
  const [camera] = useState(() => {
    const c = new Camera()
    c.set({ x: rect.x - fit.x / fit.scale, y: rect.y - fit.y / fit.scale, zoom: fit.scale })
    return c
  })
  // the camera keeps its zoom within its limits: the step lands where that zoom puts it
  const hole = { x: (rect.x - camera.x) * camera.zoom, y: (rect.y - camera.y) * camera.zoom, w: rect.w * camera.zoom, h: rect.h * camera.zoom }
  const rectKey = `${rect.x},${rect.y},${rect.w},${rect.h}`
  // a PDF Frame on the canvas is not page content: not even mounted
  const shown = useMemo(() => new Set(index.query(rect).map((item) => item.id).filter((id) => !partOfPdfFrame(store.outline, id))),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- rectKey is the rectangle
    [index, store, rectKey])
  const hideKey = (step.step.hide ?? []).join(',')
  const hidden = useMemo(() => new Set([...hiddenOf(laid, store.outline), ...(hideKey ? hideKey.split(',') : [])]), [laid, store, hideKey])
  const W = job.paper.width_mm * PX_PER_MM
  const H = job.paper.height_mm * PX_PER_MM
  const inside = step.kind === 'frame' ? item.background ?? pathBackground : pathBackground
  return (
    <PrintPage paper={job.paper} frame={frame} context={context} background={inside}>
      <div className="aiws-print-free" style={{ ['--aiws-print-letterbox' as string]: pathBackground }} data-step={step.step.id} data-surface={surfaceId}>
        <RenderHost surfaceId={surfaceId} mode="print" camera={camera} laid={laid} index={index} selection={NONE} onSelectionChange={() => undefined}
          editing={null} onEditingChange={() => undefined} canLayout={NO_LAYOUT} onCommitLayout={() => undefined}
          onContextMenu={() => undefined} onOpenEntity={() => undefined} gesturesPaused prefetch={shown} hidden={hidden} />
        <svg className="aiws-print-mask" width={W} height={H} viewBox={`0 0 ${W} ${H}`} aria-hidden="true">
          <path fillRule="evenodd" d={`M0 0H${W}V${H}H0Z M${hole.x} ${hole.y}H${hole.x + hole.w}V${hole.y + hole.h}H${hole.x}Z`} />
        </svg>
      </div>
    </PrintPage>
  )
}

export function FreePages({ steps, job, frame, background }: { steps: FreeStep[]; job: PrintJob; frame: PdfFrameInfo | null; background: string | undefined }) {
  const pathBackground = background ?? DEFAULT_BACKGROUND
  return (
    <>
      {steps.map((item, i) => (
        <FreePage key={`${item.step.step.id}:${i}`} item={item} job={job} frame={frame} pathBackground={pathBackground}
          context={{ page: i + 1, pages: steps.length, title: job.title, date: job.date }} />
      ))}
    </>
  )
}
