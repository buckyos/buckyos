/* The candidate of a run (许愿格 §8.7, §8.8, §11): results as they will look, the checks and what
 * needs the user's confirmation, structure changes, manual-edit choices, results that will be
 * missing, inputs appended during execution — and feedback for another round. Nothing is written
 * until 应用; the plan applied is exactly the one previewed. */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { WishChoices, WishRunView } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { ResultPreview } from './WishPreviews'
import { applyMessage } from './WishService'

const ACTION_LABEL: Record<string, string> = {
  create: 'aiworkspace.wish.action.create', update: 'aiworkspace.wish.action.update', unchanged: 'aiworkspace.wish.action.unchanged',
  keep_manual: 'aiworkspace.wish.action.keepManual', new_copy: 'aiworkspace.wish.action.newCopy',
}
const CHECK_LABEL: Record<string, string> = {
  passed: 'aiworkspace.wish.check.passed', failed: 'aiworkspace.wish.check.failed', not_run: 'aiworkspace.wish.check.notRun', review: 'aiworkspace.wish.check.review',
}
const TYPE_LABEL: Record<string, string> = {
  table: 'aiworkspace.wish.type.table', table_columns: 'aiworkspace.wish.type.tableColumns', record: 'aiworkspace.wish.type.record', richtext: 'aiworkspace.wish.type.richtext',
  image: 'aiworkspace.wish.type.image', asset: 'aiworkspace.wish.type.asset', html: 'aiworkspace.wish.type.html', video: 'aiworkspace.wish.type.video',
}
const CHANGE_LABEL: Record<string, string> = {
  add_field: 'aiworkspace.wish.change.addField', delete_field: 'aiworkspace.wish.change.deleteField', change_type: 'aiworkspace.wish.change.changeType',
  delete_rows: 'aiworkspace.wish.change.deleteRows', insert_rows: 'aiworkspace.wish.change.insertRows', update_values: 'aiworkspace.wish.change.updateValues',
  clear_values: 'aiworkspace.wish.change.clearValues', key_changed: 'aiworkspace.wish.change.keyChanged', recreate_field: 'aiworkspace.wish.change.recreateField',
  widen_scale: 'aiworkspace.wish.change.widenScale',
}

