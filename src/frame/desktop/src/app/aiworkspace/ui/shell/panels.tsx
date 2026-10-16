/* Shell panels shared by both top-level modes: the save-state list and the phase-one
 * controlled-processing panel kept as a kernel regression tool (a developer tool, UI improvement §6.1). */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { CommitResult, RunView, ServiceError, Touched } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdits, useStore } from '../../state/hooks'
import type { AiwsText } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'

/** Save states (design §6.5): entries that need attention, unsaved inputs, local-only saves. */
export function EditsPanel() {
  const store = useStore()
  const { t, tNode } = useAiwsText()
  const edits = useEdits()
  const list = [...edits.values()].sort((a, b) => b.at - a.at)
  const attention = list.filter((entry) => entry.state === 'needs_attention')
  const unsaved = list.filter((entry) => entry.state === 'unsaved')
  const local = list.filter((entry) => entry.state === 'saved_locally')
  if (attention.length === 0 && unsaved.length === 0 && local.length === 0) return null
  const replica = store.session.offline
  return (
    <div className="aiws-edits" data-testid="aiws-edits">
      <div className="aiws-panel-title">{t('aiworkspace.shell.side.edits')}</div>
      {[...attention, ...unsaved, ...local].map((entry) => (
        <div key={entry.id} className={`aiws-edit aiws-edit-${entry.state}`} data-testid="aiws-edit-entry" data-state={entry.state} data-code={entry.code} data-edit-id={entry.id}>
          <span className={`aiws-state aiws-state-${entry.state}`}>{EDIT_STATE_LABEL[entry.state]}</span>
          <b>{entry.label}</b>
          {entry.detail && <div data-testid="aiws-edit-detail">{entry.detail}</div>}
          {entry.hasMine && <div>{tNode('aiworkspace.shell.edits.mine', { value: <code data-testid="aiws-edit-mine">{typeof entry.mine === 'string' ? entry.mine : JSON.stringify(entry.mine)}</code> })}</div>}
          {entry.state === 'needs_attention' && (
            <>
              {replica && entry.pendingKey && entry.code !== 'REVISION_CONFLICT' && entry.code !== 'TARGET_DELETED' && store.session.status().kind !== 'stopped' && (
                <button type="button" className="aiws-link" data-testid="aiws-edit-requeue" title={t('aiworkspace.shell.edits.requeueHint')}
                  onClick={() => { void replica.requeuePending(entry.pendingKey ?? '').catch((error: unknown) => store.notify('error', describeError(error))) }}>{t('aiworkspace.shell.edits.requeue')}</button>
              )}
              <button type="button" className="aiws-link" data-testid="aiws-edit-dismiss" onClick={() => { void store.dismissEdit(entry.id) }}>{entry.pendingKey ? t('aiworkspace.shell.edits.discard') : t('aiworkspace.shell.edits.dismiss')}</button>
            </>
          )}
          {entry.state === 'unsaved' && entry.storageFailed && (
            <button type="button" className="aiws-link" data-testid="aiws-edit-export" onClick={() => { void store.exportLocal() }}>{t('aiworkspace.shell.exportUnsaved')}</button>
          )}
        </div>
      ))}
    </div>
  )
}

// ---- controlled processing (design §7): the phase-one Mock kept as a kernel regression tool

const MOCK_PROGRAM = 'mock.task-summary@1'

function touchedText(t: AiwsText['t'], item: Touched): string {
  const selector = item.selector
  const where = !selector ? '' : selector.kind === 'table_cell' ? ` / ${selector.record_id} / ${selector.field_id}` : selector.kind === 'richtext_block' ? t('aiworkspace.shell.mock.touchedBlock', { block: String(selector.block_id) }) : ` / ${selector.kind}`
  return t('aiworkspace.shell.mock.touched', { entity: item.entity_id, where, change: item.change })
}

