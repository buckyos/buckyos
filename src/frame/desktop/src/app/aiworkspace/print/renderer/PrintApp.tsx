/* The pages of a print (打印 TODO §3.2, T7): the job's canvas — a flow canvas paginated, a free canvas along its
 * presentation path — rendered by the Desktop's own Blocks in the `print` mode, with the Workspace's fonts. Tells
 * `onReady` once every page is drawn. */

import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { cssStack } from '@print/fonts.ts'
import type { PrintJob } from '@print/package.ts'
import type { Json, KeyedContent, PresentationProps, ShowPathPayload } from '../../api/types'
import { DEFAULT_BACKGROUND, playable, resolveSteps } from '../../presentation/model'
import { StoreContext, WorkspaceUiContext, type WorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { BlockHost } from '../../ui/blocks/BlockHost'
import { BudgetContext, createBudget } from '../../ui/blocks/budget'
import { listPdfFrames, type PdfFrameInfo } from '../../ui/controls/pdfframe/api'
import { FlowPages } from './FlowPages'
import { FreePages, type FreeStep } from './FreePages'
import { settled } from './ready'

/** Nothing a page mounts is limited: every editor-free Block, every HTML Block runs. */
const UNLIMITED = { editors: 1_000_000, html: 1_000_000 }
const HEX = /^#[0-9a-f]{6}$/i

interface Loaded {
  frame: PdfFrameInfo | null
  /** Flow: the canvas background. Free: the path background. */
  background: string
  steps: FreeStep[]
}

async function load(store: WorkspaceStore, job: PrintJob): Promise<Loaded> {
  const frame = job.frame_id ? listPdfFrames(store.outline.all()).find((f) => f.id === job.frame_id) ?? null : null
  if (job.mode === 'flow') {
    const read = await store.session.read<KeyedContent<{ config?: Record<string, Json> }>>(job.surface_id).catch(() => null)
    const bg = read?.content.payload.config?.background
    return { frame, background: typeof bg === 'string' && HEX.test(bg) ? bg.toLowerCase() : DEFAULT_BACKGROUND, steps: [] }
  }
  if (!job.path_id) return { frame, background: DEFAULT_BACKGROUND, steps: [] }
  const path = (await store.session.read<KeyedContent<ShowPathPayload>>(job.path_id)).content.payload
  const steps = await Promise.all(playable(resolveSteps(store.outline, path)).map(async (step): Promise<FreeStep> => {
    if (step.kind !== 'frame') return { step, background: null }
    const read = await store.session.read<KeyedContent<{ presentation?: PresentationProps }>>(step.targetId).catch(() => null)
    return { step, background: read?.content.payload.presentation?.background ?? null }
  }))
  return { frame, background: path.background ?? DEFAULT_BACKGROUND, steps }
}

function FreeDone({ deadline, onReady, children }: { deadline: number; onReady: () => void; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const root = ref.current
    if (!root) return
    let live = true
    void settled(root, deadline).then(() => { if (live) onReady() })
    return () => { live = false }
  }, [deadline, onReady])
  return <div ref={ref}>{children}</div>
}

export function PrintApp({ store, job, deadline, measureDeadline, onReady, onFail }: {
  store: WorkspaceStore; job: PrintJob; deadline: number; measureDeadline: number; onReady: () => void; onFail: (message: string) => void
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  useEffect(() => {
    let live = true
    load(store, job).then((l) => { if (live) setLoaded(l) }, (error: unknown) => {
      console.error('[aiws-print] the canvas could not be read', error)
      if (live) onFail(error instanceof Error ? error.message : String(error))
    })
    return () => { live = false }
  }, [store, job, onFail])
  // the Blocks on a page: no annotations, no navigation, embedded Cells read-only
  const ui = useMemo<WorkspaceUi>(() => {
    const entities = store.outline.all()
    return {
      entities, byId: new Map(entities.map((e) => [e.entity_id, e])), annotations: [], openEntity: () => undefined, annotate: null,
      activeAnnotation: null, setActiveAnnotation: () => undefined, annotationFilter: null, showAnnotations: () => undefined,
      renderCell: (cellId, depth) => <BlockHost cellId={cellId} mode="print" view="canvas" depth={depth} />,
    }
  }, [store])
  const [budget] = useState(() => createBudget(UNLIMITED))
  const fonts: CSSProperties = {
    ['--aiws-font-sans' as string]: cssStack(job.fonts, 'sans'),
    ['--aiws-font-serif' as string]: cssStack(job.fonts, 'serif'),
    ['--aiws-font-mono' as string]: cssStack(job.fonts, 'mono'),
    ['--aiws-mono' as string]: cssStack(job.fonts, 'mono'),
  }
  return (
    <StoreContext.Provider value={store}>
      <WorkspaceUiContext.Provider value={ui}>
        <BudgetContext.Provider value={budget}>
          <div className="aiws-root aiws-print-root" style={fonts} data-mode={job.mode}>
            {loaded && (job.mode === 'flow'
              ? <FlowPages job={job} frame={loaded.frame} background={loaded.background} measureDeadline={measureDeadline} deadline={deadline} onReady={onReady} />
              : <FreeDone deadline={deadline} onReady={onReady}><FreePages steps={loaded.steps} job={job} frame={loaded.frame} background={loaded.background} /></FreeDone>)}
          </div>
        </BudgetContext.Provider>
      </WorkspaceUiContext.Provider>
    </StoreContext.Provider>
  )
}
