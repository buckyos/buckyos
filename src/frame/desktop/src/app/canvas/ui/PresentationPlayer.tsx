/* ── Presentation overlay: step counter, prev/next, notes, return-to-step, exit (PRD FR-PRESENT-002) ── */

import { ChevronLeft, ChevronRight, LocateFixed, X } from 'lucide-react'
import { useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { Btn, IconBtn, Kbd } from './primitives'

export function PresentationPlayer() {
  const { doc, ui } = useStoreState()
  const actions = useEditorActions()
  const session = ui.presentation
  if (!session) return null
  const path = doc.presentationPaths.find((p) => p.id === session.pathId)
  if (!path) return null
  const step = path.steps[session.index]
  const total = path.steps.length
  return (
    <>
      <div className="pointer-events-none absolute left-4 top-4 z-[70] rounded-lg bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_92%,transparent)] px-3 py-2 text-xs shadow-[var(--cp-panel-shadow)]">
        <div className="font-display text-sm font-semibold">{path.name}</div>
        <div className="text-[color:var(--cp-muted)]">
          步骤 {session.index + 1} / {total}
          {step?.title ? ` · ${step.title}` : ''}
        </div>
      </div>
      <div className="absolute inset-x-0 bottom-4 z-[70] flex justify-center">
        <div className="flex max-w-[720px] flex-col items-center gap-2 rounded-xl bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_94%,transparent)] px-4 py-3 shadow-[var(--cp-window-shadow)]">
          {step?.note ? <p className="max-w-[640px] text-center text-sm leading-6">{step.note}</p> : null}
          <div className="flex items-center gap-2">
            <IconBtn icon={<ChevronLeft />} label="上一步 (←)" disabled={session.index === 0} onClick={() => actions.goToStep(session.index - 1)} />
            <span className="text-xs tabular-nums text-[color:var(--cp-muted)]">
              {session.index + 1} / {total}
            </span>
            <IconBtn icon={<ChevronRight />} label="下一步 (→)" disabled={session.index >= total - 1} onClick={() => actions.goToStep(session.index + 1)} />
            {session.deviated ? (
              <Btn variant="primary" icon={<LocateFixed />} onClick={() => actions.returnToStep()}>
                返回当前步骤
              </Btn>
            ) : (
              <span className="text-[11px] text-[color:var(--cp-muted)]">可随时拖动/缩放自由查看</span>
            )}
            <span className="mx-1 h-5 w-px bg-[color:var(--cp-border)]" />
            <Btn variant="ghost" icon={<X />} onClick={() => actions.stopPresentation()}>
              退出 <Kbd>Esc</Kbd>
            </Btn>
          </div>
        </div>
      </div>
    </>
  )
}
