/* A wish's records (许愿格交互升级 §3.4): one entry per run of a stage, newest first — the newest unfolded, the earlier
 * ones a line each that unfolds on a click. An entry says when, from what, which step, how long, what it used and how it
 * went, warnings first and in bold; what it read and made are links. The newest entry's links open what is there now
 * (its Block on this canvas); an earlier one's open what it was then (the version that run left or read). The record in
 * effect of each stage can be rolled back by whoever may update the wish. */

import { useCallback, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Undo2 } from 'lucide-react'
import type { WishRecord, WishStage } from '../../api/types'
import { useReadOnlyReason, useStore, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { useConfirm } from './confirm'
import { AnalysisDialog, ProgramDialog, TextDialog, VersionDialog, versionOf } from './dialogs'
import { allRecords, currentRecord, describeRecord, findRecord, shortDigest, STAGES, stageRecords, whenText, actionText } from './records'
import { useWish, type WishState } from './useWish'
import './wish.css'

type Dialog =
  | { kind: 'prompt' | 'log'; objectId: string }
  | { kind: 'analysis'; objectId: string }
  | { kind: 'program'; sourceId: string | undefined; current: boolean }
  | { kind: 'version'; entityId: string; title: string; runId?: string; rev?: number }

const CHECK_STATUS: Record<string, string> = {
  passed: 'aiworkspace.wish.check.passed', failed: 'aiworkspace.wish.check.failed', not_run: 'aiworkspace.wish.check.notRun', review: 'aiworkspace.wish.check.review',
}

/** A name in a record: a link where it leads somewhere, greyed with why where it does not. */
function RecordLink({ kind, label, onOpen, gone }: { kind: string; label: string; onOpen?: () => void; gone?: string | null }) {
  if (!onOpen || gone) return <span className="aiws-wr-name is-gone" data-testid="aiws-wish-record-link" data-kind={kind} data-gone="true" title={gone ?? undefined}>{label}</span>
  return <button type="button" className="aiws-link aiws-wr-name" data-testid="aiws-wish-record-link" data-kind={kind} onClick={onOpen}>{label}</button>
}

/** A Block of `entityId` to show: one on the canvas in front first (`cells` the run named), else any; else the object. */
function jumpTarget(store: WorkspaceStore, entityId: string, cells?: string[]): string {
  const surface = store.canvasFocus.surfaceId
  const onSurface = (id: string) => !surface || store.outline.ancestors(id).includes(surface)
  const live = (id: string) => { const e = store.outline.get(id); return Boolean(e && !e.deleted) }
  const named = (cells ?? []).filter(live)
  const views = named.length ? named : store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && !e.deleted && e.source_id === entityId).map((e) => e.entity_id)
  return views.find(onSurface) ?? views[0] ?? entityId
}

