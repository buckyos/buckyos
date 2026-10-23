/* The wish as the user works with it (许愿格详细设计 §14.1): the need, long-lived knowledge and
 * refinements; inputs (what is read and in which range); the analysis — context prompt, output
 * contract, acceptance checks, blockers; 分析 / 执行 / 只重跑程序 / 让 AI 修程序 / 取消; progress while
 * the service works; the candidate with its preview; and the state of the applied results. Shown by
 * the data-source detail and by the wish Block after activation. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { describeError, runErrorText, type ReadOk } from '../../api/session'
import { SHOWS_ID, SYSTEM_IDS, type Json, type WishExecutionMode, type WishInput, type WishPayloadRead, type WishRunView } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdit, useEntity, useFreshness, useLoad, useOutlineVersion, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, useIntent } from '../blocks/editorToolbar'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { WishCandidate } from './WishCandidate'
import { isActive, isWaiting, type WishBusy } from './WishService'

const BUSY_LABEL: Record<WishBusy, string> = {
  analyzing: 'aiworkspace.wish.busy.analyzing', executing: 'aiworkspace.wish.busy.executing', rerunning: 'aiworkspace.wish.busy.rerunning', repairing: 'aiworkspace.wish.busy.repairing',
  previewing: 'aiworkspace.wish.busy.previewing', applying: 'aiworkspace.wish.busy.applying', cancelling: 'aiworkspace.wish.busy.cancelling',
}
const EXECUTION_LABEL: Record<WishExecutionMode, string> = { service: 'aiworkspace.wish.execution.service', local: 'aiworkspace.wish.execution.local', hosted: 'aiworkspace.wish.execution.hosted' }
const PHASE_LABEL: Record<string, string> = {
  queued: 'aiworkspace.wish.phase.queued', snapshotting: 'aiworkspace.wish.phase.snapshotting', running: 'aiworkspace.wish.phase.running',
  validating: 'aiworkspace.wish.phase.validating', waiting_confirmation: 'aiworkspace.wish.phase.waitingConfirmation', applying: 'aiworkspace.wish.phase.applying',
}
const ACTIVITY_LABEL: Record<string, string> = {
  reading: 'aiworkspace.wish.activity.reading', writing_program: 'aiworkspace.wish.activity.writingProgram', running_program: 'aiworkspace.wish.activity.runningProgram',
  shell: 'aiworkspace.wish.activity.shell', writing: 'aiworkspace.wish.activity.writing', checking: 'aiworkspace.wish.activity.checking',
  submitting: 'aiworkspace.wish.activity.submitting', working: 'aiworkspace.wish.activity.working',
}
const STATUS_LABEL: Record<string, string> = {
  current: 'aiworkspace.wish.status.current', stale: 'aiworkspace.wish.status.stale', upstream_stale: 'aiworkspace.wish.status.upstreamStale',
  unavailable: 'aiworkspace.wish.status.unavailable', unknown: 'aiworkspace.wish.status.unknown', none: 'aiworkspace.common.none',
}

function scopeText(input: WishInput, title: (id: string) => string, t: (key: string, vars?: Record<string, string | number>) => string): string {
  const s = input.selector as { kind?: string; cell_id?: string; filter?: Json; fields?: string[] } | undefined
  if (!s || s.kind === 'entity') return t('aiworkspace.wish.scope.whole')
  if (s.kind === 'table_view') return t('aiworkspace.wish.scope.view', { view: title(s.cell_id ?? '') })
  if (s.kind === 'table_query') {
    if (s.fields) return t(s.filter ? 'aiworkspace.wish.scope.queryFieldsFiltered' : 'aiworkspace.wish.scope.queryFields', { count: s.fields.length })
    return t(s.filter ? 'aiworkspace.wish.scope.queryFiltered' : 'aiworkspace.wish.scope.query')
  }
  return s.kind ?? ''
}

/** The stale note naming the changed inputs (joined with " / "), or the inputs in general. */
function staleInputs(t: (key: string, vars?: Record<string, string | number>) => string, names: string[]): string {
  const joined = names.join(' / ')
  return joined ? t('aiworkspace.wish.staleInputs', { inputs: joined }) : t('aiworkspace.wish.staleInputsUnnamed')
}