export function WishCandidate({ run, canApply, onMessage }: { run: WishRunView; canApply: boolean; onMessage: (kind: 'info' | 'error', text: string) => void }) {
  const store = useStore()
  const { t, list, clauses } = useAiwsText()
  const label = (labels: Record<string, string>, id: string) => (labels[id] ? t(labels[id]) : id)
  const [choices, setChoices] = useState<WishChoices>({})
  const [feedback, setFeedback] = useState('')
  const [open, setOpen] = useState<Record<string, boolean>>({})
  const [acceptFailed, setAcceptFailed] = useState(false)
  const [runSeen, setRunSeen] = useState(run.run_id)
  if (runSeen !== run.run_id) { setRunSeen(run.run_id); setChoices({}); setFeedback(''); setAcceptFailed(false) }
  const busy = store.wish.busy.get(run.wish_id)
  const cand = run.candidate ?? {}
  const preview = run.preview
  const summary = preview?.summary
  const choose = (next: WishChoices) => {
    setChoices(next)
    void store.wish.preview(run, next).catch((error: unknown) => onMessage('error', describeError(error)))
  }
  const apply = async () => {
    if (!preview) return
    try {
      const next = await store.wish.apply(run, preview.plan_digest)
      if (next.applied?.status === 'accepted') onMessage('info', run.stage === 'analyze' ? t('aiworkspace.wish.analysisWritten') : applyMessage(next))
      else if (next.applied?.status === 'conflict') onMessage('error', t('aiworkspace.wish.applyConflict', { code: next.applied.commit.status === 'conflict' ? next.applied.commit.code : '' }))
      else onMessage('error', t('aiworkspace.wish.applyRejected', { code: next.applied?.commit && 'code' in next.applied.commit ? next.applied.commit.code : next.state }))
    } catch (error) { onMessage('error', t('aiworkspace.wish.applyFailed', { error: describeError(error) })) }
  }
  const sendFeedback = async () => {
    if (!feedback.trim()) return
    try {
      await store.wish.execute(run.wish_id, { feedback: feedback.trim(), parentRunId: run.run_id, cellId: (run.params.location as { cell_id?: string } | undefined)?.cell_id })
      setFeedback('')
    } catch (error) { onMessage('error', describeError(error)) }
  }
  const results = cand.results ?? []
  const planned = new Map((summary?.results ?? []).map((r) => [r.name, r]))
  const checks = cand.checks ?? []
  const failed = checks.filter((c) => c.status === 'failed')
  const manual = summary?.manual ?? []
  const needsChoice = manual.some((m) => !choices.results?.[m.name])
  const destructive = Boolean(summary?.destructive)
  // i18n-exempt: matches the service's problem text (the server reports structure changes in Chinese)
  const blocking = (summary?.problems ?? []).filter((p) => !(destructive && p.includes('结构变化')))
  const title = t(run.stage === 'analyze' ? 'aiworkspace.wish.candidate.analysis' : run.stage === 'rerun_program' ? 'aiworkspace.wish.candidate.rerun' : run.stage === 'repair_program' ? 'aiworkspace.wish.candidate.repair' : 'aiworkspace.wish.candidate.results')
  return (
    <div className="aiws-wish-candidate" data-testid="aiws-wish-candidate" data-run-id={run.run_id} data-state={run.state} data-ready={preview?.ready ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <b>{title}</b>
        <span className="aiws-chip aiws-chip-derived">{t(run.simulated ? 'aiworkspace.wish.simulated' : cand.mode === 'program' ? 'aiworkspace.wish.candidate.programMode' : 'aiworkspace.wish.candidate.generated')}</span>
        {run.feedback && <span className="aiws-chip" title={run.feedback}>{t('aiworkspace.wish.reviseWithFeedback')}</span>}
        {cand.summary && <span className="aiws-muted">{cand.summary}</span>}
      </div>
      {run.stage === 'analyze' && cand.analysis && (
        <div className="aiws-wish-context">{cand.analysis.context_prompt}</div>
      )}
      {results.length > 0 && (
        <ul className="aiws-wish-results">
          {results.map((r) => {
            const p = planned.get(r.name)
            const isOpen = open[r.name] ?? results.length <= 3
            return (
              <li key={r.name} data-testid="aiws-wish-result" data-name={r.name} data-action={p?.action ?? ''}>
                <div className="aiws-wish-result-head">
                  <button type="button" className="aiws-link" onClick={() => setOpen((o) => ({ ...o, [r.name]: !isOpen }))}>{isOpen ? '▾' : '▸'} {r.title ?? r.name}</button>
                  <span className="aiws-muted">{label(TYPE_LABEL, r.type)}</span>
                  <span className="aiws-chip" title={t(r.approach === 'program' ? 'aiworkspace.wish.approach.programHint' : 'aiworkspace.wish.approach.directHint')}>{t(r.approach === 'program' ? 'aiworkspace.wish.approach.program' : 'aiworkspace.wish.approach.direct')}</span>
                  {p && <span className="aiws-chip aiws-chip-derived">{label(ACTION_LABEL, p.action)}{p.reordered ? ` · ${t('aiworkspace.wish.rowOrderUpdated')}` : ''}</span>}
                  {cand.model_judgment?.includes(r.name) && <span className="aiws-chip aiws-chip-warn" title={t('aiworkspace.wish.modelJudgmentHint')}>{t('aiworkspace.wish.modelJudgment')}</span>}
                  {r.approach === 'program' && (cand.external_data?.length ?? 0) > 0 && <span className="aiws-chip aiws-chip-warn" title={cand.external_data?.join('\n')}>{t('aiworkspace.wish.externalData')}</span>}
                </div>
                {isOpen && <ResultPreview result={r} />}
              </li>
            )
          })}
        </ul>
      )}
      {checks.length > 0 && (
        <div className="aiws-wish-checks" data-testid="aiws-wish-checks">
          <div className="aiws-muted">{t('aiworkspace.wish.acceptanceChecks')}</div>
          {checks.map((c) => (
            <div key={c.id} className={`aiws-wish-check is-${c.status}`} data-testid="aiws-wish-check" data-check={c.id} data-status={c.status}>
              <span className="aiws-chip">{label(CHECK_LABEL, c.status)}</span> {c.text}
              {c.detail !== undefined && c.detail !== null && <span className="aiws-muted"> · {typeof c.detail === 'string' ? c.detail : JSON.stringify(c.detail)}</span>}
              {c.self_assessment !== undefined && <span className="aiws-muted"> · {t('aiworkspace.wish.selfAssessment', { text: typeof c.self_assessment === 'string' ? c.self_assessment : JSON.stringify(c.self_assessment) })}</span>}
            </div>
          ))}
        </div>
      )}
      {failed.length > 0 && (
        <label className="aiws-error" role="alert" data-testid="aiws-wish-failed-checks">
          <input type="checkbox" data-testid="aiws-wish-accept-failed" checked={acceptFailed} onChange={(e) => setAcceptFailed(e.target.checked)} /> {t('aiworkspace.wish.acceptFailedChecks', { count: failed.length })}
        </label>
      )}
      {(cand.uncited_numbers?.length ?? 0) > 0 && (
        <div className="aiws-warning" data-testid="aiws-wish-uncited">{t('aiworkspace.wish.uncitedNumbers', { items: clauses((cand.uncited_numbers ?? []).map((u) => t('aiworkspace.wish.uncitedItem', { result: u.result, numbers: list(u.numbers) }))) })}</div>
      )}
      {(cand.assumptions?.length ?? 0) > 0 && <div className="aiws-muted">{t('aiworkspace.wish.assumptions', { items: clauses(cand.assumptions ?? []) })}</div>}
      {(cand.warnings?.length ?? 0) > 0 && <div className="aiws-warning" data-testid="aiws-wish-warnings">{clauses(cand.warnings ?? [])}</div>}
      {(summary?.appended_inputs?.length ?? 0) > 0 && (
        <div className="aiws-muted" data-testid="aiws-wish-appended">{t('aiworkspace.wish.appendedInputs', { names: list((summary?.appended_inputs ?? []).map((i) => i.label ?? store.outline.get(i.entity_id)?.title ?? i.entity_id)) })}</div>
      )}
      {(summary?.structure?.length ?? 0) > 0 && (
        <div className="aiws-wish-section" data-testid="aiws-wish-structure">
          <div className="aiws-muted">{t('aiworkspace.wish.structureChanges')}</div>
          {summary?.structure?.map((s) => (
            <div key={s.name}>{t('aiworkspace.wish.structureLine', {
              name: s.name,
              changes: s.changes.map((c) => `${label(CHANGE_LABEL, c.kind)}${c.field ? ` ${c.field}` : ''}${c.count ? ` ${c.count}` : ''}${c.from ? t('aiworkspace.wish.changeFromTo', { from: c.from, to: String(c.to) }) : c.kind === 'widen_scale' ? t('aiworkspace.wish.changeScale', { to: String(c.to) }) : ''}`).join(t('aiworkspace.wish.commaSeparator')),
              rows: s.rows_before !== undefined ? t('aiworkspace.wish.structureRows', { before: s.rows_before, after: String(s.rows_after), count: s.rows_after ?? 0 }) : '',
            })}</div>
          ))}
          {destructive && (
            <label className="aiws-warning"><input type="checkbox" data-testid="aiws-wish-confirm-structure" checked={Boolean(choices.confirm_structure)} onChange={(e) => choose({ ...choices, confirm_structure: e.target.checked })} /> {t('aiworkspace.wish.confirmStructure')}</label>
          )}
        </div>
      )}
      {manual.length > 0 && (
        <div className="aiws-warning" data-testid="aiws-wish-manual">
          {t('aiworkspace.wish.manualIntro')}
          {manual.map((m) => (
            <div key={m.name} className="aiws-inline-form" data-testid={`aiws-wish-manual-${m.name}`}>
              <span>{m.name}{m.cells ? t('aiworkspace.wish.manualCells', { count: m.cells }) : ''}{m.field_deleted ? t('aiworkspace.wish.manualFieldDeleted', { field: m.field_deleted }) : ''}</span>
              {(['keep', 'replace', 'new'] as const).map((choice) => (
                <label key={choice}><input type="radio" name={`manual-${run.run_id}-${m.name}`} checked={choices.results?.[m.name] === choice} onChange={() => choose({ ...choices, results: { ...(choices.results ?? {}), [m.name]: choice } })} /> {t(choice === 'keep' ? 'aiworkspace.wish.action.keepManual' : choice === 'replace' ? 'aiworkspace.wish.manualReplace' : 'aiworkspace.wish.manualNew')}</label>
              ))}
            </div>
          ))}
        </div>
      )}
      {(summary?.missing?.length ?? 0) > 0 && <div className="aiws-warning">{t('aiworkspace.wish.missingNotDeleted', { names: list((summary?.missing ?? []).map((m) => m.name)) })}</div>}
      {summary?.program && <div className="aiws-muted" data-testid="aiws-wish-program-change">{t(summary.program.from ? 'aiworkspace.wish.programChangeEdit' : 'aiworkspace.wish.programChangeFirst')}</div>}
      {(summary?.refinements?.length ?? 0) > 0 && <div className="aiws-muted">{t('aiworkspace.wish.refinementsOnApply', { items: clauses((summary?.refinements ?? []).map((r) => r.text)) })}</div>}
      {blocking.length > 0 && !needsChoice && <div className="aiws-error" data-testid="aiws-wish-problems">{clauses(blocking)}</div>}
      {run.state === 'conflict' && <div className="aiws-error">{t('aiworkspace.wish.lastApplyConflict')}</div>}
      {run.stage !== 'analyze' && !run.simulated && canApply && (
        <div className="aiws-wish-feedback">
          <textarea data-testid="aiws-wish-feedback" rows={2} placeholder={t('aiworkspace.wish.feedbackPlaceholder')} value={feedback} onChange={(e) => setFeedback(e.target.value)} />
          <button type="button" data-testid="aiws-wish-feedback-send" disabled={Boolean(busy) || !feedback.trim()} onClick={() => { void sendFeedback() }}>{t('aiworkspace.wish.reviseWithFeedback')}</button>
        </div>
      )}
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-wish-apply" disabled={Boolean(busy) || !canApply || !preview?.ready || needsChoice || (failed.length > 0 && !acceptFailed)} title={t(needsChoice ? 'aiworkspace.wish.applyHint.choose' : !preview?.ready ? 'aiworkspace.wish.applyHint.problems' : failed.length > 0 && !acceptFailed ? 'aiworkspace.wish.applyHint.confirmChecks' : 'aiworkspace.wish.applyHint.ready')} onClick={() => { void apply() }}>{t(run.stage === 'analyze' ? 'aiworkspace.wish.writeBackAnalysis' : 'aiworkspace.common.apply')}</button>
        <button type="button" data-testid="aiws-wish-discard" disabled={Boolean(busy)} onClick={() => store.wish.discard(run.wish_id)}>{t('aiworkspace.wish.discard')}</button>
      </div>
    </div>
  )
}
