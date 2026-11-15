/* The wish card (许愿格交互升级 §3.1–§3.6): everything a wish does happens on it. From top to bottom the title, one line of
 * state, the current tab's content (scrolling inside the card); four tabs bottom left — the request, its requirements,
 * the macro, its results — the next step bottom right and the run button top right. A canvas, a flow page, the
 * data-source detail, the data mode and a show's stage all show this one component.
 *
 * Its tabs and buttons take a click in design mode too: they are the Block's own controls, not its content (an enabled
 * button inside a Block is not a canvas gesture, RenderHost `insideEditor`). A press on the rest of the card — and on a
 * greyed tab, which is no control — selects and moves the Block as on any other. */

import { useEffect, useRef, useState, type ReactNode } from 'react'
import { CircleStop, Play, Sparkles } from 'lucide-react'
import { needsCodingReasonText, runErrorText } from '../../api/session'
import type { FreshnessInfo, Json, NeedsCodingReason, WishMainStatus, WishRunView, WishStage } from '../../api/types'
import type { AiwsText } from '../../i18n'
import { useEntity, useReadOnlyReason, useStore, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, useIntent } from '../blocks/editorToolbar'
import { runMacro } from '../framework/macros'
import { useInvalidation } from './confirm'
import { holdsMacro, macroUsesAi } from './macro'
import { failureText, latestRecord, stageRecords } from './records'
import { WishRecords } from './WishRecords'
import { isCodingFailure } from './WishService'
import { setWishKey, useWish, type WishState } from './useWish'
import './wish.css'

export type WishTab = 'prompt' | 'analysis' | 'macro' | 'results'
const TABS: readonly WishTab[] = ['prompt', 'analysis', 'macro', 'results']
const TAB_STAGE: Record<Exclude<WishTab, 'prompt'>, WishStage> = { analysis: 'analyze', macro: 'code', results: 'run' }
const STAGE_TAB: Record<WishStage, WishTab> = { analyze: 'analysis', code: 'macro', run: 'results' }
const TAB_LABEL: Record<WishTab, string> = {
  prompt: 'aiworkspace.wishcard.tab.prompt', analysis: 'aiworkspace.wishcard.tab.analysis', macro: 'aiworkspace.wishcard.tab.macro', results: 'aiworkspace.wishcard.tab.results',
}
const STAGE_RUNNING: Record<WishStage, string> = { analyze: 'aiworkspace.wishcard.running.analyze', code: 'aiworkspace.wishcard.running.code', run: 'aiworkspace.wishcard.running.run' }
const STAGE_FAILED: Record<WishStage, string> = { analyze: 'aiworkspace.wishcard.status.failedAnalyze', code: 'aiworkspace.wishcard.status.failedCode', run: 'aiworkspace.wishcard.status.failedRun' }
const MAIN_LABEL: Record<WishMainStatus['main'], string> = {
  needs_analysis: 'aiworkspace.wish.main.needsAnalysis', needs_coding: 'aiworkspace.wish.main.needsCoding', waiting_data: 'aiworkspace.wish.main.waitingData',
  no_access: 'aiworkspace.wish.main.noAccess', cannot_run: 'aiworkspace.wish.main.cannotRun', in_progress: 'aiworkspace.wish.main.inProgress',
  stale: 'aiworkspace.wish.main.stale', current: 'aiworkspace.wish.main.current',
}
const ACTIVITY_LABEL: Record<string, string> = {
  reading: 'aiworkspace.wish.activity.reading', writing_program: 'aiworkspace.wish.activity.writingProgram', running_program: 'aiworkspace.wish.activity.runningProgram',
  shell: 'aiworkspace.wish.activity.shell', writing: 'aiworkspace.wish.activity.writing', checking: 'aiworkspace.wish.activity.checking',
  submitting: 'aiworkspace.wish.activity.submitting', working: 'aiworkspace.wish.activity.working',
}
const PHASE_LABEL: Record<string, string> = {
  queued: 'aiworkspace.wish.phase.queued', snapshotting: 'aiworkspace.wish.phase.snapshotting', validating: 'aiworkspace.wish.phase.validating', applying: 'aiworkspace.wish.phase.applying',
}
const PROBLEM_LABEL: Record<string, string> = {
  missing: 'aiworkspace.wish.inputProblem.missing', view_missing: 'aiworkspace.wish.inputProblem.viewMissing', unreadable: 'aiworkspace.wish.inputProblem.unreadable',
}