/** Whose AI resources the current run uses, or — before one — whose the next run will use (AI Center Quota PRD §9.5). */
function AiccLine({ run }: { run: WishRunView | null | undefined }) {
  const { t } = useAiwsText()
  const store = useStore()
  const aicc = run?.aicc
  const text = aicc
    ? t(aicc.mode === 'current_user' ? 'aiworkspace.wish.aicc.runStarter' : 'aiworkspace.wish.aicc.runOwner', { user: aicc.user_id })
    : t(store.session.info().wish_aicc_mode === 'current_user' ? 'aiworkspace.wish.aicc.modeStarter' : 'aiworkspace.wish.aicc.modeOwner')
  return <div className="aiws-muted" data-testid="aiws-wish-aicc" data-mode={aicc?.mode ?? store.session.info().wish_aicc_mode ?? 'workspace_owner'}>{text}</div>
}

function Progress({ run, onCancel }: { run: WishRunView; onCancel: () => void }) {
  const { t } = useAiwsText()
  const p = run.progress ?? {}
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => { const t = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(t) }, [])
  const elapsed = Math.max(0, Math.round((now - Date.parse(run.created_at)) / 1000))
  const activity = ACTIVITY_LABEL[p.activity ?? '']
  const doing = p.waiting_model ? t('aiworkspace.wish.waitingModel') : activity ? t(activity) : ''
  const phase = p.phase ?? run.state
  return (
    <div className="aiws-wish-progress" data-testid="aiws-wish-progress" data-phase={p.phase ?? run.state}>
      <span className="aiws-chip">{PHASE_LABEL[phase] ? t(PHASE_LABEL[phase]) : phase}</span>
      {doing && <span>{doing}</span>}
      <span className="aiws-muted">
        {elapsed}s{p.tool_calls ? ` · ${t('aiworkspace.wish.progress.toolCalls', { count: p.tool_calls })}` : ''}{p.program_runs ? ` · ${t('aiworkspace.wish.progress.programRuns', { count: p.program_runs })}` : ''}
        {p.llm_map ? ` · ${t('aiworkspace.wish.progress.llmMap', { count: p.llm_map.items, cached: p.llm_map.cached })}` : ''}{p.model ? ` · ${p.model}` : ''}
      </span>
      {p.last_command && <code className="aiws-muted" title={p.last_command}>{p.last_command.slice(0, 60)}</code>}
      <button type="button" data-testid="aiws-wish-cancel" onClick={onCancel}>{t('aiworkspace.common.cancel')}</button>
    </div>
  )
}

function ProgramView({ wishId, canEdit }: { wishId: string; canEdit: boolean }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [source, setSource] = useState<string | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const load = async () => {
    setError(null)
    try { setSource(await store.wish.programSource(wishId) ?? '') } catch (e) { setError(describeError(e)) }
  }
  const save = async () => {
    if (draft === null) return
    try { await store.wish.saveProgram(wishId, draft); setSource(draft); setDraft(null) } catch (e) { setError(describeError(e)) }
  }
  if (source === null) return <button type="button" className="aiws-link" data-testid="aiws-wish-program" onClick={() => { void load() }}>{t('aiworkspace.wish.program.view')}</button>
  return (
    <div className="aiws-wish-program" data-testid="aiws-wish-program-view">
      {draft === null ? <pre className="aiws-wish-context">{source || t('aiworkspace.wish.program.none')}</pre> : <textarea data-testid="aiws-wish-program-edit" rows={14} value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} />}
      {error && <div className="aiws-error">{error}</div>}
      <div className="aiws-inline-form">
        {canEdit && draft === null && <button type="button" onClick={() => setDraft(source)}>{t('aiworkspace.common.edit')}</button>}
        {draft !== null && <button type="button" data-testid="aiws-wish-program-save" onClick={() => { void save() }}>{t('aiworkspace.wish.program.save')}</button>}
        {draft !== null && <button type="button" onClick={() => setDraft(null)}>{t('aiworkspace.wish.program.discard')}</button>}
        <button type="button" onClick={() => { setSource(null); setDraft(null) }}>{t('aiworkspace.wish.program.collapse')}</button>
      </div>
    </div>
  )
}

