import { useMemo, useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { useSWRConfig } from 'swr'
import { DECISION_LABEL, DECISION_TONE, LIFECYCLE_LABEL, OUTCOME_TONE, STATUS_LABEL, STATUS_TONE, errorText, lastSkipReason } from '../goals'
import { t, type TextKey } from '../i18n'
import { fmtDuration, fmtTokens, goalExtension, useQuery } from '../lib'
import {
  dataModel,
  type GoalActivation,
  type GoalDefinition,
  type GoalSchedule,
  type GoalState,
  type MaintenanceGate,
  type MaintenanceState,
} from '../model'
import { Badge, Empty, ErrorBox, Fields, Mono, Panel, Pre, Sid, Time } from '../ui'
import { NextOpportunity } from './Goals'

const ACTIVATIONS = 200
const CHANGES = 50

const list = (items?: string[]): ReactNode => (items?.length ? <ul className="list-disc pl-4 text-xs">{items.map((x) => <li key={x}>{x}</li>)}</ul> : null)

interface DialogSpec {
  title: string
  hint: string
  confirmLabel: string
  reasonRequired?: boolean
  option?: TextKey
  danger?: boolean
  run: (reason: string, option: boolean) => Promise<string>
}

function GoalDialog({ spec, onClose, onDone }: { spec: DialogSpec; onClose: () => void; onDone: (what: string) => void }) {
  const [reason, setReason] = useState('')
  const [option, setOption] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const confirm = async () => {
    if (spec.reasonRequired && !reason.trim()) {
      setError(new Error(t('opReasonRequired')))
      return
    }
    setBusy(true)
    setError(null)
    try {
      onDone(await spec.run(reason.trim(), option))
      onClose()
    } catch (e) {
      setError(e)
      setBusy(false)
    }
  }
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4">
      <form
        role="dialog"
        aria-modal="true"
        aria-label={spec.title}
        className="w-full max-w-md rounded border border-line bg-panel p-4"
        onSubmit={(e) => {
          e.preventDefault()
          void confirm()
        }}
      >
        <h3 className="mb-2 text-sm font-semibold">{spec.title}</h3>
        <p className="mb-3 text-xs text-mute">{spec.hint}</p>
        <label className="mb-3 block text-xs text-mute">
          {t(spec.reasonRequired ? 'opReason' : 'opReasonOptional')}
          <input className="input mt-1 w-full" value={reason} onChange={(e) => setReason(e.target.value)} />
        </label>
        {spec.option && (
          <label className="mb-3 flex items-center gap-2 text-xs">
            <input type="checkbox" checked={option} onChange={(e) => setOption(e.target.checked)} />
            {t(spec.option)}
          </label>
        )}
        <ErrorBox error={error} />
        <div className="flex justify-end gap-2">
          <button type="button" className="btn" onClick={onClose} disabled={busy}>
            {t('cancel')}
          </button>
          <button type="submit" className={clsx('btn', spec.danger ? 'border-bad text-bad' : 'border-accent text-accent')} disabled={busy}>
            {spec.confirmLabel}
          </button>
        </div>
      </form>
    </div>
  )
}