const nameOf = (store: WorkspaceStore, id: string) => { const e = store.outline.get(id); return e?.title ?? e?.name ?? id }

/** What the run is doing now, in a few words (its phase around the work, else the model's step). */
function runDoing(run: WishRunView | undefined, text: AiwsText): string {
  if (!run) return ''
  const p = run.progress ?? {}
  const phase = p.phase ?? run.state
  if (PHASE_LABEL[phase]) return text.t(PHASE_LABEL[phase])
  if (p.waiting_model) return text.t('aiworkspace.wish.waitingModel')
  if (p.activity === 'running_program' && run.stage === 'code') return text.t('aiworkspace.wishcard.running.trial')
  return ACTIVITY_LABEL[p.activity ?? ''] ? text.t(ACTIVITY_LABEL[p.activity ?? '']) : ''
}

/** One reason of the main state as a short phrase. */
function reasonText(reason: Json, store: WorkspaceStore, text: AiwsText): string | null {
  const r = (reason ?? {}) as { code?: string; entity_id?: string; reason?: string }
  if (r.reason && r.entity_id) return text.t('aiworkspace.wish.reason.inputProblem', { input: nameOf(store, r.entity_id), problem: PROBLEM_LABEL[r.reason] ? text.t(PROBLEM_LABEL[r.reason]) : r.reason })
  if (r.code === 'analysis_sources_changed') return text.t('aiworkspace.wish.reason.analysisSources')
  if (r.code && /^[a-z_]+$/.test(r.code)) return needsCodingReasonText(reason as unknown as NeedsCodingReason)
  if (r.code) return runErrorText({ code: r.code, sub_code: r.code }) ?? r.code
  return null
}

/** The card's line of state (§3.1): the run under way and what it does; else the newest record when it failed; else the
 * main state and its first reason. */
function statusLine(w: WishState, store: WorkspaceStore, text: AiwsText): { kind: 'running' | 'failed' | 'warn' | 'ok'; text: string } {
  const { t } = text
  if (w.flowError) return { kind: 'failed', text: t('aiworkspace.wishcard.status.notStarted', { error: w.flowError }) }
  if (w.runningStage) {
    const doing = runDoing(w.active, text)
    return { kind: 'running', text: doing ? t('aiworkspace.wishcard.status.runningDoing', { stage: t(STAGE_RUNNING[w.runningStage]), doing }) : t(STAGE_RUNNING[w.runningStage]) }
  }
  if (w.payload && w.payload.executor !== 'xllm') return { kind: 'failed', text: t('aiworkspace.wishcard.status.noExecutor') }
  const latest = latestRecord(w.payload?.history)
  // a run this window followed fails before its record arrives
  if (w.run?.state === 'failed' && (!latest || latest.at < w.run.created_at)) {
    return { kind: 'failed', text: t(STAGE_FAILED[w.run.stage], { reason: runErrorText(w.run.error) ?? t('aiworkspace.wishcard.record.failedUnknown') }) }
  }
  if (latest && !latest.ok && latest.action !== 'rollback') return { kind: 'failed', text: t(STAGE_FAILED[latest.stage], { reason: failureText(latest, text) }) }
  const main = w.main
  if (!main) return { kind: 'ok', text: '' }
  const first = main.reasons.map((r) => reasonText(r, store, text)).find((r): r is string => Boolean(r))
  let reason = first ?? null
  let label = t(MAIN_LABEL[main.main])
  if (main.main === 'needs_analysis') reason ??= t(w.payload?.analysis ? 'aiworkspace.wishcard.status.requestChanged' : 'aiworkspace.wishcard.status.notAnalyzed')
  if (main.main === 'stale') {
    if (!main.stale) { label = t('aiworkspace.wish.main.notRun'); reason = null } else reason ??= staleReason(w.freshness, text)
  }
  const kind = main.main === 'current' || main.main === 'in_progress' ? 'ok' : 'warn'
  return { kind, text: reason ? t('aiworkspace.wishcard.status.withReason', { state: label, reason }) : label }
}

function staleReason(freshness: FreshnessInfo | undefined, text: AiwsText): string | null {
  if (!freshness) return null
  if (freshness.config_changed) return text.t('aiworkspace.wishcard.status.configChanged')
  const names = (freshness.changed_inputs ?? []).map((l) => l.name ?? l.label ?? l.entity_id)
  return names.length ? text.t('aiworkspace.wishcard.status.inputsChanged', { inputs: text.list(names) }) : null
}