/** `canvasSelection`: what is selected on the canvas while the wish is open in the right panel — offered as inputs.
 * A "start" sent to `cellId` (the Block's "run") runs the next step once the wish is loaded. */
export function WishPanel({ wishId, readOnly, compact, cellId, canvasSelection }: { wishId: string; readOnly: boolean; compact?: boolean; cellId?: string; canvasSelection?: string[] }) {
  const store = useStore()
  const { t, list, clauses, dateTime } = useAiwsText()
  const ui = useWorkspaceUi()
  const entity = useEntity(wishId)
  const version = useVersion(`e:${wishId}`)
  const load = useCallback(() => store.session.read<WishPayloadRead>(wishId), [store, wishId])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  useOutlineVersion()
  const freshness = useFreshness(wishId)
  const [prompt, setPrompt] = useState<string | null>(null)
  const [knowledge, setKnowledge] = useState<string | null>(null)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [showKnowledge, setShowKnowledge] = useState(false)
  const editEntry = useEdit(`wish:${wishId}:apply`)
  const startIntent = useIntent(cellId ?? '')
  const startsDone = useRef(0)
  useEffect(() => { void store.wish.restore(wishId) }, [store, wishId])
  // "run" from the canvas: the next step of the two passes — analyse first, execute once the analysis is ready
  useEffect(() => {
    const payload = read.data?.content.payload
    if (!cellId || startIntent?.value !== 'start' || startIntent.seq <= startsDone.current || !payload || !entity || freshness === undefined) return
    startsDone.current = startIntent.seq
    consumeIntent(cellId, startIntent)
    if (readOnly || !entity.capabilities.includes('update') || isActive(store.wish.run(wishId)) || store.wish.busy.get(wishId)) return
    const analysis = payload.analysis
    const work = (freshness.needs_analysis ?? !analysis) || analysis?.status !== 'ready'
      ? store.wish.analyze(wishId, cellId).then(() => setMessage({ kind: 'info', text: t('aiworkspace.wish.analysisWritten') }))
      : store.wish.execute(wishId, { cellId })
    work.catch((error: unknown) => setMessage({ kind: 'error', text: describeError(error) }))
  }, [startIntent, read.data, entity, freshness, readOnly, store, wishId, cellId, t])
  const run = store.wish.run(wishId)
  const busy = store.wish.busy.get(wishId)
  const flowError = store.wish.errors.get(wishId)
  if (read.error && !read.data) return <div className="aiws-error" role="alert">{t('aiworkspace.wish.readFailed', { error: read.error })}</div>
  if (!read.data || !entity) return <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  const payload = read.data.content.payload
  const keyRevs = read.data.content.key_revs
  const canEdit = !readOnly && entity.capabilities.includes('update')
  // a collaborator with only `run`: the owner's Agent executes and writes (D22)
  const runOnly = !readOnly && !canEdit && store.wish.runOnly(wishId)
  const canRun = canEdit || runOnly
  const executionModes = payload.executor === 'xllm' ? store.wish.executionModes() : []
  const analysis = payload.analysis
  const needsAnalysis = freshness?.needs_analysis ?? !analysis
  const ready = analysis?.status === 'ready' && !needsAnalysis
  const inputs = payload.inputs ?? []
  const title = (id: string) => { const e = store.outline.get(id); return e?.title ?? e?.name ?? id }
  const running = isActive(run) || busy === 'analyzing' || busy === 'executing' || busy === 'rerunning' || busy === 'repairing'
  const act = async (work: () => Promise<unknown>) => {
    setMessage(null)
    try { await work() } catch (error) { setMessage({ kind: 'error', text: describeError(error) }) }
  }
  const setKey = (key: string, value: Json | null, label: string) => act(async () => {
    const op = value === null
      ? { op: 'entity.unset_keys' as const, entity_id: wishId, keys: [{ key, expect: { rev: keyRevs[key] ?? 0 } }] }
      : { op: 'entity.set_keys' as const, entity_id: wishId, keys: [{ key, value, expect: { rev: keyRevs[key] ?? 0 } }] }
    const outcome = await store.submit({ editId: `wish:${wishId}:${key}`, label, operations: [op] })
    if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(t('aiworkspace.wish.notSaved', { code: outcome.code }))
  })
  const savePrompt = () => {
    if (prompt === null || prompt === payload.prompt) { setPrompt(null); return }
    void setKey('prompt', prompt, t('aiworkspace.wish.edit.prompt')).then(() => setPrompt(null))
  }
  const saveKnowledge = () => {
    if (knowledge === null || knowledge === (payload.knowledge ?? '')) { setKnowledge(null); return }
    void setKey('knowledge', knowledge.trim() ? knowledge : null, t('aiworkspace.wish.edit.knowledge')).then(() => setKnowledge(null))
  }
  const setInputs = (next: WishInput[], label: string) => setKey('inputs', next as unknown as Json, label)
  const addInput = (entityId: string) => {
    if (!entityId || inputs.some((input) => input.entity_id === entityId)) return
    void setInputs([...inputs, { entity_id: entityId, label: title(entityId), version: { mode: 'follow' } }], t('aiworkspace.wish.addInput'))
  }
  const removeRefinement = (index: number) => {
    const next = (payload.refinements ?? []).filter((_, i) => i !== index)
    void setKey('refinements', next.length ? (next as unknown as Json) : null, t('aiworkspace.wish.deleteRefinement'))
  }
  const editRefinement = (index: number, text: string) => {
    const next = (payload.refinements ?? []).map((r, i) => (i === index ? { ...r, text } : r))
    void setKey('refinements', next as unknown as Json, t('aiworkspace.wish.edit.refinement'))
  }
  const analyze = () => act(async () => { await store.wish.analyze(wishId, cellId); setMessage({ kind: 'info', text: t('aiworkspace.wish.analysisWritten') }) })
  const execute = () => act(() => store.wish.execute(wishId, { cellId }))
  const resume = () => act(async () => { if (run) { await store.wish.resume(run); if (run.stage === 'analyze') setMessage({ kind: 'info', text: t('aiworkspace.wish.analysisWritten') }) } })
  const selectedInputs = canEdit ? [...new Set((canvasSelection ?? []).map((id) => { const e = store.outline.get(id); return e?.type_id === 'buckyos.cell' ? e.source_id ?? null : e && e.type_id !== 'buckyos.container' ? e.entity_id : null })
    .filter((id): id is string => Boolean(id) && id !== wishId && !inputs.some((input) => input.entity_id === id)))] : []
  const rerun = () => act(() => store.wish.rerunProgram(wishId, cellId))
  const repair = () => act(async () => { if (run) await store.wish.repairProgram(wishId, run.run_id, cellId) })
  const cancel = () => act(async () => { if (run) await store.wish.cancel(run) })
  const candidates = store.outline.all().filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.wish' && e.type_id !== 'buckyos.block-def' && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && store.outline.ancestors(e.entity_id).includes('data') && !SYSTEM_IDS.has(e.entity_id) && !store.outline.ancestors(e.entity_id).includes(SHOWS_ID))
  const lastRun = payload.last_run
  const programFailed = run?.stage === 'rerun_program' && run.state === 'failed' && run.error?.sub_code === 'PROGRAM_FAILED'
  const contract = analysis?.output_contract.results ?? []
  return (
    <div className={`aiws-wish${compact ? ' is-compact' : ''}`} data-testid={`aiws-wish-${wishId}`} data-busy={busy ?? (running ? 'running' : '')} data-needs-analysis={needsAnalysis ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <select data-testid="aiws-wish-executor" aria-label={t('aiworkspace.wish.executor')} value={payload.executor} disabled={!canEdit || running} onChange={(e) => { void setKey('executor', e.target.value, t('aiworkspace.wish.edit.executor')) }}>
          <option value="xllm">{t('aiworkspace.wish.executor.xllm')}</option>
          <option value="mock">{t('aiworkspace.wish.executor.mock')}</option>
        </select>
        {executionModes.length > 0 && !runOnly && (
          <select data-testid="aiws-wish-exec-mode" aria-label={t('aiworkspace.wish.execution')} value={store.wish.executionMode(wishId)} disabled={running} onChange={(e) => store.wish.setExecutionMode(e.target.value as WishExecutionMode)}>
            {executionModes.map((mode) => <option key={mode} value={mode}>{t(EXECUTION_LABEL[mode])}</option>)}
          </select>
        )}
        <FreshnessBadge entityId={wishId} />
        {busy && <span className="aiws-muted" data-testid="aiws-wish-busy">{t(BUSY_LABEL[busy])}</span>}
      </div>
      {runOnly && <div className="aiws-muted" data-testid="aiws-wish-run-only">{t('aiworkspace.wish.runOnlyHosted')}</div>}
      <label className="aiws-wish-prompt">
        <span className="aiws-muted">{t('aiworkspace.wish.request')}</span>
        <textarea data-testid="aiws-wish-prompt" rows={compact ? 2 : 3} value={prompt ?? payload.prompt} disabled={!canEdit} onChange={(event) => setPrompt(event.target.value)} onBlur={savePrompt} />
      </label>
      <div className="aiws-wish-knowledge">
        <button type="button" className="aiws-link" data-testid="aiws-wish-knowledge-toggle" onClick={() => setShowKnowledge((v) => !v)}>{showKnowledge ? '▾' : '▸'} {t(payload.knowledge ? 'aiworkspace.wish.knowledge' : 'aiworkspace.wish.knowledgeEmpty')}</button>
        {showKnowledge && <textarea data-testid="aiws-wish-knowledge" rows={3} placeholder={t('aiworkspace.wish.knowledgePlaceholder')} value={knowledge ?? payload.knowledge ?? ''} disabled={!canEdit} onChange={(e) => setKnowledge(e.target.value)} onBlur={saveKnowledge} />}
        {!showKnowledge && payload.knowledge && <div className="aiws-muted aiws-wish-oneline">{payload.knowledge}</div>}
      </div>
      {(payload.refinements?.length ?? 0) > 0 && (
        <div className="aiws-wish-section" data-testid="aiws-wish-refinements">
          <div className="aiws-muted">{t('aiworkspace.wish.refinements')}</div>
          {payload.refinements?.map((r, i) => (
            <div key={`${i}:${r.text}`} className="aiws-inline-form" data-testid="aiws-wish-refinement">
              <input defaultValue={r.text} disabled={!canEdit} onBlur={(e) => { if (e.target.value.trim() && e.target.value !== r.text) editRefinement(i, e.target.value) }} />
              {canEdit && <button type="button" className="aiws-icon" aria-label={t('aiworkspace.wish.deleteRefinement')} onClick={() => removeRefinement(i)}>×</button>}
            </div>
          ))}
        </div>
      )}
      <div className="aiws-wish-section">
        <div className="aiws-muted">{t('aiworkspace.wish.inputs')} {needsAnalysis && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-wish-needs-analysis">{t('aiworkspace.wish.needsAnalysis')}</span>}</div>
        {inputs.length === 0 && <div className="aiws-muted">{t('aiworkspace.wish.noInputs')}</div>}
        <ul className="aiws-wish-inputs">
          {inputs.map((input, index) => {
            const target = store.outline.get(input.entity_id)
            const problem = freshness?.input_problems?.find((p) => p.entity_id === input.entity_id)
            return (
              <li key={`${input.entity_id}:${index}`} data-testid="aiws-wish-input" data-problem={problem?.reason ?? ''}>
                {input.name && <code>{input.name}</code>}
                <button type="button" className="aiws-link" onClick={() => ui.openEntity(input.entity_id)}>{input.label ?? target?.title ?? target?.name ?? input.entity_id}</button>
                <span className="aiws-muted">{target?.type_id.replace('buckyos.', '') ?? '?'} · {scopeText(input, title, t)}{input.appended_by ? ` · ${t('aiworkspace.wish.appendedDuringRun')}` : ''}</span>
                {problem && <span className="aiws-error" data-testid="aiws-wish-input-problem">{t(problem.reason === 'missing' ? 'aiworkspace.wish.inputProblem.missing' : problem.reason === 'view_missing' ? 'aiworkspace.wish.inputProblem.viewMissing' : 'aiworkspace.wish.inputProblem.unreadable')}</span>}
                {canEdit && <button type="button" className="aiws-icon" aria-label={t('aiworkspace.wish.removeInput')} onClick={() => { void setInputs(inputs.filter((_, i) => i !== index), t('aiworkspace.wish.removeInput')) }}>×</button>}
              </li>
            )
          })}
        </ul>
        {selectedInputs.length > 0 && (
          <button type="button" data-testid="aiws-wish-add-selection" onClick={() => { void setInputs([...inputs, ...selectedInputs.map((id) => ({ entity_id: id, label: title(id), version: { mode: 'follow' as const } }))], t('aiworkspace.wish.edit.addSelection')) }}>
            {t('aiworkspace.wish.addSelection', { count: selectedInputs.length })}
          </button>
        )}
        {canEdit && (
          <select aria-label={t('aiworkspace.wish.addInput')} value="" data-testid="aiws-wish-add-input" onChange={(event) => addInput(event.target.value)}>
            <option value="">{t('aiworkspace.wish.addInputOption')}</option>
            {candidates.map((e) => <option key={e.entity_id} value={e.entity_id}>{t('aiworkspace.wish.nameWithType', { name: e.title ?? e.name ?? e.entity_id, type: e.type_id.replace('buckyos.', '') })}</option>)}
          </select>
        )}
      </div>
      {analysis && (
        <div className="aiws-wish-section" data-testid="aiws-wish-analysis" data-status={analysis.status}>
          <div className="aiws-muted">
            {t('aiworkspace.wish.analysis')} · {t(analysis.status === 'ready' ? 'aiworkspace.wish.analysisReady' : 'aiworkspace.wish.analysisMissingInputs')}{analysis.executor === 'mock' ? ` · ${t('aiworkspace.wish.executor.mock')}` : ''}{analysis.at ? ` · ${dateTime(analysis.at)}` : ''}
          </div>
          {analysis.blockers.length > 0 && (
            <div className="aiws-error" data-testid="aiws-wish-blockers">
              {analysis.blockers.map((b, i) => (
                <div key={i} data-testid="aiws-wish-blocker">
                  {b.message}
                  {(b.candidates ?? []).map((c) => <button key={c} type="button" className="aiws-link" disabled={!canEdit} onClick={() => addInput(c)} title={t('aiworkspace.wish.addBlockerInput')}>{title(c)}</button>)}
                </div>
              ))}
            </div>
          )}
          <details open={!compact}>
            <summary className="aiws-muted">{t('aiworkspace.wish.contextPrompt')}</summary>
            <div className="aiws-wish-context">{analysis.context_prompt}</div>
          </details>
          {contract.length > 0 && (
            <table className="aiws-rt-table aiws-wish-contract" data-testid="aiws-wish-contract">
              <thead><tr><th>{t('aiworkspace.wish.contract.result')}</th><th>{t('aiworkspace.wish.contract.type')}</th><th>{t('aiworkspace.wish.contract.approach')}</th><th>{t('aiworkspace.wish.contract.views')}</th></tr></thead>
              <tbody>{contract.map((r) => (
                <tr key={r.name}><td><code>{r.name}</code> {r.title}</td><td>{r.type}{r.key ? t('aiworkspace.wish.contract.key', { key: r.key.join('+') }) : ''}{r.target ? `→ ${r.target}` : ''}</td><td>{t(r.approach === 'program' ? 'aiworkspace.wish.approach.program' : 'aiworkspace.wish.approach.direct')}</td><td>{list((r.views ?? []).map((v) => v.renderer)) || t('aiworkspace.wish.contract.defaultView')}</td></tr>
              ))}</tbody>
            </table>
          )}
          {analysis.checks.length > 0 && (
            <ul className="aiws-wish-checklist" data-testid="aiws-wish-checkdefs">{analysis.checks.map((c) => <li key={c.id}>{t(c.kind === 'program' ? 'aiworkspace.wish.checkProgram' : 'aiworkspace.wish.checkManual', { text: c.text })}</li>)}</ul>
          )}
          {analysis.warnings.length > 0 && <div className="aiws-warning">{clauses(analysis.warnings)}</div>}
        </div>
      )}
      <div className="aiws-inline-form">
        <label>{t('aiworkspace.wish.outputMode')}
          <select data-testid="aiws-wish-mode" value={payload.output_mode ?? 'overwrite'} disabled={!canEdit} onChange={(event) => { void setKey('output_mode', event.target.value === 'new' ? 'new' : 'overwrite', t(event.target.value === 'new' ? 'aiworkspace.wish.edit.outputNew' : 'aiworkspace.wish.edit.outputOverwrite')) }}>
            <option value="overwrite">{t('aiworkspace.wish.outputOverwrite')}</option>
            <option value="new">{t('aiworkspace.wish.outputNew')}</option>
          </select>
        </label>
        <button type="button" data-testid="aiws-wish-analyze" disabled={!canRun || running || Boolean(busy)} onClick={() => { void analyze() }}>{t('aiworkspace.wish.analyze')}</button>
        <button type="button" data-testid="aiws-wish-execute" disabled={!canRun || running || Boolean(busy) || !ready} title={t(!analysis ? 'aiworkspace.wish.executeHint.analyzeFirst' : needsAnalysis ? 'aiworkspace.wish.executeHint.changed' : analysis.status !== 'ready' ? 'aiworkspace.wish.executeHint.missingInputs' : 'aiworkspace.wish.executeHint.ready')} onClick={() => { void execute() }}>{t('aiworkspace.wish.execute')}</button>
        {payload.program && payload.executor === 'xllm' && <button type="button" data-testid="aiws-wish-rerun" disabled={!canEdit || running || Boolean(busy)} title={t('aiworkspace.wish.rerunHint')} onClick={() => { void rerun() }}>{t('aiworkspace.wish.rerun')}</button>}
        {programFailed && <button type="button" data-testid="aiws-wish-repair" disabled={!canEdit || running || Boolean(busy)} onClick={() => { void repair() }}>{t('aiworkspace.wish.repair')}</button>}
      </div>
      <AiccLine run={run} />
      {run && isActive(run) && <Progress run={run} onCancel={() => { void cancel() }} />}
      {(message || flowError) && (
        <div className={message?.kind === 'info' ? 'aiws-warning' : 'aiws-error'} role={message?.kind === 'info' ? 'status' : 'alert'} data-testid="aiws-wish-message">{message?.text ?? flowError}</div>
      )}
      {run && !isActive(run) && run.state === 'failed' && !message && !flowError && <div className="aiws-error" data-testid="aiws-wish-message">{t('aiworkspace.wish.runFailed', { detail: runErrorText(run.error) ?? run.state })}</div>}
      {run && run.state === 'interrupted' && !busy && (
        <div className="aiws-warning aiws-inline-form" data-testid="aiws-wish-interrupted">
          <span>{t('aiworkspace.wish.interrupted')}</span>
          {store.wish.canResume(run)
            ? <button type="button" data-testid="aiws-wish-resume" disabled={!canRun} onClick={() => { void resume() }}>{t('aiworkspace.wish.resume')}</button>
            : <span className="aiws-muted">{t('aiworkspace.wish.resumeHint')}</span>}
        </div>
      )}
      {editEntry && editEntry.state !== 'committed' && <div className={`aiws-state aiws-state-${editEntry.state}`} data-testid="aiws-wish-apply-state">{editEntry.detail ? t('aiworkspace.wish.stateWithDetail', { state: EDIT_STATE_LABEL[editEntry.state], detail: editEntry.detail }) : EDIT_STATE_LABEL[editEntry.state]}</div>}
      {run && isWaiting(run) && <WishCandidate run={run} canApply={canEdit} onMessage={(kind, text) => setMessage({ kind, text })} />}
      {lastRun && (
        <div className="aiws-wish-section" data-testid="aiws-wish-last-run">
          <div className="aiws-muted">{t('aiworkspace.wish.lastApplied')} {lastRun.mode === 'program' ? t('aiworkspace.wish.lastAppliedProgram') : ''}{lastRun.at ? ` · ${dateTime(lastRun.at)}` : ''}{lastRun.simulated ? ` · ${t('aiworkspace.wish.simulated')}` : ''}{lastRun.checks ? ` · ${t(lastRun.checks.failed ? 'aiworkspace.wish.lastChecksFailed' : 'aiworkspace.wish.lastChecks', { passed: lastRun.checks.passed, failed: lastRun.checks.failed ?? 0 })}` : ''}</div>
          <div>
            {(freshness?.results?.length ? freshness.results : Object.entries(lastRun.result_bindings?.results ?? {}).map(([name, b]) => ({ name, entity_id: b.entity_id, status: 'unknown' as const, approach: b.approach }))).map((r) => (
              <button key={r.name} type="button" className="aiws-link" data-testid="aiws-wish-produced" data-status={r.status} onClick={() => { if (r.entity_id) ui.openEntity(r.entity_id) }}>
                {lastRun.result_bindings?.results[r.name]?.title ?? r.name}<span className="aiws-muted">{t(r.approach === 'direct' ? 'aiworkspace.wish.producedStatusDirect' : 'aiworkspace.wish.producedStatus', { status: STATUS_LABEL[r.status] ? t(STATUS_LABEL[r.status]) : r.status })}</span>
              </button>
            ))}
          </div>
          {freshness?.direct_stale && <div className="aiws-warning" data-testid="aiws-wish-direct-stale">{t('aiworkspace.wish.directStale')}</div>}
          {(lastRun.missing ?? []).length > 0 && (
            <div className="aiws-warning" data-testid="aiws-wish-missing">{t('aiworkspace.wish.lastMissing', { names: list((lastRun.missing ?? []).map((id) => title(id))) })}</div>
          )}
          {freshness && freshness.status !== 'none' && freshness.status !== 'current' && !freshness.direct_stale && (
            <div className="aiws-warning" data-testid="aiws-wish-stale">{freshness.config_changed ? t('aiworkspace.wish.staleConfig') : freshness.status === 'stale' ? staleInputs(t, (freshness.changed_inputs ?? []).map((l) => l.name ?? l.label ?? l.entity_id)) : t(freshness.status === 'upstream_stale' ? 'aiworkspace.wish.status.upstreamStale' : freshness.status === 'unavailable' ? 'aiworkspace.wish.status.unavailable' : 'aiworkspace.wish.status.unknown')}</div>
          )}
          {payload.program && <ProgramView wishId={wishId} canEdit={canEdit && payload.executor === 'xllm'} />}
        </div>
      )}
    </div>
  )
}