function Operations({ goal, schedule, onDone }: { goal: GoalDefinition; schedule?: GoalSchedule | null; onDone: () => void }) {
  const [spec, setSpec] = useState<DialogSpec | null>(null)
  const [done, setDone] = useState('')
  const ext = goalExtension(goal)
  const builtin = Boolean(ext?.builtin)
  const live = goal.status === 'active' || goal.status === 'paused'
  const id = goal.goal_id
  const vars = { title: goal.title }
  const setStatus = (status: 'paused' | 'active' | 'archived', title: TextKey, hint: TextKey, label: TextKey, danger = false) =>
    setSpec({
      title: t(title, vars),
      hint: t(hint, vars),
      confirmLabel: t(label),
      danger,
      reasonRequired: status === 'archived',
      run: async (reason) => t('opStatusSet', { status: t(STATUS_LABEL[(await dataModel.setGoalStatus(id, status, reason || undefined)).status]) }),
    })
  return (
    <Panel title={t('goalOperations')} testId="goal-operations">
      {builtin && (
        <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs" data-testid="goal-builtin-info">
          <Badge value={t('goalSystemBuiltin')} tone="running" />
          <span className="text-mute">
            {t('goalPolicyVersion')}: <Mono>{ext?.policy_version ?? 0}</Mono>
          </span>
          <span className="text-mute">
            {t('goalConfigSwitch')}:{' '}
            <span className={schedule?.gate?.status === 'disabled' ? 'text-bad' : 'text-ok'} data-testid="goal-config-switch">
              {t(schedule?.gate?.status === 'disabled' ? 'switchOff' : 'switchOn')}
            </span>
          </span>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {live && (
          <button
            className="btn border-accent text-accent"
            onClick={() =>
              setSpec({
                title: t('opActivateTitle', vars),
                hint: t('opActivateHint'),
                confirmLabel: t('opActivate'),
                run: async (reason) => t('opActivated', { index: (await dataModel.activateGoal(id, reason || undefined)).index }),
              })
            }
          >
            {t('opActivate')}
          </button>
        )}
        {goal.status === 'active' && (
          <button className="btn" onClick={() => setStatus('paused', 'opPauseTitle', 'opPauseHint', 'opPause')}>
            {t('opPause')}
          </button>
        )}
        {goal.status === 'paused' && (
          <button className="btn border-ok text-ok" onClick={() => setStatus('active', 'opResumeTitle', 'opPauseHint', 'opResume')}>
            {t('opResume')}
          </button>
        )}
        {live && (
          <button
            className="btn border-bad text-bad"
            disabled={builtin}
            title={builtin ? t('opArchiveBuiltinHint') : undefined}
            onClick={() => setStatus('archived', 'opArchiveTitle', 'opArchiveHint', 'opArchive', true)}
          >
            {t('opArchive')}
          </button>
        )}
        {live && (
          <button
            className="btn"
            onClick={() =>
              setSpec({
                title: t('opRestartTitle', vars),
                hint: t('opRestartHint'),
                confirmLabel: t('opRestart'),
                reasonRequired: true,
                option: 'opCarryState',
                run: async (reason, carry) => t('opEpochOpened', { epoch: (await dataModel.restartGoal(id, reason, carry)).session.epoch }),
              })
            }
          >
            {t('opRestart')}
          </button>
        )}
        {!live && (
          <button
            className="btn border-accent text-accent"
            onClick={() =>
              setSpec({
                title: t('opReopenTitle', vars),
                hint: t('opReopenHint'),
                confirmLabel: t('opReopen'),
                reasonRequired: true,
                option: 'opFreshState',
                run: async (reason, fresh) => t('opEpochOpened', { epoch: (await dataModel.reopenGoal(id, reason, fresh)).session.epoch }),
              })
            }
          >
            {t('opReopen')}
          </button>
        )}
        {builtin && live && <span className="text-xs text-mute">{t('opArchiveBuiltinHint')}</span>}
      </div>
      {done && (
        <div className="mt-2 text-xs text-ok" data-testid="goal-operation-result">
          {t('opDone', { what: done })}
        </div>
      )}
      {spec && (
        <GoalDialog
          spec={spec}
          onClose={() => setSpec(null)}
          onDone={(what) => {
            setDone(what)
            onDone()
          }}
        />
      )}
    </Panel>
  )
}

function Definition({ goal }: { goal: GoalDefinition }) {
  const [rev, setRev] = useState<number | null>(null)
  const snapshot = useQuery(rev != null ? ['goals.revision', goal.goal_id, rev] : null, () => dataModel.goalRevision(goal.goal_id, rev!), false)
  const shown = (rev != null && snapshot.data) || goal
  const revisions = Array.from({ length: goal.revision }, (_, i) => goal.revision - i)
  const budget = shown.budget
  const schedule = shown.schedule
  return (
    <Panel
      title={t('goalDefinition')}
      testId="goal-definition"
      extra={
        <label className="flex items-center gap-1 text-xs text-mute">
          {t('goalRevision')}
          <select className="input" aria-label={t('goalRevision')} value={rev ?? goal.revision} onChange={(e) => setRev(Number(e.target.value) === goal.revision ? null : Number(e.target.value))}>
            {revisions.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
      }
    >
      <ErrorBox error={snapshot.error} />
      <Fields
        rows={[
          [t('goalObjective'), <span className="text-xs">{shown.objective}</span>],
          [t('goalLifecycle'), t(LIFECYCLE_LABEL[shown.lifecycle ?? 'maintain'])],
          [t('goalHealthyState'), shown.healthy_state ? <span className="text-xs">{shown.healthy_state}</span> : null],
          [t('goalAchievedWhen'), shown.achieved_when ? <span className="text-xs">{shown.achieved_when}</span> : null],
          [t('goalConstraints'), list(shown.constraints)],
          [
            t('goalScope'),
            shown.scope && (shown.scope.objects?.length || shown.scope.paths?.length || shown.scope.workspace) ? (
              <Mono>{[...(shown.scope.objects ?? []), ...(shown.scope.paths ?? []), ...(shown.scope.workspace ? [`workspace:${shown.scope.workspace}`] : [])].join(', ')}</Mono>
            ) : null,
          ],
          [
            t('goalBudget'),
            budget ? (
              <Mono>
                {[
                  budget.activation?.max_total_tokens != null && `activation ≤ ${fmtTokens(budget.activation.max_total_tokens)} tokens`,
                  budget.activation?.max_runs != null && `≤ ${budget.activation.max_runs} runs`,
                  budget.period?.max_total_tokens != null && `period ≤ ${fmtTokens(budget.period.max_total_tokens)} tokens`,
                  budget.period?.max_activations != null && `≤ ${budget.period.max_activations} activations`,
                  budget.period?.max_works != null && `≤ ${budget.period.max_works} works`,
                  budget.period?.window_secs != null && `window ${fmtDuration(budget.period.window_secs)}`,
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </Mono>
            ) : null,
          ],
          [
            t('goalScheduleSpec'),
            schedule && (schedule.every_secs || schedule.min_interval_secs || schedule.start_on_create) ? (
              <span className="text-xs">
                {[
                  schedule.every_secs ? t('goalEvery', { n: fmtDuration(schedule.every_secs) }) : '',
                  schedule.min_interval_secs ? t('goalMinInterval', { n: fmtDuration(schedule.min_interval_secs) }) : '',
                  schedule.start_on_create ? t('goalStartOnCreate') : '',
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </span>
            ) : (
              <span className="text-xs text-mute">{t('goalManualOnly')}</span>
            ),
          ],
          [t('goalBehavior'), shown.behavior ? <Mono>{shown.behavior}</Mono> : null],
          [t('goalRevision'), <Mono>{shown.revision}{shown.revision !== goal.revision ? ` / ${goal.revision}` : ''}</Mono>],
          [t('goalCreatedBy'), <Mono>{goal.created_by.principal} via {goal.created_by.via}</Mono>],
          [
            t('goalStatusField'),
            <span className="flex flex-wrap items-center gap-1">
              <Badge value={t(STATUS_LABEL[goal.status])} tone={STATUS_TONE[goal.status]} />
              {goal.status_reason && <span className="text-xs text-mute">{goal.status_reason}</span>}
              {goal.status_changed_at_ms ? <Time ms={goal.status_changed_at_ms} /> : null}
            </span>,
          ],
          [
            t('goalEpochs'),
            goal.epochs?.length ? (
              <table className="tbl">
                <tbody>
                  {goal.epochs.map((ep) => (
                    <tr key={ep.epoch} data-testid={`goal-epoch-${ep.epoch}`}>
                      <td className="w-px">#{ep.epoch}</td>
                      <td className="w-px"><Badge value={ep.start} /></td>
                      <td className="w-px"><Sid sid={ep.sid} /></td>
                      <td className="w-px"><Time ms={ep.started_at_ms} /></td>
                      <td className="text-xs">
                        {ep.end && <Badge value={ep.end} tone="finished" />} {ep.end_reason ?? ep.reason}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null,
          ],
        ]}
      />
    </Panel>
  )
}

function GoalStatePanel({ goal, state }: { goal: GoalDefinition; state?: GoalState | null }) {
  if (!state) {
    return (
      <Panel title={t('goalStateTitle')} testId="goal-state">
        <Empty>{state === null ? t('goalNoState') : t('loading')}</Empty>
      </Panel>
    )
  }
  const usage = state.usage
  const period = goal.budget?.period
  const used = (usage.tokens ?? 0) + (usage.work_tokens ?? 0)
  const remaining = (max: number | undefined, value: number) => (max != null ? ` / ${fmtTokens(max)} (${t('goalRemaining', { n: fmtTokens(Math.max(0, max - value)) })})` : '')
  return (
    <Panel title={t('goalStateTitle')} testId="goal-state" extra={<span className="text-xs text-mute">rev {state.rev} · epoch {state.epoch}</span>}>
      <Fields
        rows={[
          [
            t('goalDistance'),
            state.distance ? (
              <span className="text-xs">
                {state.distance.text}
                {state.distance.observed_at_ms ? <span className="ml-1 text-mute">· <Time ms={state.distance.observed_at_ms} /></span> : null}
                {state.distance.unknowns?.length ? (
                  <span className="block text-mute">
                    {t('goalUnknowns')}: {state.distance.unknowns.join('; ')}
                  </span>
                ) : null}
              </span>
            ) : null,
          ],
          [t('goalPending'), list(state.pending)],
          [
            t('goalQuestions'),
            state.questions?.length ? (
              <ul className="text-xs">
                {state.questions.map((q, i) => (
                  <li key={`${q.epoch}-${q.activation}-${i}`} className="mb-1" data-testid="goal-question">
                    <Badge value={q.status ?? 'open'} tone={q.status === 'answered' ? 'ok' : q.status === 'open' || !q.status ? 'waiting' : 'finished'} /> {q.text}
                    <span className="block text-mute">
                      #{q.epoch}.{q.activation} · <Time ms={q.at_ms} />
                      {q.notify_id && <> · {t('goalNotification')} <Mono>{q.notify_id}</Mono></>}
                      {q.candidate_id && <> · {t('goalCandidate')} <Mono>{q.candidate_id}</Mono></>}
                    </span>
                    {q.answer && <span className="block">{t('goalAnswer')}: {q.answer}</span>}
                  </li>
                ))}
              </ul>
            ) : null,
          ],
          [
            t('goalActiveWorks'),
            state.active_works?.length ? (
              <ul className="text-xs">
                {state.active_works.map((w) => (
                  <li key={w.sid}>
                    <Sid sid={w.sid} /> {w.objective} <span className="text-mute">#{w.epoch}.{w.created_in_activation}</span>
                  </li>
                ))}
              </ul>
            ) : null,
          ],
          [
            t('goalRecentWorks'),
            state.recent_works?.length ? (
              <ul className="text-xs">
                {state.recent_works.map((w) => (
                  <li key={w.sid}>
                    <Sid sid={w.sid} /> {w.outcome && <Badge value={w.outcome} />} {w.one_line_status}
                    <span className="text-mute"> · #{w.absorbed_in[0]}.{w.absorbed_in[1]}</span>
                  </li>
                ))}
              </ul>
            ) : null,
          ],
          [
            t('goalUsage'),
            <span className="text-xs" data-testid="goal-usage">
              {t('goalUsageLine', { tokens: fmtTokens(used), activations: usage.activations ?? 0, works: usage.works ?? 0 })}
              <span className="block text-mute">
                tokens {fmtTokens(used)}{remaining(period?.max_total_tokens, used)}
                {period?.max_activations != null && ` · activations ${usage.activations ?? 0} / ${period.max_activations}`}
                {period?.max_works != null && ` · works ${usage.works ?? 0} / ${period.max_works}`}
                {' · '}{t('goalPeriodSince', { time: new Date(usage.period_start_ms).toLocaleString() })}
              </span>
            </span>,
          ],
          [t('goalLastError'), state.last_error != null ? <Pre value={state.last_error} className="text-bad" /> : null],
        ]}
      />
    </Panel>
  )
}

function Activations({ goal, activations, error }: { goal: GoalDefinition; activations?: GoalActivation[]; error?: unknown }) {
  const groups = useMemo(() => {
    const byEpoch = new Map<number, GoalActivation[]>()
    for (const a of activations ?? []) byEpoch.set(a.epoch, [...(byEpoch.get(a.epoch) ?? []), a])
    return [...byEpoch.entries()].sort((a, b) => b[0] - a[0]).map(([epoch, rows]) => ({ epoch, rows: rows.sort((a, b) => b.seq - a.seq) }))
  }, [activations])
  return (
    <Panel title={`${t('goalActivations')} (${activations?.length ?? 0})`} testId="goal-activations">
      <ErrorBox error={error} />
      {groups.length ? (
        groups.map(({ epoch, rows }) => {
          const ep = goal.epochs?.find((e) => e.epoch === epoch)
          const sid = ep?.sid ?? rows[0].session_id
          return (
            <div key={epoch} className="mb-3" data-testid={`activations-epoch-${epoch}`}>
              <div className="mb-1 flex flex-wrap items-center gap-2 text-xs">
                <span className="font-semibold">{t('goalEpochN', { n: epoch })}</span>
                {ep && <Badge value={ep.start} />}
                {ep?.end && <Badge value={ep.end} tone="finished" />}
                <span className="text-mute">{t('goalSession')}</span> <Sid sid={sid} />
              </div>
              <table className="tbl">
                <thead>
                  <tr>
                    <th>#</th>
                    <th>{t('goalReasons')}</th>
                    <th>{t('goalOutcome')}</th>
                    <th>{t('goalDecision')}</th>
                    <th>{t('goalStarted')}</th>
                    <th>{t('goalCost')}</th>
                    <th>{t('goalSummary')}</th>
                    <th>{t('goalSession')}</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((a) => (
                    <tr key={a.seq} data-testid="activation-row">
                      <td className="w-px">{a.seq}</td>
                      <td className="text-xs">
                        {(a.reasons ?? []).map((r, i) => (
                          <div key={`${r.key}-${i}`}>
                            <Badge value={r.kind} />
                            {r.note && <span className="ml-1">{r.note}</span>}
                            {r.ref && <Mono className="ml-1 text-mute">{r.ref}</Mono>}
                            {r.missed ? <span className="ml-1 text-mute">missed {r.missed}</span> : null}
                          </div>
                        ))}
                      </td>
                      <td className="w-px"><Badge value={a.outcome} tone={OUTCOME_TONE[a.outcome]} /></td>
                      <td className="w-px"><Badge value={t(DECISION_LABEL[a.decision])} tone={DECISION_TONE[a.decision]} /></td>
                      <td className="w-px">
                        <Time ms={a.started_at_ms} />
                        {a.ended_at_ms > a.started_at_ms && <span className="block text-xs text-mute">{Math.round((a.ended_at_ms - a.started_at_ms) / 1000)}s</span>}
                      </td>
                      <td className="w-px whitespace-nowrap text-xs">
                        {t('goalRuns', { n: a.cost?.runs ?? 0 })} · {fmtTokens(a.cost?.total_tokens ?? 0)}
                      </td>
                      <td className="text-xs">
                        {a.summary}
                        {a.works_created?.length ? <span className="block text-mute">+ {a.works_created.map((w) => <Sid key={w} sid={w} />)}</span> : null}
                        {a.end_reason && <span className="block text-mute">{a.end_reason}</span>}
                      </td>
                      <td className="w-px"><Sid sid={a.session_id} /></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )
        })
      ) : (
        <Empty>{activations ? t('goalNoActivations') : t('loading')}</Empty>
      )}
    </Panel>
  )
}

function Maintenance({ gate, maintenance }: { gate?: MaintenanceGate; maintenance?: MaintenanceState }) {
  const pending = gate?.pending
  return (
    <Panel title={t('goalMaintenance')} testId="goal-maintenance">
      <div className="grid gap-3 lg:grid-cols-2">
        <div>
          <h3 className="mb-1 text-xs font-semibold text-mute">{t('gateTitle')}</h3>
          {gate ? (
            <Fields
              rows={[
                [t('goalStatusField'), <Badge value={gate.status} tone={gate.status === 'active' || gate.status === 'mechanical' ? 'running' : gate.status === 'paused' || gate.status === 'disabled' ? 'waiting' : gate.status === 'backoff' || gate.status === 'period_budget' ? 'bad' : undefined} />],
                [t('gateLastChecked'), gate.last_checked_at_ms ? <Time ms={gate.last_checked_at_ms} /> : null],
                [t('gateLastFullCheck'), gate.last_full_check_at_ms ? <Time ms={gate.last_full_check_at_ms} /> : null],
                [t('gateSkipReason'), gate.last_skip_reason ? <Mono>{gate.last_skip_reason}</Mono> : null],
                [
                  t('gatePending'),
                  pending ? (
                    <span className="text-xs">
                      {t('gatePendingLine', { new: pending.new ?? 0, due: pending.due ?? 0, waiting: pending.waiting ?? 0, retry: pending.retry_cleanup ?? 0 })}
                      {pending.next_deadline && <span className="block text-mute">{t('gateNextDeadline')}: {new Date(pending.next_deadline).toLocaleString()}</span>}
                      {pending.oldest_at_ms ? <span className="block text-mute">{t('gateOldest')}: <Time ms={pending.oldest_at_ms} /></span> : null}
                    </span>
                  ) : null,
                ],
                [t('gateDueReviews'), String(gate.due_reviews ?? 0)],
                [t('gateNextReview'), gate.next_review_at ? new Date(gate.next_review_at).toLocaleString() : null],
                [t('gateUnfinished'), String(gate.unfinished_receipts ?? 0)],
                [t('gateBackoff'), gate.backoff_until_ms ? <span><Time ms={gate.backoff_until_ms} /> (level {gate.backoff_level ?? 0})</span> : null],
                [t('goalLastError'), gate.last_error ? <span className="text-xs text-bad">{gate.last_error}</span> : null],
              ]}
            />
          ) : (
            <Empty />
          )}
        </div>
        <div>
          <h3 className="mb-1 text-xs font-semibold text-mute">{t('mntTitle')}</h3>
          {maintenance ? (
            <Fields
              rows={[
                [
                  t('goalSummary'),
                  <span className="text-xs">
                    {t('mntCounts', {
                      consolidations: maintenance.consolidations ?? 0,
                      operations: maintenance.operations ?? 0,
                      dispositions: maintenance.dispositions ?? 0,
                      abandoned: maintenance.abandoned_batches ?? 0,
                    })}
                  </span>,
                ],
                [t('mntLastSuccess'), maintenance.last_successful_consolidation_at_ms ? <Time ms={maintenance.last_successful_consolidation_at_ms} /> : null],
                [t('goalLastError'), maintenance.last_error ? <span className="text-xs text-bad">{maintenance.last_error}</span> : null],
              ]}
            />
          ) : (
            <Empty />
          )}
        </div>
      </div>
      <h3 className="mb-1 mt-3 text-xs font-semibold text-mute">{t('mntReceipts')} ({maintenance?.receipts?.length ?? 0})</h3>
      {maintenance?.receipts?.length ? (
        <table className="tbl">
          <thead>
            <tr>
              <th>operation_id</th>
              <th>stage</th>
              <th>batch</th>
              <th>#</th>
              <th>operations</th>
              <th>dispositions</th>
              <th>updated</th>
            </tr>
          </thead>
          <tbody>
            {maintenance.receipts.map((r) => (
              <tr key={r.operation_id} data-testid="receipt-row">
                <td><Mono>{r.operation_id}</Mono></td>
                <td>
                  <Badge value={r.stage} tone={r.stage === 'completed' ? 'ok' : r.stage === 'failed' ? 'bad' : 'pending'} />
                  {r.cleanup_pending && <span className="ml-1"><Badge value={t('cleanupPending')} tone="waiting" /></span>}
                </td>
                <td><Mono>{r.batch_id}</Mono></td>
                <td className="text-xs">#{r.epoch}.{r.activation}</td>
                <td>{r.operations ?? 0}</td>
                <td>{r.dispositions ?? 0}</td>
                <td><Time ms={r.updated_at_ms || r.created_at_ms} />{r.last_error && <span className="block text-xs text-bad">{r.last_error}</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <Empty>{t('mntNoReceipts')}</Empty>
      )}
      <h3 className="mb-1 mt-3 text-xs font-semibold text-mute">{t('mntCandidates')} ({maintenance?.candidates?.length ?? 0})</h3>
      {maintenance?.candidates?.length ? (
        <table className="tbl">
          <thead>
            <tr>
              <th>candidate_id</th>
              <th>kind</th>
              <th>title</th>
              <th>status</th>
              <th>updated</th>
            </tr>
          </thead>
          <tbody>
            {maintenance.candidates.map((c) => (
              <tr key={c.candidate_id} data-testid="candidate-row">
                <td><Mono>{c.candidate_id}</Mono></td>
                <td><Badge value={c.kind} /></td>
                <td className="text-xs">
                  {c.title}
                  <span className="block text-mute">{c.action} · {c.intent_basis}{c.notify_id && <> · {t('goalNotification')} <Mono>{c.notify_id}</Mono></>}</span>
                  {c.last_error && <span className="block text-bad">{c.last_error}</span>}
                </td>
                <td><Badge value={c.status} tone={c.status === 'needs_confirmation' ? 'waiting' : c.status === 'created' || c.status === 'linked' || c.status === 'approved' ? 'ok' : c.status === 'rejected' || c.status === 'blocked' ? 'bad' : undefined} /></td>
                <td><Time ms={c.updated_at_ms || c.created_at_ms} /></td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <Empty>{t('mntNoCandidates')}</Empty>
      )}
    </Panel>
  )
}

export default function GoalDetailPage({ goalId }: { goalId: string }) {
  const { mutate } = useSWRConfig()
  const goal = useQuery(['goals.get', goalId], () => dataModel.goal(goalId))
  const state = useQuery(['goals.state', goalId], () => dataModel.goalState(goalId))
  const schedule = useQuery(['goals.schedule', goalId], () => dataModel.goalSchedule(goalId))
  const activations = useQuery(['goals.activations', goalId], () => dataModel.goalActivations(goalId, undefined, ACTIVATIONS))
  const changes = useQuery(['goals.changes', goalId], () => dataModel.goalChanges(goalId, CHANGES))
  const back = (
    <a href="#/" className="text-xs text-accent">
      ← {t('goals')}
    </a>
  )
  if (!goal.data) {
    return (
      <>
        {back}
        {goal.error ? <ErrorBox error={goal.error} /> : <Empty>{goal.data === null ? t('goalNotFound') : t('loading')}</Empty>}
      </>
    )
  }
  const def = goal.data
  const builtin = goalExtension(def)?.builtin
  const skip = schedule.data ? lastSkipReason(schedule.data, Boolean(builtin)) : undefined
  const last = state.data?.last_activation
  return (
    <>
      <div className="mb-2 flex flex-wrap items-center gap-2" data-testid="goal-header">
        {back}
        <span className="text-sm font-semibold">{def.title}</span>
        <Mono className="text-mute">{def.goal_id}</Mono>
        <Badge value={t(STATUS_LABEL[def.status])} tone={STATUS_TONE[def.status]} />
        <Badge value={t(LIFECYCLE_LABEL[def.lifecycle ?? 'maintain'])} />
        {builtin && <Badge value={`${t('goalBuiltin')}: ${builtin}`} tone="running" />}
      </div>
      <div className="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-mute" data-testid="goal-summary">
        <span>
          {t('goalLastActivation')}: {last ? <><Time ms={last.at_ms} /> · {t(DECISION_LABEL[last.decision])}</> : t('goalNoActivation')}
        </span>
        {schedule.data && (
          <span>
            {t('goalNextOpportunity')}: <NextOpportunity goal={def} schedule={schedule.data} />
          </span>
        )}
        {skip && (
          <span>
            {t('goalLastSkip')}: <Mono>{skip}</Mono>
          </span>
        )}
        {schedule.data?.last_error && <span className="text-bad">{t('goalLastError')}: {schedule.data.last_error}</span>}
      </div>
      <ErrorBox error={goal.error ?? state.error ?? schedule.error} />
      <Operations goal={def} schedule={schedule.data} onDone={() => void mutate(() => true)} />
      <div className="grid gap-x-3 lg:grid-cols-2">
        <Definition goal={def} />
        <GoalStatePanel goal={def} state={state.data} />
      </div>
      {builtin === 'memory_maintenance' && <Maintenance gate={schedule.data?.gate} maintenance={state.data?.maintenance} />}
      <Activations goal={def} activations={activations.data} error={activations.error} />
      <Panel title={`${t('goalChanges')} (${changes.data?.length ?? 0})`} testId="goal-changes">
        <ErrorBox error={changes.error} />
        {changes.data?.length ? (
          <table className="tbl">
            <tbody>
              {[...changes.data].sort((a, b) => b.seq - a.seq).map((c) => (
                <tr key={c.seq} data-testid="change-row">
                  <td className="w-px text-xs text-mute">{c.seq}</td>
                  <td className="w-px"><Badge value={c.kind} /></td>
                  <td className="text-xs">
                    <Mono>{c.who}</Mono> <span className="text-mute">via {c.via}</span>
                    {c.revision != null && <span className="text-mute"> · rev {c.revision}</span>}
                    {c.epoch != null && <span className="text-mute"> · epoch {c.epoch}</span>}
                    {c.reason && <span className="block">{c.reason}</span>}
                    {c.detail != null && <Mono className="text-mute">{errorText(c.detail)}</Mono>}
                  </td>
                  <td className="w-px"><Time ms={c.at_ms} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{changes.data ? t('goalNoChanges') : t('loading')}</Empty>
        )}
      </Panel>
    </>
  )
}