interface Facts {
  canUpdate: boolean
  /** Why nothing of the wish can be changed here now, or null. */
  writeReason: string | null
  results: number
  hasAnalysis: boolean
  hasMacro: boolean
  needsAnalysis: boolean
  needsCoding: NeedsCodingReason[] | null
  executorOk: boolean
}

function factsOf(w: WishState, mode: 'edit' | 'view', readOnly: boolean, windowReason: string | null, text: AiwsText): Facts {
  const canUpdate = Boolean(w.entity?.capabilities.includes('update'))
  const writeReason = windowReason ?? (readOnly || mode !== 'edit' ? text.t('aiworkspace.wishcard.reason.readOnly') : !canUpdate ? text.t('aiworkspace.blocks.noPermission') : null)
  const payload = w.payload
  return {
    canUpdate, writeReason,
    results: payload?.history?.results_count ?? 0,
    hasAnalysis: Boolean(payload?.analysis),
    hasMacro: Boolean(payload?.program),
    needsAnalysis: w.freshness?.needs_analysis ?? !payload?.analysis,
    needsCoding: w.freshness?.needs_coding?.reasons?.length ? w.freshness.needs_coding.reasons : null,
    executorOk: !payload || payload.executor === 'xllm',
  }
}

/** Which tabs have something to show (a failed record counts: it says why), or are the stage under way. */
function enabledTabs(w: WishState, f: Facts): Record<WishTab, boolean> {
  const history = w.payload?.history
  return {
    prompt: true,
    analysis: f.hasAnalysis || stageRecords(history, 'analyze').length > 0 || w.runningStage === 'analyze',
    macro: f.hasMacro || stageRecords(history, 'code').length > 0 || w.runningStage === 'code',
    results: f.results > 0 || stageRecords(history, 'run').length > 0 || w.runningStage === 'run',
  }
}

/** Where the card stands by default (§3.2): with the kernel's next step, the stage under way. */
function defaultTab(w: WishState, f: Facts, enabled: Record<WishTab, boolean>): WishTab {
  let tab: WishTab
  if (w.runningStage) tab = STAGE_TAB[w.runningStage]
  else switch (w.main?.next_action) {
    case 'analyze': tab = 'prompt'; break
    case 'code': tab = 'analysis'; break
    case 'run': tab = f.results > 0 ? 'results' : 'macro'; break
    default: tab = f.results > 0 || stageRecords(w.payload?.history, 'run').length > 0 ? 'results' : f.hasMacro ? 'macro' : f.hasAnalysis ? 'analysis' : 'prompt'
  }
  for (let i = TABS.indexOf(tab); i > 0; i--) if (enabled[TABS[i]]) return TABS[i]
  return 'prompt'
}

interface NextStep { kind: 'analyze' | 'code' | 'repair' | 'run' | 'cancel' | 'resume'; label: string; primary: boolean; reason: string | null; go: () => void }

/** The bottom-right button of a tab (§3.2): the stage that tab leads to, its name as things stand, primary only when it is
 * the kernel's next step, greyed with the reason when it cannot run; "取消" while a stage runs, "继续" for an interrupted
 * run this browser can resume. */
