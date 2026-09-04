/* ── Three-step first-run coach for the sample canvas (PRD §9) ── */

import { useEffect, useState } from 'react'
import { SAMPLE_IDS, SAMPLE_TEMPLATE_ID } from '../fixtures/sample-canvas'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { trackEvent } from '../events'
import { useEditorActions } from './actions'
import { Btn } from './primitives'

const KEY = 'aicanvas.onboarding.done'

const STEPS = [
  { title: '这里是你的数据', body: '这是一张普通的季度销售表，可以像 Excel 一样直接修改数字。', block: SAMPLE_IDS.table, run: false },
  { title: '在许愿格里直接写目标', body: '许愿格已经引用了这张表，里面预置了一段用自然语言写的分析目标，你也可以改成自己的话。', block: SAMPLE_IDS.wish, run: false },
  { title: '按运行，让结果留在画布上', body: '结果会作为可编辑的块出现在下方框架里，并记住它来自哪份数据。', block: SAMPLE_IDS.wish, run: true },
]

export function OnboardingCoach() {
  const { doc } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const [step, setStep] = useState<number | null>(() => {
    try {
      if (localStorage.getItem(KEY)) return null
    } catch {
      /* ignore */
    }
    return doc.metadata.sourceTemplateId === SAMPLE_TEMPLATE_ID && doc.blocks[SAMPLE_IDS.wish] ? 0 : null
  })

  useEffect(() => {
    if (step === null) {
      store.setUi({ highlightBlockId: null, highlightRun: false })
      return
    }
    const s = STEPS[step]
    store.setUi({ highlightBlockId: s.block, highlightRun: s.run })
    actions.focusBlock(s.block)
  }, [step, store, actions])

  if (step === null) return null
  const finish = (skipped: boolean) => {
    try {
      localStorage.setItem(KEY, '1')
    } catch {
      /* ignore */
    }
    trackEvent(skipped ? 'onboarding_skipped' : 'onboarding_completed')
    setStep(null)
  }
  const s = STEPS[step]
  return (
    <div className="aic-fade-in absolute bottom-14 right-[316px] z-[65] w-[300px] rounded-xl border border-[color:var(--cp-accent)] bg-[color:var(--cp-surface-opaque)] p-3 shadow-[var(--cp-window-shadow)]">
      <div className="text-[11px] font-semibold uppercase tracking-wide text-[color:var(--cp-accent)]">
        {step + 1} / {STEPS.length}
      </div>
      <div className="mt-1 font-display text-sm font-semibold">{s.title}</div>
      <p className="mt-1 text-xs leading-5 text-[color:var(--cp-muted)]">{s.body}</p>
      <div className="mt-3 flex justify-end gap-1">
        <Btn variant="ghost" onClick={() => finish(true)}>
          跳过
        </Btn>
        {step < STEPS.length - 1 ? (
          <Btn variant="primary" onClick={() => setStep(step + 1)}>
            下一步
          </Btn>
        ) : (
          <Btn variant="primary" onClick={() => finish(false)}>
            知道了
          </Btn>
        )}
      </div>
    </div>
  )
}