function RecordItem({ record, w, newest, current, open, onToggle, canChange, onDialog, onRollback }: {
  record: WishRecord
  w: WishState
  newest: boolean
  current: boolean
  open: boolean
  onToggle: () => void
  canChange: boolean
  onDialog: (dialog: Dialog) => void
  onRollback: (record: WishRecord) => void
}) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const text = useAiwsText()
  const { t, tNode } = text
  const parts = describeRecord(record, text)
  const history = w.payload?.history
  const canUpdate = Boolean(w.entity?.capabilities.includes('update'))
  const entityGone = (id: string) => { const e = store.outline.get(id); return !e || e.deleted ? t('aiworkspace.wishcard.record.gone') : !e.capabilities.includes('read') ? t('aiworkspace.wishcard.record.unreadable') : null }
  const nameOf = (id: string, fallback?: string) => fallback ?? store.outline.get(id)?.title ?? store.outline.get(id)?.name ?? id
  // a result or input of an earlier run: the version it was, unless that is the version now
  const openThen = async (entityId: string, title: string, match: { runId?: string; rev?: number }, cells?: string[]) => {
    try {
      const found = await versionOf(store, entityId, (v) => (match.runId ? v.run_id === match.runId : v.content_rev === match.rev))
      if (found && !found.current) { onDialog({ kind: 'version', entityId, title, ...match }); return }
      if (!found && match.rev !== undefined) store.notify('info', t('aiworkspace.wishcard.record.changedSince', { name: title }))
    } catch { /* no versions to read: what is there now */ }
    ui.openEntity(jumpTarget(store, entityId, cells))
  }
  const openNow = (entityId: string, cells?: string[]) => ui.openEntity(jumpTarget(store, entityId, cells))
  const basis = (): ReactNode => {
    if (record.stage === 'analyze') return <RecordLink kind="prompt" label={t('aiworkspace.wishcard.record.basisPrompt')} onOpen={record.objects.prompt ? () => onDialog({ kind: 'prompt', objectId: record.objects.prompt! }) : undefined} />
    if (record.stage === 'code') {
      const analysis = findRecord(history, record.basis.analysis)?.objects.analysis
      return <RecordLink kind="analysis" label={t('aiworkspace.wishcard.record.basisAnalysis')} onOpen={analysis ? () => onDialog({ kind: 'analysis', objectId: analysis }) : undefined} />
    }
    const source = findRecord(history, record.basis.program_record)?.objects.source
    const isCurrent = Boolean(record.basis.program) && w.payload?.program?.digest === record.basis.program
    return <RecordLink kind="program" label={t('aiworkspace.wishcard.record.macro', { digest: shortDigest(record.basis.program) })}
      onOpen={source || isCurrent ? () => onDialog({ kind: 'program', sourceId: source, current: isCurrent }) : undefined} />
  }
  const rolledBackOf = record.action === 'rollback' ? findRecord(history, record.rollback_of ?? record.summary.of) : undefined
  const sentence = record.action === 'rollback'
    ? tNode('aiworkspace.wishcard.record.rollbackSentence', {
      target: rolledBackOf ? t('aiworkspace.wishcard.record.rollbackTarget', { when: whenText(rolledBackOf.at, text), action: actionText(rolledBackOf.action, text) }) : t('aiworkspace.wishcard.record.rollbackTargetGone'),
      metrics: parts.metrics,
    })
    : tNode('aiworkspace.wishcard.record.sentence', { basis: basis(), action: parts.action, metrics: parts.metrics })
  const outcome = (
    <span className={`aiws-wr-outcome${record.ok ? '' : ' is-failed'}`}>
      {parts.status}{t('aiworkspace.wishcard.record.colon')}
      {parts.warnings.length > 0 && <b className="aiws-wr-warnings" data-testid="aiws-wish-record-warnings">{text.clauses(parts.warnings)} </b>}
      {parts.outcome}
    </span>
  )
  const inputs = record.inputs.map((input) => {
    const title = nameOf(input.entity_id, input.label ?? input.name)
    const gone = entityGone(input.entity_id)
    return <RecordLink key={input.entity_id} kind="input" label={title} gone={gone}
      onOpen={() => { if (newest) openNow(input.entity_id); else void openThen(input.entity_id, title, { rev: input.version?.rev }) }} />
  })
  const outputs: ReactNode[] = []
  if (record.stage === 'analyze' && record.ok && record.objects.analysis) outputs.push(<RecordLink key="analysis" kind="analysis" label={t('aiworkspace.wishcard.record.contract')} onOpen={() => onDialog({ kind: 'analysis', objectId: record.objects.analysis! })} />)
  if (record.stage === 'code' && record.ok && record.outputs.program) {
    const isCurrent = w.payload?.program?.source === record.objects.source && Boolean(record.objects.source)
    outputs.push(<RecordLink key="program" kind="program" label={t('aiworkspace.wishcard.record.macro', { digest: shortDigest(record.outputs.program.digest) })}
      onOpen={record.objects.source || isCurrent ? () => onDialog({ kind: 'program', sourceId: record.objects.source, current: isCurrent }) : undefined} />)
  }
  for (const r of record.outputs.results ?? []) {
    const title = r.title ?? r.name
    outputs.push(<RecordLink key={`r:${r.name}`} kind="result" label={title} gone={entityGone(r.entity_id)}
      onOpen={() => { if (newest) openNow(r.entity_id, r.cells); else void openThen(r.entity_id, title, { runId: record.id }, r.cells) }} />)
  }
  const extras: ReactNode[] = []
  if (record.objects.prompt && record.stage !== 'analyze') extras.push(<RecordLink key="prompt" kind="prompt" label={t('aiworkspace.wishcard.record.prompt')} onOpen={() => onDialog({ kind: 'prompt', objectId: record.objects.prompt! })} />)
  if (record.objects.log) extras.push(<RecordLink key="log" kind="log" label={t('aiworkspace.wishcard.record.log')} gone={canUpdate ? null : t('aiworkspace.wishcard.record.logNeedsUpdate')} onOpen={() => onDialog({ kind: 'log', objectId: record.objects.log! })} />)
  const checks = (record.checks ?? []).filter((c) => c.status !== 'passed')
  return (
    <li className={`aiws-wr-item${open ? ' is-open' : ''}${record.ok ? '' : ' is-failed'}`} data-testid="aiws-wish-record" data-record-id={record.id} data-stage={record.stage} data-action={record.action}
      data-ok={record.ok ? 'true' : 'false'} data-current={current ? 'true' : 'false'} data-rolled-back={record.rolled_back ? 'true' : 'false'} data-expanded={open ? 'true' : 'false'}>
      <div className="aiws-wr-head">
        <button type="button" className="aiws-wr-toggle" data-testid="aiws-wish-record-toggle" aria-expanded={open} onClick={onToggle}>
          {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}
          <b className="aiws-wr-when">{parts.when}</b>
          {!open && <span className="aiws-wr-line">{parts.action} · {parts.status}{parts.warnings.length ? ` · ${parts.warnings[0]}` : ` · ${parts.outcome}`}</span>}
        </button>
        {record.rolled_back && <span className="aiws-chip" data-testid="aiws-wish-record-rolled-back">{t('aiworkspace.wishcard.record.rolledBack')}</span>}
        {current && canChange && (
          <button type="button" className="aiws-wr-rollback" data-testid="aiws-wish-rollback" title={t('aiworkspace.wishcard.rollback.hint')} onClick={() => onRollback(record)}>
            <Undo2 size={14} aria-hidden="true" />{t('aiworkspace.wishcard.rollback.button')}
          </button>
        )}
      </div>
      {open && (
        <div className="aiws-wr-body">
          <div className="aiws-wr-sentence">{sentence}{outcome}</div>
          {record.summary.text && <div className="aiws-wr-model-text" data-testid="aiws-wish-record-summary">{record.summary.text}</div>}
          {checks.length > 0 && (
            <ul className="aiws-wr-checks">
              {checks.map((c) => <li key={c.id} data-status={c.status} data-level={c.level ?? ''}><span className={`aiws-chip${c.status === 'failed' ? ' aiws-chip-warn' : ''}`}>{t(CHECK_STATUS[c.status] ?? CHECK_STATUS.not_run)}</span> {c.text}</li>)}
            </ul>
          )}
          {inputs.length > 0 && <div className="aiws-wr-names">{tNode('aiworkspace.wishcard.record.inputs', { names: <span className="aiws-wr-list">{inputs}</span> })}</div>}
          {outputs.length > 0 && <div className="aiws-wr-names">{tNode(record.stage === 'run' ? 'aiworkspace.wishcard.record.results' : 'aiworkspace.wishcard.record.outputs', { names: <span className="aiws-wr-list">{outputs}</span> })}</div>}
          {extras.length > 0 && <div className="aiws-wr-names aiws-wr-list">{extras}</div>}
          {record.aicc && <div className="aiws-muted" data-testid="aiws-wish-aicc" data-user={record.aicc.user_id}>{t(record.aicc.mode === 'current_user' ? 'aiworkspace.wish.aicc.runStarter' : 'aiworkspace.wish.aicc.runOwner', { user: record.aicc.user_id })}</div>}
          {record.rolled_back && <div className="aiws-muted">{t('aiworkspace.wishcard.record.rolledBackBy', { by: record.rolled_back.by, when: whenText(record.rolled_back.at, text) })}</div>}
          {!record.ok && record.error?.missing_inputs?.length ? <div className="aiws-muted">{t('aiworkspace.wishcard.record.bindHint')}</div> : null}
          {record.by && <div className="aiws-muted aiws-wr-by">{t('aiworkspace.wishcard.record.by', { by: record.by })}</div>}
        </div>
      )}
    </li>
  )
}

