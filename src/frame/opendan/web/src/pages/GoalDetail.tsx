import { useMemo, useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { useSWRConfig } from 'swr'
import {
  CHECK_MIN_EVERY_SECS,
  GUARANTEE_MIN_SECS,
  DECISION_LABEL,
  DECISION_TONE,
  GOAL_DEFAULTS,
  LIFECYCLE_LABEL,
  OUTCOME_TONE,
  STATUS_LABEL,
  STATUS_TONE,
  errorText,
  lastSkipReason,
} from '../goals'
import { lang, t, type TextKey } from '../i18n'
import { fmtDuration, fmtTime, fmtTokens, goalExtension, toText, useQuery } from '../lib'
import {
  dataModel,
  OpenDanError,
  type CheckOutcome,
  type GoalActivation,
  type GoalBudget,
  type GoalCheckChange,
  type GoalDefinition,
  type GoalSchedule,
  type GoalScheduleSpec,
  type GoalState,
  type GoalUsage,
  type HoldReason,
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
  const off = Boolean(schedule?.switched_off)
  const live = (goal.status === 'active' || goal.status === 'paused') && !off
  const ended = goal.status === 'archived' || goal.status === 'achieved'
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
            <span className={off ? 'text-bad' : 'text-ok'} data-testid="goal-config-switch">
              {t(off ? 'switchOff' : 'switchOn')}
            </span>
          </span>
        </div>
      )}
      {off && <p className="mb-2 text-xs text-mute" data-testid="goal-off-hint">{t('goalSwitchedOffHint')}</p>}
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
        {live && goal.status === 'active' && (
          <button className="btn" onClick={() => setStatus('paused', 'opPauseTitle', 'opPauseHint', 'opPause')}>
            {t('opPause')}
          </button>
        )}
        {live && goal.status === 'paused' && (
          <button className="btn border-ok text-ok" onClick={() => setStatus('active', 'opResumeTitle', 'opResumeHint', 'opResume')}>
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
        {ended && (
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

const scheduleText = (schedule?: GoalScheduleSpec): string =>
  [
    schedule?.cron ? t('gateCron', { expr: schedule.cron, zone: schedule.timezone || 'UTC' }) : '',
    schedule?.every_secs ? t('goalEvery', { n: fmtDuration(schedule.every_secs) }) : '',
    schedule?.check ? t('goalCheckScriptV', { v: schedule.check.version }) : '',
    schedule?.min_interval_secs != null ? t('goalMinInterval', { n: fmtDuration(schedule.min_interval_secs) }) : '',
    schedule?.guarantee_secs != null ? (schedule.guarantee_secs ? t('goalGuarantee', { n: fmtDuration(schedule.guarantee_secs) }) : t('goalGuaranteeOff')) : '',
    schedule?.start_on_create ? t('goalStartOnCreate') : '',
  ]
    .filter(Boolean)
    .join(' · ')

const DIFF_FIELDS = [
  'title',
  'objective',
  'scope',
  'lifecycle',
  'achieved_when',
  'healthy_state',
  'distance_evaluation',
  'constraints',
  'initial_skills',
  'budget',
  'schedule',
  'behavior',
] as const

/** A value as the diff compares it: object keys in order, nothing for an empty value. */
const canon = (value: unknown): unknown => {
  if (value == null || value === '') return undefined
  if (Array.isArray(value)) return value.length ? value.map(canon) : undefined
  if (typeof value === 'object') {
    const entries = Object.keys(value)
      .sort()
      .map((k) => [k, canon((value as Record<string, unknown>)[k])] as const)
      .filter(([, v]) => v !== undefined)
    return entries.length ? Object.fromEntries(entries) : undefined
  }
  return value
}
const sameValue = (a: unknown, b: unknown) => JSON.stringify(canon(a)) === JSON.stringify(canon(b))

/** Field by field: what revision `revision` changed from the one before. */
function RevisionDiff({ goalId, revision }: { goalId: string; revision: number }) {
  const before = useQuery(['goals.revision', goalId, revision - 1], () => dataModel.goalRevision(goalId, revision - 1), false)
  const after = useQuery(['goals.revision', goalId, revision], () => dataModel.goalRevision(goalId, revision), false)
  const error = before.error ?? after.error
  const rows = before.data && after.data ? DIFF_FIELDS.filter((f) => !sameValue(before.data![f], after.data![f])) : []
  const text = (value: unknown) => {
    const v = canon(value)
    return v === undefined ? '–' : toText(v)
  }
  return (
    <div className="mt-2 border-t border-line pt-2" data-testid="goal-revision-diff">
      <h3 className="mb-1 text-xs font-semibold text-mute">{t('goalDiffTitle', { from: revision - 1, to: revision })}</h3>
      <ErrorBox error={error} />
      {!before.data || !after.data ? (
        !error && <Empty>{t('loading')}</Empty>
      ) : rows.length ? (
        rows.map((field) => (
          <div key={field} className="mb-2 min-w-0 text-xs" data-testid={`goal-diff-${field}`}>
            <div className="font-mono font-semibold">{field}</div>
            <pre className="whitespace-pre-wrap break-words rounded bg-bg p-1 font-mono text-bad">− {text(before.data![field])}</pre>
            <pre className="mt-0.5 whitespace-pre-wrap break-words rounded bg-bg p-1 font-mono text-ok">+ {text(after.data![field])}</pre>
          </div>
        ))
      ) : (
        <Empty>{t('goalDiffNone')}</Empty>
      )}
    </div>
  )
}

function Definition({ goal }: { goal: GoalDefinition }) {
  const [rev, setRev] = useState<number | null>(null)
  const [diff, setDiff] = useState(false)
  const snapshot = useQuery(rev != null ? ['goals.revision', goal.goal_id, rev] : null, () => dataModel.goalRevision(goal.goal_id, rev!), false)
  const shown = (rev != null && snapshot.data) || goal
  const shownRevision = rev ?? goal.revision
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
                  budget.activation?.max_wall_secs != null && `≤ ${fmtDuration(budget.activation.max_wall_secs)} wall`,
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
            scheduleText(schedule) ? (
              <span className="text-xs">{scheduleText(schedule)}</span>
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
      {shownRevision > 1 && (
        <button className="btn mt-2" onClick={() => setDiff(!diff)} data-testid="goal-diff-toggle">
          {diff ? t('goalDiffHide') : t('goalDiffShow', { n: shownRevision - 1 })}
        </button>
      )}
      {diff && shownRevision > 1 && <RevisionDiff goalId={goal.goal_id} revision={shownRevision} />}
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

/** The goal's statistics window (fixed windows, as the host counts them) and its usage there; an older window counts as zero. */
const currentPeriod = (goal: GoalDefinition, state?: GoalState | null) => {
  const windowSecs = goal.budget?.period?.window_secs || GOAL_DEFAULTS.window_secs
  const start = Math.floor(Date.now() / (windowSecs * 1000)) * windowSecs * 1000
  const usage: GoalUsage | undefined = state && state.usage.period_start_ms >= start ? state.usage : undefined
  return { windowSecs, start, usage }
}

const CHECK_TONE: Record<CheckOutcome, string> = { true: 'ok', false: 'finished', error: 'bad' }
const HOLD_LABEL: Record<HoldReason, TextKey> = { min_interval: 'gateHoldInterval', period_budget: 'gateHoldPeriod', agent_budget: 'gateHoldAgent' }
const withDefault = (value: number | undefined, fallback: number) =>
  value != null ? fmtDuration(value) : `${fmtDuration(fallback)} (${t('gateDefault')})`

function CheckScript({ goalId, version }: { goalId: string; version: number }) {
  const text = useQuery(['goals.check_script', goalId, version], () => dataModel.goalCheckScript(goalId, version), false)
  return (
    <div className="mt-2" data-testid="gate-script-text">
      <ErrorBox error={text.error} />
      {text.data != null ? <Pre value={text.data} /> : !text.error && <Empty>{text.data === null ? t('gateNoScriptText') : t('loading')}</Empty>}
    </div>
  )
}

type TimetableMode = 'default' | 'every' | 'cron'
type FormText = 'interval' | 'guarantee' | 'periodTokens' | 'window' | 'works' | 'actTokens' | 'runs' | 'wall' | 'every' | 'cron' | 'timezone' | 'reason'
const fieldText = (n?: number) => (n != null ? String(n) : '')
const localZone = () => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone ?? ''
  } catch {
    return ''
  }
}

/** The owner edits the budget and the gates: a revision for those, a check change for the timetable. */
function GatesForm({ goal, onClose, onSaved, onConflict }: {
  goal: GoalDefinition
  onClose: () => void
  onSaved: (what: string) => void
  onConflict: (error: unknown) => void
}) {
  const s = goal.schedule ?? {}
  const b = goal.budget ?? {}
  const [revision] = useState(goal.revision)
  const [text, setText] = useState<Record<FormText, string>>({
    interval: fieldText(s.min_interval_secs),
    guarantee: s.guarantee_secs ? String(s.guarantee_secs) : '',
    periodTokens: fieldText(b.period?.max_total_tokens),
    window: fieldText(b.period?.window_secs),
    works: fieldText(b.period?.max_works),
    actTokens: fieldText(b.activation?.max_total_tokens),
    runs: fieldText(b.activation?.max_runs),
    wall: fieldText(b.activation?.max_wall_secs),
    every: fieldText(s.every_secs),
    cron: s.cron ?? '',
    timezone: s.timezone ?? '',
    reason: '',
  })
  const [guaranteeOff, setGuaranteeOff] = useState(s.guarantee_secs === 0)
  const [mode, setMode] = useState<TimetableMode>(s.cron ? 'cron' : s.every_secs ? 'every' : 'default')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const update = (key: FormText, value: string) => setText((old) => ({ ...old, [key]: value }))
  const parse = (key: FormText, label: TextKey, min: number): number | undefined => {
    const raw = text[key].trim()
    if (!raw) return undefined
    const n = Number(raw)
    if (!Number.isInteger(n) || n < min) throw new Error(t('formNumber', { field: t(label), min }))
    return n
  }
  const field = (key: FormText, label: TextKey, disabled = false) => (
    <label className="block min-w-0 text-xs text-mute">
      {t(label)}
      <input className="input mt-1 block w-full" inputMode="numeric" placeholder={t('gateDefault')} disabled={disabled} value={text[key]} onChange={(e) => update(key, e.target.value)} />
    </label>
  )

  const submit = async () => {
    let budget: GoalBudget
    let schedule: GoalScheduleSpec
    let change: GoalCheckChange | null = null
    try {
      budget = {
        activation: { ...b.activation, max_total_tokens: parse('actTokens', 'formActTokens', 1), max_runs: parse('runs', 'formRuns', 1), max_wall_secs: parse('wall', 'formWall', 1) },
        period: { ...b.period, max_total_tokens: parse('periodTokens', 'formPeriodTokens', 1), window_secs: parse('window', 'formWindow', 1), max_works: parse('works', 'formWorks', 0) },
      }
      schedule = { ...s, min_interval_secs: parse('interval', 'formInterval', 0), guarantee_secs: guaranteeOff ? 0 : parse('guarantee', 'formGuarantee', GUARANTEE_MIN_SECS) }
      if (mode === 'default') {
        schedule = { ...schedule, every_secs: undefined, cron: undefined, timezone: undefined }
      } else if (mode === 'every') {
        const every = parse('every', 'formEvery', CHECK_MIN_EVERY_SECS)
        if (every == null) throw new Error(t('formRequired', { field: t('formEvery') }))
        if (every !== s.every_secs) change = { every_secs: every }
      } else {
        const cron = text.cron.trim()
        const timezone = text.timezone.trim()
        if (!cron) throw new Error(t('formRequired', { field: t('formCron') }))
        if (cron !== s.cron || timezone !== (s.timezone ?? '')) change = { cron, timezone }
      }
    } catch (e) {
      setError(e)
      return
    }
    const reason = text.reason.trim()
    const revise = !sameValue(budget, goal.budget) || !sameValue(schedule, s)
    if (!revise && !change) {
      onSaved(t('formNoChange'))
      onClose()
      return
    }
    setBusy(true)
    setError(null)
    try {
      let saved = revision
      if (revise) saved = (await dataModel.reviseGoal(goal.goal_id, { expected_revision: revision, budget, schedule, ...(reason ? { reason } : {}) })).revision
      if (change) await dataModel.setGoalCheck(goal.goal_id, { ...change, ...(reason ? { reason } : {}) })
      onSaved(t('formSaved', { revision: saved }))
      onClose()
    } catch (e) {
      if (e instanceof OpenDanError && e.kind === 'goal_conflict') {
        onConflict(e)
        return
      }
      setError(e)
      setBusy(false)
    }
  }

  const legend = 'mb-1 text-xs font-semibold text-mute'
  const group = 'mb-3 grid min-w-0 grid-cols-1 gap-2 sm:grid-cols-2'
  return (
    <form
      className="mt-2 min-w-0 rounded border border-line p-2"
      data-testid="gate-form"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <p className="mb-2 text-xs text-mute">{t('formHint')}</p>
      <fieldset className={group}>
        <legend className={legend}>{t('formGates')}</legend>
        {field('interval', 'formInterval')}
        <div className="min-w-0">
          {field('guarantee', 'formGuarantee', guaranteeOff)}
          <label className="mt-1 flex items-center gap-2 text-xs">
            <input type="checkbox" checked={guaranteeOff} onChange={(e) => setGuaranteeOff(e.target.checked)} />
            {t('formGuaranteeOff')}
          </label>
        </div>
      </fieldset>
      <fieldset className={group}>
        <legend className={legend}>{t('gatePeriodBudget')}</legend>
        {field('periodTokens', 'formPeriodTokens')}
        {field('window', 'formWindow')}
        {field('works', 'formWorks')}
      </fieldset>
      <fieldset className={group}>
        <legend className={legend}>{t('formActivation')}</legend>
        {field('actTokens', 'formActTokens')}
        {field('runs', 'formRuns')}
        {field('wall', 'formWall')}
      </fieldset>
      <fieldset className={group}>
        <legend className={legend}>{t('gateTimetable')}</legend>
        <label className="block min-w-0 text-xs text-mute">
          {t('formTimetable')}
          <select
            className="input mt-1 block w-full"
            value={mode}
            onChange={(e) => {
              const next = e.target.value as TimetableMode
              if (next === 'cron' && !text.timezone) update('timezone', localZone())
              setMode(next)
            }}
          >
            <option value="default">{t('formTimetableDefault')}</option>
            <option value="every">{t('formTimetableEvery')}</option>
            <option value="cron">cron</option>
          </select>
        </label>
        {mode === 'every' && field('every', 'formEvery')}
        {mode === 'cron' && (
          <>
            <label className="block min-w-0 text-xs text-mute">
              {t('formCron')}
              <input className="input mt-1 block w-full font-mono" placeholder="0 9 * * *" value={text.cron} onChange={(e) => update('cron', e.target.value)} />
            </label>
            <label className="block min-w-0 text-xs text-mute">
              {t('formTimezone')}
              <input className="input mt-1 block w-full" placeholder="Asia/Shanghai" value={text.timezone} onChange={(e) => update('timezone', e.target.value)} />
            </label>
          </>
        )}
      </fieldset>
      <label className="mb-2 block text-xs text-mute">
        {t('opReasonOptional')}
        <input className="input mt-1 block w-full" value={text.reason} onChange={(e) => update('reason', e.target.value)} />
      </label>
      <ErrorBox error={error} />
      <div className="flex justify-end gap-2">
        <button type="button" className="btn" onClick={onClose} disabled={busy}>
          {t('cancel')}
        </button>
        <button type="submit" className="btn border-accent text-accent" disabled={busy}>
          {t('save')}
        </button>
      </div>
    </form>
  )
}

/** The four gates of a run opportunity (hard guarantee, hard interval, period budget, check) and what they did this period. */
function Gates({ goal, state, schedule, onDone }: { goal: GoalDefinition; state?: GoalState | null; schedule?: GoalSchedule | null; onDone: () => void }) {
  const [editing, setEditing] = useState(false)
  const [script, setScript] = useState(false)
  const [spec, setSpec] = useState<DialogSpec | null>(null)
  const [done, setDone] = useState('')
  const [conflict, setConflict] = useState<unknown>(null)
  const s = goal.schedule ?? {}
  const check = s.check
  const limits = goal.budget?.period
  const { windowSecs, start, usage } = currentPeriod(goal, state)
  const goalTokens = usage?.tokens ?? 0
  const workTokens = usage?.work_tokens ?? 0
  const used = goalTokens + workTokens
  const limit = limits?.max_total_tokens ?? GOAL_DEFAULTS.period_max_total_tokens
  const stats = schedule && schedule.stats.period_start_ms >= start ? schedule.stats : undefined
  const holds = state && state.holds.period_start_ms >= start ? state.holds : undefined
  const last = schedule?.last_check
  const holding = state?.holding
  const pkg = check?.package
  const restorable = Boolean(pkg) && check?.source !== `package:${pkg}`
  const editable = (goal.status === 'active' || goal.status === 'paused') && !schedule?.switched_off
  const timetable = s.cron
    ? t('gateCron', { expr: s.cron, zone: s.timezone || 'UTC' })
    : s.every_secs
      ? t('goalEvery', { n: fmtDuration(s.every_secs) })
      : check
        ? `${t('goalEvery', { n: fmtDuration(GOAL_DEFAULTS.check_every_secs) })} (${t('gateDefault')})`
        : t('gateNoTimetable')
  const finished = (what: string) => {
    setDone(what)
    setConflict(null)
    onDone()
  }
  return (
    <Panel title={t('gatesTitle')} testId="goal-gates">
      <p className="mb-2 text-xs text-mute">{t('gatesHint')}</p>
      <Fields
        rows={[
          [
            t('gateGuarantee'),
            <span className="text-xs" data-testid="gate-guarantee">
              {s.guarantee_secs === 0 ? t('gateOff') : withDefault(s.guarantee_secs, GOAL_DEFAULTS.guarantee_secs)}
            </span>,
          ],
          [t('gateInterval'), <span className="text-xs" data-testid="gate-interval">{withDefault(s.min_interval_secs, GOAL_DEFAULTS.min_interval_secs)}</span>],
          [
            t('gatePeriodBudget'),
            <span className="text-xs" data-testid="gate-budget">
              {t('gateUsed', {
                used: fmtTokens(used),
                limit: limits?.max_total_tokens != null ? fmtTokens(limit) : `${fmtTokens(limit)} (${t('gateDefault')})`,
                remaining: fmtTokens(Math.max(0, limit - used)),
              })}
              <span className="block text-mute">{t('gateUsedSplit', { goal: fmtTokens(goalTokens), works: fmtTokens(workTokens) })}</span>
              {limits?.max_works != null && <span className="block text-mute">{t('gateWorks', { n: usage?.works ?? 0, max: limits.max_works })}</span>}
              <span className="block text-mute">{t('gateWindow', { window: fmtDuration(windowSecs), time: fmtTime(start + windowSecs * 1000) })}</span>
            </span>,
          ],
          [t('gateTimetable'), <span className="text-xs" data-testid="gate-timetable">{timetable}</span>],
          [
            t('gateScript'),
            check ? (
              <span className="text-xs" data-testid="gate-script">
                <Mono>v{check.version} · {check.source}</Mono> <Time ms={check.updated_at_ms} />
                {pkg && <span className="block text-mute">{t('gateScriptPackage', { path: pkg })}</span>}
              </span>
            ) : (
              <span className="text-xs text-mute" data-testid="gate-script">{t('gateNoScript')}</span>
            ),
          ],
          [
            t('gateLastCheck'),
            last ? (
              <span className="text-xs" data-testid="gate-last-check">
                <Badge value={last.outcome} tone={CHECK_TONE[last.outcome]} /> <Time ms={last.at_ms} />
                <span className="text-mute"> · v{last.script_version} · {last.elapsed_ms} ms</span>
                {last.reason && <span className="block">{last.reason}</span>}
              </span>
            ) : null,
          ],
          [
            t('gateCheckErrors'),
            schedule && (schedule.check_errors || schedule.last_check_error) ? (
              <span className={clsx('text-xs', schedule.check_errors ? 'text-bad' : 'text-mute')} data-testid="gate-check-errors">
                {t('gateErrorsLine', { n: schedule.check_errors })}
                {schedule.last_check_error && <span className="block break-words">{t('goalLastError')}: {schedule.last_check_error}</span>}
              </span>
            ) : null,
          ],
          [
            t('gateCounts'),
            <span className="text-xs" data-testid="gate-counts">
              <span className="block">{t('gateCountChecks', { true: stats?.checks_true ?? 0, false: stats?.checks_false ?? 0, error: stats?.checks_error ?? 0 })}</span>
              <span className="block">{t('gateCountSkipped', { interval: stats?.skipped_min_interval ?? 0, budget: stats?.skipped_budget ?? 0 })}</span>
              <span className="block">{t('gateCountHeld', { interval: holds?.min_interval ?? 0, budget: holds?.budget ?? 0 })}</span>
              <span className="block">{t('gateCountGuarantees', { n: stats?.guarantees ?? 0 })}</span>
            </span>,
          ],
          [t('gateNoAction'), <span className="text-xs" data-testid="gate-no-action">{state?.consecutive_no_action ?? 0}</span>],
          [
            t('gateHolding'),
            holding ? (
              <span className="text-xs text-warn" data-testid="gate-holding">
                {t(HOLD_LABEL[holding.reason])} · {t('gateSince')} <Time ms={holding.since_ms} />
                {holding.until_ms ? <> · {t('gateUntil')} <Time ms={holding.until_ms} /></> : null}
              </span>
            ) : (
              <span className="text-xs text-mute" data-testid="gate-holding">{t('gateNotHolding')}</span>
            ),
          ],
        ]}
      />
      <div className="mt-2 flex flex-wrap gap-2">
        {check && (
          <button className="btn" onClick={() => setScript(!script)} data-testid="gate-script-toggle">
            {t(script ? 'gateScriptHide' : 'gateScriptShow')}
          </button>
        )}
        {restorable && editable && (
          <button
            className="btn"
            data-testid="gate-restore"
            onClick={() =>
              setSpec({
                title: t('gateRestoreTitle', { title: goal.title }),
                hint: t('gateRestoreHint', { path: pkg ?? '' }),
                confirmLabel: t('gateRestore'),
                run: async (reason) =>
                  t('gateRestored', { v: (await dataModel.setGoalCheck(goal.goal_id, { restore_package: true, ...(reason ? { reason } : {}) })).schedule?.check?.version ?? '?' }),
              })
            }
          >
            {t('gateRestore')}
          </button>
        )}
        {editable && !editing && (
          <button
            className="btn border-accent text-accent"
            data-testid="gate-edit"
            onClick={() => {
              setEditing(true)
              setDone('')
              setConflict(null)
            }}
          >
            {t('gateEdit')}
          </button>
        )}
      </div>
      {script && check && <CheckScript goalId={goal.goal_id} version={check.version} />}
      {conflict != null && (
        <div className="mt-2" data-testid="gate-conflict">
          <ErrorBox error={conflict} />
          <p className="text-xs text-warn">{t('formConflict')}</p>
        </div>
      )}
      {done && (
        <div className="mt-2 text-xs text-ok" data-testid="gate-result">
          {t('opDone', { what: done })}
        </div>
      )}
      {editing && (
        <GatesForm
          goal={goal}
          onClose={() => setEditing(false)}
          onSaved={finished}
          onConflict={(e) => {
            setEditing(false)
            setConflict(e)
            onDone()
          }}
        />
      )}
      {spec && <GoalDialog spec={spec} onClose={() => setSpec(null)} onDone={finished} />}
    </Panel>
  )
}

/** Tokens per day of the goal's own activations (last 7 days) and the current period's total. */
function Cost({ goal, state, activations }: { goal: GoalDefinition; state?: GoalState | null; activations?: GoalActivation[] }) {
  const days = useMemo(() => {
    const today = new Date()
    today.setHours(0, 0, 0, 0)
    return Array.from({ length: 7 }, (_, i) => {
      const from = new Date(today)
      from.setDate(today.getDate() - 6 + i)
      const to = new Date(from)
      to.setDate(from.getDate() + 1)
      const tokens = (activations ?? [])
        .filter((a) => a.ended_at_ms >= from.getTime() && a.ended_at_ms < to.getTime())
        .reduce((sum, a) => sum + (a.cost?.total_tokens ?? 0), 0)
      return { from, tokens }
    })
  }, [activations])
  const { usage } = currentPeriod(goal, state)
  const max = Math.max(1, ...days.map((d) => d.tokens))
  const week = days.reduce((sum, d) => sum + d.tokens, 0)
  return (
    <Panel title={t('costTitle')} testId="goal-cost">
      <p className="mb-2 text-xs" data-testid="cost-period">
        {t('costPeriod', {
          total: fmtTokens((usage?.tokens ?? 0) + (usage?.work_tokens ?? 0)),
          goal: fmtTokens(usage?.tokens ?? 0),
          works: fmtTokens(usage?.work_tokens ?? 0),
        })}
      </p>
      <p className="mb-1 text-xs text-mute">{t('costByDay')}</p>
      <ul className="text-xs">
        {days.map(({ from, tokens }) => (
          <li key={from.getTime()} className="flex items-center gap-2 py-0.5" data-testid="cost-day">
            <span className="w-24 shrink-0 tabular-nums text-mute">{from.toLocaleDateString(lang === 'zh' ? 'zh-CN' : 'en', { month: '2-digit', day: '2-digit', weekday: 'short' })}</span>
            <span className="h-2 min-w-0 flex-1 rounded bg-soft">
              <span className="block h-2 rounded bg-accent" style={{ width: `${(tokens / max) * 100}%` }} />
            </span>
            <span className="w-12 shrink-0 text-right tabular-nums">{fmtTokens(tokens)}</span>
          </li>
        ))}
      </ul>
      <p className="mt-1 text-xs text-mute" data-testid="cost-week">
        {t('costWeek', { n: fmtTokens(week) })}
      </p>
    </Panel>
  )
}

function Maintenance({ maintenance }: { maintenance?: MaintenanceState }) {
  return (
    <Panel title={t('goalMaintenance')} testId="goal-maintenance">
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
  const off = Boolean(schedule.data?.switched_off)
  const skip = schedule.data && !off ? lastSkipReason(schedule.data) : undefined
  const last = state.data?.last_activation
  const refresh = () => void mutate(() => true)
  return (
    <>
      <div className="mb-2 flex flex-wrap items-center gap-2" data-testid="goal-header">
        {back}
        <span className="text-sm font-semibold">{def.title}</span>
        <Mono className="text-mute">{def.goal_id}</Mono>
        {off ? (
          <span data-testid="goal-off">
            <Badge value={t('goalSwitchedOff')} tone="finished" />
          </span>
        ) : (
          <Badge value={t(STATUS_LABEL[def.status])} tone={STATUS_TONE[def.status]} />
        )}
        <Badge value={t(LIFECYCLE_LABEL[def.lifecycle ?? 'maintain'])} />
        {builtin && <Badge value={`${t('goalBuiltin')}: ${builtin}`} tone="running" />}
      </div>
      <div className="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-mute" data-testid="goal-summary">
        <span>
          {t('goalLastActivation')}: {last ? <><Time ms={last.at_ms} /> · {t(DECISION_LABEL[last.decision])}</> : t('goalNoActivation')}
        </span>
        {schedule.data && (
          <span>
            {t('goalNextOpportunity')}: <NextOpportunity schedule={schedule.data} />
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
      <Operations goal={def} schedule={schedule.data} onDone={refresh} />
      <div className="grid grid-cols-1 gap-x-3 lg:grid-cols-2">
        <Definition goal={def} />
        <GoalStatePanel goal={def} state={state.data} />
      </div>
      <div className="grid grid-cols-1 gap-x-3 lg:grid-cols-2">
        <Gates goal={def} state={state.data} schedule={schedule.data} onDone={refresh} />
        <Cost goal={def} state={state.data} activations={activations.data} />
      </div>
      {builtin === 'memory_maintenance' && <Maintenance maintenance={state.data?.maintenance} />}
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