export function MockRunPanel() {
  const store = useStore()
  const { t, tNode } = useAiwsText()
  const [today, setToday] = useState(() => new Date().toISOString().slice(0, 10))
  const [run, setRun] = useState<RunView | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const surfaces = store.outline.childrenOf('surfaces')
  const act = async (work: () => Promise<void>) => {
    setBusy(true)
    setMessage(null)
    try { await work() } catch (error) { setMessage(describeError(error)) } finally { setBusy(false) }
  }
  const start = () => act(async () => { setRun(await store.session.procStart(MOCK_PROGRAM, { today, surface_id: surfaces[0]?.entity_id ?? 'surface-main' })) })
  const apply = () => act(async () => {
    if (!run) return
    const next = await store.session.procApply(run.run_id)
    setRun(next)
    const result = next.result
    if (result && result.status === 'accepted') {
      store.undo.pushCommit(result.commit_id, t('aiworkspace.shell.mock.undoLabel'))
      await store.session.whenApplied(result.seq)
    }
  })
  const cancel = () => act(async () => {
    if (!run) return
    const result = await store.session.procCancel(run.run_id)
    setMessage(result.already_applied ? t('aiworkspace.shell.mock.alreadyApplied', { commit: result.commit_id ?? '' }) : t('aiworkspace.shell.mock.cancelled'))
    setRun(await store.session.procGet(run.run_id))
  })
  const prepare = run?.prepare
  const waiting = run?.state === 'waiting_confirmation'
  const outcome = run?.result as { error?: ServiceError; status?: string } | null | undefined
  const runError = outcome?.error ?? null
  const applied = outcome?.status ? (outcome as CommitResult) : null
  return (
    <div className="aiws-mock" data-testid="aiws-mock">
      <div className="aiws-panel-title">{t('aiworkspace.shell.mock.title')} <span className="aiws-chip aiws-chip-derived" data-testid="aiws-mock-simulated">{t('aiworkspace.shell.mock.simulated')}</span></div>
      <div className="aiws-muted">{t('aiworkspace.shell.mock.note', { program: MOCK_PROGRAM })}</div>
      <div className="aiws-inline-form">
        <label>{t('aiworkspace.shell.mock.today')} <input type="date" aria-label={t('aiworkspace.shell.mock.today')} data-testid="aiws-mock-today" value={today} onChange={(event) => setToday(event.target.value)} /></label>
        <button type="button" data-testid="aiws-mock-start" disabled={busy} onClick={() => { void start() }}>{t('aiworkspace.shell.mock.start')}</button>
      </div>
      {message && <div className="aiws-warning" role="status" data-testid="aiws-mock-message">{message}</div>}
      {run && (
        <div className="aiws-mock-run" data-testid="aiws-mock-run" data-state={run.state}>
          <div>{tNode('aiworkspace.shell.mock.run', { run: run.run_id, state: <b>{run.state}</b> })}</div>
          {prepare && prepare.status === 'ok' && <div data-testid="aiws-mock-prepare">{t('aiworkspace.shell.mock.prepareOk', { count: prepare.touched.length })}<ul>{prepare.touched.slice(0, 20).map((item, index) => <li key={index}>{touchedText(t, item)}</li>)}</ul></div>}
          {prepare && prepare.status !== 'ok' && <div className="aiws-error" role="alert" data-testid="aiws-mock-prepare">{t('aiworkspace.shell.mock.prepareFailed', { code: prepare.code, detail: prepare.status === 'conflict' ? t('aiworkspace.shell.mock.conflicts', { count: prepare.conflicts.length }) : prepare.errors?.[0]?.detail ?? '' })}</div>}
          {runError && <div className="aiws-warning" role="status" data-testid="aiws-mock-failed">{t('aiworkspace.shell.mock.runFailed', { code: runError.code, detail: runError.detail ?? '' })}</div>}
          {run.warnings.length > 0 && <div className="aiws-warning" data-testid="aiws-mock-warnings">{t('aiworkspace.shell.mock.warnings', { count: run.warnings.length })}<ul>{run.warnings.map((warning, index) => <li key={index}>{typeof warning === 'string' ? warning : JSON.stringify(warning)}</li>)}</ul></div>}
          {applied && applied.status !== 'accepted' && <div className="aiws-error" role="alert" data-testid="aiws-mock-result">{t('aiworkspace.shell.mock.applyRefused', { code: applied.code })}</div>}
          {applied?.status === 'accepted' && <div data-testid="aiws-mock-result">{t('aiworkspace.shell.mock.applied', { commit: applied.commit_id })}</div>}
          {waiting && (
            <div className="aiws-inline-form">
              <button type="button" data-testid="aiws-mock-apply" disabled={busy} onClick={() => { void apply() }}>{t('aiworkspace.shell.mock.apply')}</button>
              <button type="button" data-testid="aiws-mock-cancel" disabled={busy} onClick={() => { void cancel() }}>{t('aiworkspace.shell.mock.cancel')}</button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
