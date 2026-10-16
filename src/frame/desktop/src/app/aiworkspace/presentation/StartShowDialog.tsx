/* "开始放映" (第三期规划 §7.2, §8.1): choose a presentation path and its first step; the edits of this window are
 * settled first (what cannot be is asked about), then the show is started (showSession.ts). */

import { useCallback, useState } from 'react'
import { describeError } from '../api/session'
import type { KeyedContent, ShowPathPayload } from '../api/types'
import { useLoad, useStore, useVersion } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { hasLiveBlocks, pathsOf, playable, resolveSteps, surfacesOfSteps } from './model'
import { WRITE_CAPS } from './showSession'

export function StartShowDialog({ initialPathId, initialStepId, onClose, onStart }: {
  initialPathId: string | null
  initialStepId: string | null
  onClose: () => void
  onStart: (pathId: string, stepId: string | null) => Promise<void>
}) {
  const store = useStore()
  const { t, clauses } = useAiwsText()
  const paths = pathsOf(store.outline, 'presentation')
  const [pathId, setPathId] = useState(initialPathId ?? paths[0]?.entity_id ?? '')
  const [stepId, setStepId] = useState(initialStepId ?? '')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [leaving, setLeaving] = useState<{ unsaved: number; attention: number; memoryOnly: number } | null>(null)
  const version = useVersion(`e:${pathId}`)
  const load = useCallback(() => (pathId ? store.session.read<KeyedContent<ShowPathPayload>>(pathId).then((r) => r.content.payload) : Promise.resolve(null)), [store, pathId])
  const path = useLoad(load, version).data ?? null
  const all = path ? resolveSteps(store.outline, path) : []
  const steps = playable(all)
  const skipped = all.length - steps.length
  const live = steps.length > 0 && hasLiveBlocks(store.outline, surfacesOfSteps(steps))
  const offline = store.session.status().kind !== 'live'
  const writer = store.session.info().capabilities.some((c) => WRITE_CAPS.includes(c))
  const go = async (force: boolean) => {
    if (!pathId) return
    setError(null)
    if (!force) {
      // §8.1: end the active edit and wait for the writes that can still complete
      setBusy(t('aiworkspace.presentation.start.savingEdits'))
      const left = await store.prepareLeave()
      setBusy(null)
      if (left.unsaved + left.attention + left.memoryOnly > 0) { setLeaving(left); return }
    }
    setBusy(live && writer && !offline ? t('aiworkspace.presentation.start.preparingClone') : t('aiworkspace.presentation.start.starting'))
    try {
      await onStart(pathId, stepId || null)
    } catch (e) {
      setError(describeError(e))
      setBusy(null)
    }
  }
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget && !busy) onClose() }}>
      <div className="aiws-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.presentation.show.start')} data-testid="aiws-start-show" onKeyDown={(event) => { if (event.key === 'Escape' && !busy) onClose() }}>
        <div className="aiws-dialog-head"><b>{t('aiworkspace.presentation.show.start')}</b></div>
        {paths.length === 0 ? (
          <div className="aiws-muted">{t('aiworkspace.presentation.start.noPaths')}</div>
        ) : (
          <>
            <label className="aiws-inline-form">{t('aiworkspace.presentation.start.path')}
              <select aria-label={t('aiworkspace.presentation.start.path')} data-testid="aiws-start-path" value={pathId} disabled={busy !== null} onChange={(event) => { setPathId(event.target.value); setStepId('') }}>
                {paths.map((p) => <option key={p.entity_id} value={p.entity_id}>{p.title ?? p.name ?? p.entity_id}</option>)}
              </select>
            </label>
            <label className="aiws-inline-form">{t('aiworkspace.presentation.start.fromStep')}
              <select aria-label={t('aiworkspace.presentation.start.firstStepLabel')} data-testid="aiws-start-step" value={stepId} disabled={busy !== null} onChange={(event) => setStepId(event.target.value)}>
                <option value="">{t('aiworkspace.presentation.start.firstStep')}</option>
                {steps.map((s, i) => <option key={s.step.id} value={s.step.id}>{i + 1}. {s.title}</option>)}
              </select>
            </label>
            {path && (
              <div className="aiws-muted" data-testid="aiws-start-summary">
                {t('aiworkspace.presentation.start.summary', { clauses: clauses([
                  t('aiworkspace.presentation.start.stepCount', { count: steps.length }),
                  ...(skipped > 0 ? [t('aiworkspace.presentation.start.skipped', { count: skipped })] : []),
                  t('aiworkspace.presentation.start.stage', { w: path.stage.w, h: path.stage.h }),
                ]) })}
              </div>
            )}
            {offline ? (
              <div className="aiws-warning">{t('aiworkspace.presentation.start.offline')}</div>
            ) : writer ? (
              <div className="aiws-muted">{live ? t('aiworkspace.presentation.start.writerLive') : t('aiworkspace.presentation.start.writer')}</div>
            ) : (
              <div className="aiws-muted">{t('aiworkspace.presentation.start.reader')}</div>
            )}
            {leaving && (
              <div className="aiws-warning" data-testid="aiws-start-pending">
                {t('aiworkspace.presentation.start.pending', { count: leaving.unsaved + leaving.attention + leaving.memoryOnly })}
                <div className="aiws-dialog-actions">
                  <button type="button" onClick={() => { setLeaving(null); void go(true) }}>{t('aiworkspace.presentation.start.anyway')}</button>
                  <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
                </div>
              </div>
            )}
          </>
        )}
        {error && <div className="aiws-error" role="alert">{t('aiworkspace.presentation.start.failed', { detail: error })}</div>}
        <div className="aiws-dialog-actions">
          <button type="button" className="is-primary" data-testid="aiws-start-show-go" disabled={!pathId || steps.length === 0 || busy !== null || leaving !== null} onClick={() => { void go(false) }}>{busy ?? t('aiworkspace.presentation.show.start')}</button>
          <button type="button" disabled={busy !== null} onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
        </div>
      </div>
    </div>
  )
}