export interface WishRecordsProps {
  wishId: string
  /** One stage's records, or all of them (the data mode's "运行记录"). */
  stage: WishStage | 'all'
  /** Nothing can be changed here (view mode): no rollback. */
  readOnly?: boolean
}

export function WishRecords({ wishId, stage, readOnly = false }: WishRecordsProps) {
  const store = useStore()
  const text = useAiwsText()
  const { t } = text
  const windowReason = useReadOnlyReason()
  const w = useWish(wishId)
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(new Set())
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set())
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const { node: confirmNode, confirm } = useConfirm()
  const history = w.payload?.history
  const records = stage === 'all' ? allRecords(history) : stageRecords(history, stage)
  const newest = new Set(STAGES.map((s) => stageRecords(history, s)[0]?.id).filter((id): id is string => Boolean(id)))
  const current = new Set(STAGES.map((s) => currentRecord(stageRecords(history, s))?.id).filter((id): id is string => Boolean(id)))
  const canChange = !readOnly && !windowReason && Boolean(w.entity?.capabilities.includes('update'))
  const rollback = useCallback(async (record: WishRecord) => {
    try {
      const first = await store.wish.rollback(wishId, record)
      if (first.status !== 'needs_confirm') return
      // a result is named as the record names it (its contract's title), not by its entity name
      const named = new Map((record.outputs.results ?? []).map((r) => [r.entity_id, r.title ?? r.name]))
      const items = first.lost.map((l) => t(l.what === 'deleted' ? 'aiworkspace.wishcard.rollback.lostDeleted' : 'aiworkspace.wishcard.rollback.lostEdited', { name: named.get(l.entity_id) ?? l.title ?? store.outline.get(l.entity_id)?.title ?? store.outline.get(l.entity_id)?.name ?? l.entity_id }))
      if (first.hidden_lost) items.push(t('aiworkspace.wishcard.rollback.hiddenLost', { count: first.hidden_lost }))
      const yes = await confirm({ title: t('aiworkspace.wishcard.rollback.confirmTitle'), body: t('aiworkspace.wishcard.rollback.confirmBody'), items, ok: t('aiworkspace.wishcard.rollback.confirmOk'), cancel: t('aiworkspace.common.cancel'), testId: 'aiws-wish-rollback-confirm' })
      if (yes) await store.wish.rollback(wishId, record, true)
    } catch { /* the card's line of state says why (WishService.errors) */ }
  }, [store, wishId, t, confirm])
  if (w.readError) return <div className="aiws-error" role="alert">{t('aiworkspace.wish.readFailed', { error: w.readError })}</div>
  if (w.loading) return <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  const isOpen = (id: string, index: number) => (index === 0 ? !folded.has(id) : unfolded.has(id))
  const toggle = (id: string, index: number) => {
    const flip = (set: ReadonlySet<string>) => { const next = new Set(set); if (next.has(id)) next.delete(id); else next.add(id); return next }
    if (index === 0) setFolded(flip); else setUnfolded(flip)
  }
  const shownDialog = (() => {
    if (!dialog) return null
    const close = () => setDialog(null)
    switch (dialog.kind) {
      case 'prompt': return <TextDialog title={t('aiworkspace.wishcard.dialog.prompt')} kind="prompt" objectId={dialog.objectId} onClose={close} />
      case 'log': return <TextDialog title={t('aiworkspace.wishcard.dialog.log')} kind="log" objectId={dialog.objectId} onClose={close} />
      case 'analysis': return <AnalysisDialog objectId={dialog.objectId} onClose={close} />
      case 'program': return <ProgramDialog wishId={wishId} sourceId={dialog.sourceId} current={dialog.current} canEdit={canChange} onClose={close} />
      case 'version': return <VersionDialog title={dialog.title} entityId={dialog.entityId} match={(v) => (dialog.runId ? v.run_id === dialog.runId : v.content_rev === dialog.rev)} onClose={close} />
    }
  })()
  return (
    <div className="aiws-wr" data-testid="aiws-wish-records" data-stage={stage}>
      {records.length === 0
        ? <div className="aiws-muted" data-testid="aiws-wish-records-empty">{t('aiworkspace.wishcard.record.none')}</div>
        : (
          <ol className="aiws-wr-list-items">
            {records.map((record, index) => (
              <RecordItem key={record.id} record={record} w={w} newest={newest.has(record.id)} current={current.has(record.id)} open={isOpen(record.id, index)}
                onToggle={() => toggle(record.id, index)} canChange={canChange} onDialog={setDialog} onRollback={(r) => { void rollback(r) }} />
            ))}
          </ol>
        )}
      {shownDialog}
      {confirmNode}
    </div>
  )
}