function nextStep(tab: WishTab, w: WishState, f: Facts, store: WorkspaceStore, wishId: string, cellId: string | undefined, text: AiwsText): NextStep {
  const { t } = text
  const run = w.run
  const blocked = f.writeReason ?? (!f.executorOk ? t('aiworkspace.wishcard.status.noExecutor') : null)
  if (w.runningStage) {
    return { kind: 'cancel', label: t('aiworkspace.common.cancel'), primary: false, reason: w.active ? f.writeReason : t('aiworkspace.wishcard.reason.starting'), go: () => { if (w.active) void store.wish.cancel(w.active).catch(() => undefined) } }
  }
  if (run && store.wish.canResume(run)) {
    return { kind: 'resume', label: t('aiworkspace.wishcard.next.resume'), primary: true, reason: blocked, go: () => { void store.wish.resume(run).catch(() => undefined) } }
  }
  const next = w.main?.next_action
  if (tab === 'prompt') {
    const again = f.hasAnalysis && !f.needsAnalysis
    return { kind: 'analyze', label: t(again ? 'aiworkspace.wishcard.next.reanalyze' : 'aiworkspace.wishcard.next.analyze'), primary: next === 'analyze', reason: blocked,
      go: () => { void store.wish.analyze(wishId, cellId).catch(() => undefined) } }
  }
  if (tab === 'analysis') {
    const failed = latestRecord(w.payload?.history)
    const repair = failed && !failed.ok && failed.stage !== 'analyze' && isCodingFailure(failed.error) ? failed.id : run?.state === 'failed' && run.stage !== 'analyze' && isCodingFailure(run.error) ? run.run_id : null
    const label = repair ? 'aiworkspace.wishcard.next.repair' : f.hasMacro && !f.needsCoding ? 'aiworkspace.wishcard.next.recode' : 'aiworkspace.wishcard.next.code'
    const reason = blocked ?? (f.needsAnalysis ? t(f.hasAnalysis ? 'aiworkspace.wishcard.reason.requestChanged' : 'aiworkspace.wishcard.reason.analyzeFirst') : null)
    return { kind: repair ? 'repair' : 'code', label: t(label), primary: next === 'code', reason,
      go: () => { void store.wish.code(wishId, { cellId, parentRunId: repair ?? undefined }).catch(() => undefined) } }
  }
  const reason = blocked ?? (f.needsAnalysis ? t('aiworkspace.wishcard.reason.analysisStale')
    : !f.hasMacro ? t('aiworkspace.wishcard.reason.codeFirst')
      : f.needsCoding ? t('aiworkspace.wishcard.reason.recodeFirst', { reason: needsCodingReasonText(f.needsCoding[0]) }) : null)
  return { kind: 'run', label: t('aiworkspace.wishcard.next.run'), primary: next === 'run', reason, go: () => { void store.wish.runMacro(wishId, { cellId }).catch(() => undefined) } }
}

/** Why the run button cannot run the macro now (§3.6), or null. */
function runReason(w: WishState, f: Facts, readOnly: boolean, windowReason: string | null, text: AiwsText): string | null {
  const { t } = text
  if (windowReason) return windowReason
  if (readOnly) return t('aiworkspace.wishcard.reason.readOnly')
  if (!f.canUpdate) return t(w.entity?.capabilities.includes('run') ? 'aiworkspace.wish.runOnlyUnavailable' : 'aiworkspace.blocks.noPermission')
  if (!f.executorOk) return t('aiworkspace.wishcard.status.noExecutor')
  if (w.runningStage && w.runningStage !== 'run') return t('aiworkspace.wishcard.reason.busy')
  if (f.needsAnalysis) return t('aiworkspace.wishcard.reason.analysisStale')
  if (f.needsCoding) return t('aiworkspace.wishcard.reason.recodeFirst', { reason: needsCodingReasonText(f.needsCoding[0]) })
  return null
}

/** The request, edited in place where the user may; the first change of an edit says what it makes out of date (§3.5). */
function PromptTab({ wishId, w, canEdit, cellId }: { wishId: string; w: WishState; canEdit: boolean; cellId?: string }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const invalidation = useInvalidation(w.payload, w.freshness)
  const [asking, setAsking] = useState(false)
  // Esc gives the edit up: the blur it causes saves nothing
  const discard = useRef(false)
  // a wish inserted a moment ago gets its request typed at once (the insert's "end" intent)
  const intent = useIntent(cellId ?? '')
  const [focus, setFocus] = useState<HTMLTextAreaElement | null>(null)
  useEffect(() => {
    if (!cellId || !intent || intent.value !== 'end' || !focus || !canEdit) return
    consumeIntent(cellId, intent)
    focus.focus()
  }, [cellId, intent, focus, canEdit])
  const prompt = w.payload?.prompt ?? ''
  const change = async (value: string, field: HTMLTextAreaElement) => {
    if (draft !== null) { setDraft(value); return }
    if (asking) return
    setAsking(true)
    const go = await invalidation.ask('request')
    setAsking(false)
    if (!go) return
    setDraft(value)
    field.focus()
  }
  const save = async () => {
    if (discard.current) { discard.current = false; setDraft(null); return }
    if (draft === null || draft === prompt) { setDraft(null); return }
    try {
      await setWishKey(store, wishId, w.keyRevs, 'prompt', draft, t('aiworkspace.wish.edit.prompt'))
      setDraft(null)
      setError(null)
    } catch (e) { setError(e instanceof Error ? e.message : String(e)) }
  }
  if (!canEdit) return <div className="aiws-wc-prompt-text" data-testid="aiws-wish-prompt">{prompt || <span className="aiws-muted">{t('aiworkspace.wishcard.prompt.empty')}</span>}</div>
  return (
    <>
      <textarea ref={setFocus} className="aiws-wc-prompt" data-testid="aiws-wish-prompt" aria-label={t('aiworkspace.wishcard.tab.prompt')} placeholder={t('aiworkspace.wishcard.prompt.placeholder')}
        value={draft ?? prompt} onChange={(event) => { void change(event.target.value, event.target) }} onBlur={() => { void save() }}
        onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); discard.current = true; setDraft(null); event.currentTarget.blur() } }} />
      {error && <div className="aiws-error" role="alert">{error}</div>}
      {invalidation.node}
    </>
  )
}

export interface WishCardProps {
  wishId: string
  /** The Block it is shown by (the task location of a run, the intents of an insert). */
  cellId?: string
  /** `view`: tabs and records can be read, nothing changed; a macro that can run still runs. */
  mode: 'edit' | 'view'
  /** Nothing can be changed here (embedded, a stage that does not run). */
  readOnly: boolean
  /** Small on screen: the title, the line of state and the run button only. */
  compact?: boolean
}

export function WishCard({ wishId, cellId, mode, readOnly, compact = false }: WishCardProps) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const text = useAiwsText()
  const { t } = text
  const windowReason = useReadOnlyReason()
  const w = useWish(wishId)
  const cell = useEntity(cellId ?? '')
  const f = factsOf(w, mode, readOnly, windowReason, text)
  const enabled = enabledTabs(w, f)
  const [chosen, setChosen] = useState<WishTab | null>(null)
  // a stage that ends (a new record, from this window or another) shows its product: its tab (§3.2), also when it failed;
  // a stage this window starts shows the tab it works on
  const latestId = latestRecord(w.payload?.history)?.id ?? null
  const [seen, setSeen] = useState<{ record: string | null; stage: WishStage | undefined } | null>(null)
  if (w.payload && (seen === null || seen.record !== latestId || seen.stage !== w.runningStage)) {
    const latest = latestRecord(w.payload.history)
    if (seen !== null && seen.record !== latestId && latest) setChosen(STAGE_TAB[latest.stage])
    else if (seen !== null && w.runningStage && seen.stage !== w.runningStage) setChosen(STAGE_TAB[w.runningStage])
    setSeen({ record: latestId, stage: w.runningStage })
  }
  const tab = chosen && enabled[chosen] ? chosen : defaultTab(w, f, enabled)
  const title = cell?.title ?? w.entity?.title ?? w.payload?.title ?? w.entity?.name ?? t('aiworkspace.wish.blockTitle')
  const status = w.readError ? { kind: 'failed' as const, text: t('aiworkspace.wish.readFailed', { error: w.readError }) } : statusLine(w, store, text)
  const program = w.payload?.program
  const showRun = Boolean(program) && (!w.freshness || holdsMacro(w.entity, w.freshness))
  const ai = macroUsesAi(program)
  const runningRun = w.runningStage === 'run'
  const blockedRun = runReason(w, f, readOnly, windowReason, text)
  const runNow = () => {
    if (!w.entity) return
    void runMacro(store, { wishId, title, cellId: cellId ?? null, onSurface: true, busy: false }, ui.openEntity)
  }
  const runButton = showRun && (
    <button type="button" className={`aiws-wc-run${ai ? ' is-ai' : ''}${runningRun ? ' is-running' : ''}`} data-testid="aiws-wish-run" data-ai={ai ? 'true' : 'false'}
      data-state={runningRun ? 'running' : blockedRun ? 'disabled' : 'ready'}
      disabled={runningRun ? !w.active || !f.canUpdate || windowReason !== null : blockedRun !== null}
      aria-label={runningRun ? t('aiworkspace.wishcard.stop') : blockedRun ?? t('aiworkspace.wishcard.run')} title={runningRun ? t('aiworkspace.wishcard.stop') : blockedRun ?? t('aiworkspace.wishcard.run')}
      onClick={() => { if (runningRun) { if (w.active) void store.wish.cancel(w.active).catch(() => undefined) } else runNow() }}>
      {runningRun ? <CircleStop size={16} aria-hidden="true" /> : <Play size={16} aria-hidden="true" />}
    </button>
  )
  const head = (
    <div className="aiws-wc-head">
      <Sparkles size={14} className="aiws-wc-icon" aria-hidden="true" />
      <b className="aiws-wc-title" data-testid="aiws-wish-title" title={title}>{title}</b>
      {runButton}
    </div>
  )
  const state = <div className={`aiws-wc-status is-${status.kind}`} data-testid="aiws-wish-status" data-kind={status.kind} role={status.kind === 'failed' ? 'alert' : undefined} title={status.text}>{status.text}</div>
  const common = {
    className: `aiws-wish-card aiws-wc${compact ? ' is-compact' : ''}`, 'data-testid': `aiws-wish-card-${wishId}`,
    'data-tab': tab, 'data-main': w.main?.main ?? '', 'data-next': w.main?.next_action ?? '', 'data-running': w.runningStage ?? '', 'data-mode': mode,
  }
  if (compact) return <div {...common}>{head}{state}</div>
  const canEdit = mode === 'edit' && f.writeReason === null && f.executorOk
  const next = mode === 'edit' && f.canUpdate ? nextStep(tab, w, f, store, wishId, cellId, text) : null
  const stale: Record<WishTab, boolean> = {
    prompt: false,
    analysis: f.hasAnalysis && f.needsAnalysis,
    macro: f.hasMacro && (f.needsAnalysis || Boolean(f.needsCoding)),
    results: f.results > 0 && Boolean(w.main?.stale),
  }
  let body: ReactNode
  if (tab === 'prompt') body = <PromptTab wishId={wishId} w={w} canEdit={canEdit} cellId={cellId} />
  else {
    const stage = TAB_STAGE[tab]
    body = (
      <>
        {w.runningStage === stage && <div className="aiws-wc-working" data-testid="aiws-wish-working">{t(STAGE_RUNNING[stage])}{runDoing(w.active, text) ? ` · ${runDoing(w.active, text)}` : ''}</div>}
        {tab === 'results' && stale.results && <div className="aiws-warning aiws-wc-stale" data-testid="aiws-wish-results-stale">{t('aiworkspace.wishcard.status.withReason', { state: t('aiworkspace.wish.main.stale'), reason: staleReason(w.freshness, text) ?? t('aiworkspace.wish.status.stale') })}</div>}
        <WishRecords wishId={wishId} stage={stage} readOnly={mode !== 'edit' || readOnly} />
      </>
    )
  }
  return (
    <div {...common}>
      {head}
      {state}
      <div className="aiws-wc-body" data-testid="aiws-wish-body" role="tabpanel" aria-label={t(TAB_LABEL[tab])}>{body}</div>
      <div className="aiws-wc-foot">
        <div className="aiws-wc-tabs" role="tablist" aria-label={t('aiworkspace.wishcard.tabs')}>
          {TABS.map((item) => (
            <button key={item} type="button" role="tab" className={`aiws-wc-tab${stale[item] ? ' is-stale' : ''}`} aria-selected={tab === item} disabled={!enabled[item]}
              data-testid={`aiws-wish-tab-${item}`} data-stale={stale[item] ? 'true' : 'false'} title={stale[item] ? t('aiworkspace.wishcard.tab.stale') : undefined}
              onClick={() => setChosen(item)}>
              {item === 'results' ? t('aiworkspace.wishcard.tab.resultsCount', { count: f.results }) : t(TAB_LABEL[item])}
              {stale[item] && <span className="aiws-wc-stale-dot" aria-hidden="true" />}
            </button>
          ))}
        </div>
        {next && (
          <div className="aiws-wc-next-wrap">
            {next.reason && next.kind !== 'cancel' && <span className="aiws-wc-reason" data-testid="aiws-wish-next-reason" title={next.reason}>{next.reason}</span>}
            <button type="button" className={`aiws-wc-next${next.primary ? ' is-primary' : ''}`} data-testid="aiws-wish-next" data-action={next.kind} data-primary={next.primary ? 'true' : 'false'}
              disabled={next.reason !== null} title={next.reason ?? undefined} onClick={next.go}>{next.label}</button>
          </div>
        )}
      </div>
    </div>
  )
}

