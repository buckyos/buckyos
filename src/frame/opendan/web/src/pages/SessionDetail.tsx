import clsx from 'clsx'
import { useState } from 'react'
import { useSWRConfig } from 'swr'
import { t, type TextKey } from '../i18n'
import { goalHref, useQuery } from '../lib'
import { dataModel, type OutboxEntry, type PauseSource, type RegistryEntry, type SessionDetail, type StopScope, type WorklogEntry } from '../model'
import { pauseLabel } from '../runtimes'
import { toneOf } from '../tone'
import { Badge, ConfirmDialog, Dot, Empty, ErrorBox, Fields, Mono, Note, PageHead, Panel, Pre, Sid, Time, type ConfirmRequest } from '../ui'

const PAUSE_HINT: Record<PauseSource, TextKey> = {
  user: 'pausedByUserHint',
  parent: 'pausedWithParentHint',
  interrupted: 'pausedInterruptedHint',
  goal: 'pausedWithGoalHint',
  budget: 'pausedByBudgetHint',
}

const str = (v: unknown): string => (typeof v === 'string' ? v : v == null ? '' : JSON.stringify(v))
const clip = (v: unknown, max = 220): string => {
  const s = str(v).replace(/\s+/g, ' ')
  return s.length > max ? `${s.slice(0, max)}…` : s
}
const inputRef = (v: unknown): string => {
  const r = v as { src?: string; index?: number; kind?: string } | undefined
  return r ? `${r.src}#${r.index}${r.kind ? ` (${r.kind})` : ''}` : ''
}
const actions = (v: unknown): string =>
  Array.isArray(v) ? v.map((a: { tool?: string; args?: unknown }) => `${a.tool}(${clip(a.args, 80)})`).join(' ') : ''

const summarize = (e: WorklogEntry): string => {
  switch (e.t) {
    case 'created':
      return `by ${str(e.by)}: ${clip(e.objective)}`
    case 'turn_started':
    case 'input_batch':
      return [
        `turn ${str(e.turn)} · ${str(e.run_id)}`,
        Array.isArray(e.inputs) && e.inputs.length ? `inputs ${e.inputs.map(inputRef).join(', ')}` : '',
        Array.isArray(e.events) && e.events.length ? `events ${e.events.join(', ')}` : '',
        e.hook ? `hook ${str(e.hook)}` : '',
      ].filter(Boolean).join(' · ')
    case 'user_message':
      return clip(e.content)
    case 'assistant_message':
      return [clip(e.assistant), actions(e.tool_calls)].filter(Boolean).join(' → ')
    case 'step':
      return [`#${str(e.step_index)}${e.behavior ? ` ${str(e.behavior)}` : ''}${e.correction ? ' (correction)' : ''}`, clip(e.assistant), actions(e.actions)]
        .filter(Boolean)
        .join(' · ')
    case 'action_result':
      return `${str(e.call_id)} ${str(e.status)}: ${clip(e.result)}`
    case 'outcome':
      return [str(e.kind), e.next_behavior ? `→ ${str(e.next_behavior)}` : '', clip(e.report)].filter(Boolean).join(' ')
    case 'turn_ended':
      return `turn ${str(e.turn)} ${str(e.status)}`
    case 'compaction':
      return `from seq ${str(e.summary_start_seq)} by ${str(e.made_by)}`
    case 'decide':
      return `${str(e.decision)} by ${str(e.by)}`
    case 'input_rejected':
      return `${inputRef(e.input)}: ${str(e.reason)} ${str(e.detail)}`
    case 'event_dropped':
      return `${inputRef(e.input)}: ${str(e.reason)}`
    case 'control_applied':
      return `${str(e.command)} ${inputRef(e.input)} ${clip(e.detail)}`
    default:
      return clip(e)
  }
}

const WORKLOG_TONE: Record<string, string> = {
  input_rejected: 'bad',
  event_dropped: 'bad',
  control_applied: 'pending',
  decide: 'pending',
  user_message: 'running',
  assistant_message: 'ok',
}

function WorklogRow({ entry }: { entry: WorklogEntry }) {
  const [open, setOpen] = useState(false)
  const at = typeof entry.at_ms === 'number' ? entry.at_ms : null
  const tone = toneOf(WORKLOG_TONE[entry.t] ?? (entry.t === 'turn_ended' ? str(entry.status) : undefined))
  return (
    <>
      <tr className={clsx('cursor-pointer transition-colors hover:bg-hover', open && 'bg-hover')} onClick={() => setOpen(!open)} data-testid="worklog-row" aria-expanded={open}>
        <td className="w-px text-right font-mono text-xs text-mute">{entry.seq}</td>
        <td className="w-px whitespace-nowrap">
          <span className={clsx('inline-flex items-baseline gap-2 font-mono text-xs', tone === 'bad' ? 'text-bad' : 'text-mute')}>
            <Dot tone={tone === 'mute' ? undefined : tone} className="self-center" />
            {entry.t}
          </span>
        </td>
        <td className="text-xs">
          {summarize(entry)}
          {at && <span className="ml-2"><Time ms={at} /></span>}
        </td>
      </tr>
      {open && (
        <tr>
          <td colSpan={3} className="pt-0">
            <Pre value={entry} />
          </td>
        </tr>
      )}
    </>
  )
}

function Outbox({ entries }: { entries: OutboxEntry[] }) {
  return (
    <table className="tbl">
      <thead>
        <tr>
          <th>{t('observeKey')}</th>
          <th>{t('observeTurn')}</th>
          <th>{t('observeStatus')}</th>
          <th>{t('observeAttempts')}</th>
          <th>{t('observeMsgIdDeliveries')}</th>
          <th>{t('observeError')}</th>
          <th>{t('observeUpdated')}</th>
        </tr>
      </thead>
      <tbody>
        {entries.map((o) => (
          <tr key={o.key} title={clip(o.msg, 600)}>
            <td>
              <Mono>{o.key}</Mono>
            </td>
            <td>{o.turn}</td>
            <td>
              <Badge value={o.status} />
            </td>
            <td>{o.attempts}</td>
            <td>
              <Mono>{[o.msg_id, ...(o.deliveries ?? [])].filter(Boolean).join(' → ')}</Mono>
            </td>
            <td className="text-xs text-bad">{o.error}</td>
            <td>
              <Time ms={o.updated_at_ms} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Operations({ detail, onDone }: { detail: SessionDetail; onDone: () => void }) {
  const { entry } = detail
  const sid = entry.session_id
  const [request, setRequest] = useState<ConfirmRequest | null>(null)
  const [text, setText] = useState('')
  const [done, setDone] = useState('')
  const finished = entry.status.run_state === 'finished'
  const decidable = finished && entry.status.acceptance === 'pending'
  const paused = entry.status.paused
  // A finished work is deleted with its descendants once they are all finished.
  const sessions = useQuery(finished && entry.kind === 'work' ? ['sessions.query'] : null, () => dataModel.sessions())
  const descendants = (parent: string): RegistryEntry[] =>
    (sessions.data ?? []).filter((e) => e.origin?.parent_session === parent && e.session_id !== parent).flatMap((e) => [e, ...descendants(e.session_id)])
  const below = descendants(sid)
  const deletable = Boolean(sessions.data) && below.every((e) => e.status.run_state === 'finished')
  // A Goal session is not stopped: its Goal goes dormant.
  const goals = useQuery(entry.kind === 'goal' && !finished ? ['goals.list'] : null, () => dataModel.goals())
  const goal = goals.data?.find((g) => g.session.sid === sid)

  const ask = (r: Omit<ConfirmRequest, 'run'>, run: (input: string) => Promise<string>) =>
    setRequest({
      ...r,
      run: async (input) => {
        setDone(await run(input))
        onDone()
      },
    })
  const decide = (decision: 'accept' | 'discard') =>
    ask(
      {
        title: t(decision === 'accept' ? 'observeAcceptResult' : 'observeDiscardResult', { sid }),
        detail: t('observeDecideHint'),
        inputLabel: t('observeNoteOptional'),
        confirmLabel: t(decision === 'accept' ? 'observeAccept' : 'observeDiscard'),
        danger: decision === 'discard',
      },
      async (note) => t('observeControlPosted', { operation: `decide(${decision})`, index: (await dataModel.decideSession(sid, decision, note || undefined)).index }),
    )
  const stop = (scope: StopScope, label: string, hint: string) =>
    ask(
      { title: `${label}?`, detail: hint, inputLabel: t('opReasonOptional'), confirmLabel: label, danger: true },
      async (reason) => t('observeControlPosted', { operation: `stop(${scope})`, index: (await dataModel.stopSession(sid, scope, reason || undefined)).index }),
    )

  return (
    <Panel title={t('goalOperations')} testId="operations">
      {paused && (
        <Note tone="warn" testId="session-paused" className="mb-3">
          {t(pauseLabel[paused])} · {t(PAUSE_HINT[paused])}
          {(paused === 'goal' || paused === 'budget') && entry.origin?.goal && <> <a className="font-mono text-accent hover:underline" href={goalHref(entry.origin.goal.goal_id)}>{entry.origin.goal.goal_id}</a></>}
        </Note>
      )}
      <div className="flex flex-wrap items-start gap-2">
        {!finished && paused && paused !== 'goal' && (
          <button
            className="btn btn-primary"
            onClick={() =>
              ask(
                { title: `${t('resume')} ${sid}?`, detail: t('resumeHint'), confirmLabel: t('resume') },
                async () => `resume posted at index ${(await dataModel.resumeSession(sid)).index}`,
              )
            }
          >
            {t('resume')}
          </button>
        )}
        {!finished && entry.kind === 'ui' && (
          <button className="btn text-bad" disabled={!entry.status.turn_open} title={entry.status.turn_open ? undefined : t('stopTurnNone')}
            onClick={() => stop('turn', t('stopTurn'), t('stopTurnHint'))}>
            {t('stopTurn')}
          </button>
        )}
        {!finished && entry.kind === 'goal' && (
          <button
            className="btn text-bad"
            disabled={!goal || goal.status !== 'active'}
            onClick={() =>
              goal && ask(
                { title: t('opPauseTitle', { title: goal.title }), detail: t('opPauseHint'), inputLabel: t('opReasonOptional'), confirmLabel: t('opPause'), danger: true },
                async (reason) => `${(await dataModel.setGoalStatus(goal.goal_id, 'paused', reason || undefined)).title}: ${t('gsPaused')}`,
              )
            }
          >
            {goal?.status === 'paused' ? t('gsPaused') : t('opPause')}
          </button>
        )}
        {!finished && entry.kind === 'work' && (
          <button className="btn text-bad" onClick={() => stop('session', t('stop'), t('stopWorkHint'))}>
            {t('stop')}
          </button>
        )}
        {decidable && (
          <>
            <button className="btn btn-primary" onClick={() => decide('accept')}>
              {t('observeAccept')}
            </button>
            <button className="btn text-bad" onClick={() => decide('discard')}>
              {t('skDiscard')}
            </button>
          </>
        )}
        {deletable && (
          <button
            className="btn text-bad"
            data-testid="session-delete"
            onClick={() =>
              setRequest({
                title: t('deleteSessionTitle', { sid }),
                detail: t('deleteSessionHint', { n: below.length }),
                confirmLabel: t('deleteSession'),
                danger: true,
                run: async () => {
                  await dataModel.deleteSession(sid)
                  window.location.hash = '#/'
                },
              })
            }
          >
            {t('deleteSession')}
          </button>
        )}
        {entry.input_queue && (
          <div className="flex min-w-[280px] flex-1 items-start gap-2">
            <textarea
              className="input flex-1 resize-y"
              rows={1}
              placeholder={t('observeMessageToTheSession')}
              aria-label={t('observeMessage')}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            <button
              className="btn"
              disabled={!text.trim()}
              onClick={() =>
                ask(
                  { title: t('observePostToSession', { sid }), detail: <Pre value={text} />, confirmLabel: t('observePostMessage') },
                  async () => {
                    const r = await dataModel.postMessage(sid, text)
                    setText('')
                    return t('observeMessagePosted', { index: r.index, key: r.key })
                  },
                )
              }
            >
              {t('observePost')}
            </button>
          </div>
        )}
      </div>
      {done && (
        <Note tone="ok" testId="operation-result" className="mt-3">
          {done}
        </Note>
      )}
      {request && <ConfirmDialog request={request} onClose={() => setRequest(null)} />}
    </Panel>
  )
}

export default function SessionDetailPage({ sid }: { sid: string }) {
  const [tail, setTail] = useState(40)
  const { mutate } = useSWRConfig()
  const { data, error } = useQuery(['session.read', sid, tail], () => dataModel.session(sid, tail))
  if (!data) {
    return (
      <>
        <PageHead back={{ href: '#/sessions', label: t('sessions') }} title={<span className="font-mono">{sid}</span>} />
        {error ? <ErrorBox error={error} /> : <Empty>{t('loading')}</Empty>}
      </>
    )
  }

  const { entry, state, hosted, config, binding, lease_holder: lease } = data
  const status = entry.status
  const lastError = state?.last_error ?? status.last_error
  const worklog = [...(data.worklog ?? [])].sort((a, b) => b.seq - a.seq)

  return (
    <>
      <PageHead back={{ href: '#/sessions', label: t('sessions') }} title={<span className="font-mono">{entry.session_id}</span>}>
        <span className="text-xs text-mute">{entry.kind}</span>
        <Badge value={status.run_state} />
        {status.outcome && <Badge value={status.outcome} />}
        {status.acceptance !== 'n/a' && <Badge value={`acceptance: ${status.acceptance}`} tone={status.acceptance} />}
        {status.paused && <Badge value={t(pauseLabel[status.paused])} tone="waiting" />}
        {entry.unreachable && <Badge value={t('observeUnreachable')} tone="bad" />}
        {entry.runtime_unavailable && <><Badge value={t('observeRuntimeUnavailable')} tone="waiting" /><span className="text-xs text-mute">{entry.runtime_unavailable}</span></>}
      </PageHead>
      <ErrorBox error={error} />
      {data.note && <Note tone="warn" className="mb-4 rounded-lg border border-line bg-panel px-3 py-2">{data.note}</Note>}
      <Operations detail={data} onDone={() => mutate(() => true)} />

      <div className="grid gap-x-4 lg:grid-cols-2">
        <Panel title={t('goalSession')} testId="session-summary">
          <Fields
            rows={[
              [t('observeObjective'), entry.objective],
              [t('observeStatus'), status.one_line_status],
              [t('observeClass'), entry.class],
              [t('observeDriver'), <Mono>{entry.driver.principal}</Mono>],
              [t('observeCreatedBy'), <Mono>{entry.created_by.principal} {t('observeVia')} {entry.created_by.via}</Mono>],
              [t('observeParent'), entry.origin?.parent_session ? <Sid sid={entry.origin.parent_session} /> : null],
              [t('observeChildren'), data.children.length ? <span className="flex flex-wrap gap-2">{data.children.map((c) => <Sid key={c} sid={c} />)}</span> : null],
              ['route_key', entry.route_key ? <Mono>{entry.route_key}</Mono> : null],
              [t('observeInputQueue'), entry.input_queue ? <Mono>{entry.input_queue}</Mono> : null],
              [t('observeArtifact'), entry.artifact_id ? <Mono>{entry.artifact_id}</Mono> : null],
              [t('observeLocation'), <Mono>{entry.location}</Mono>],
              [t('observeRev'), `registry ${status.rev}${state ? ` · state ${state.rev}` : ''}`],
              [t('observeUpdated'), <Time ms={state?.updated_at_ms ?? status.updated_at_ms} />],
            ]}
          />
        </Panel>
        <Panel title={t('observeTurnLabel')} testId="session-turn">
          <Fields
            rows={[
              [t('observeOpenTurn'), state?.open_turn ? (
                <span>
                  #{state.open_turn.index} <Mono>{state.open_turn.run_id}</Mono> input_seq {state.open_turn.input_seq}
                  {state.open_turn.hook && ` hook ${state.open_turn.hook}`}
                  {state.open_turn.inputs?.length ? <Mono> [{state.open_turn.inputs.join(', ')}]</Mono> : null} <Time ms={state.open_turn.at_ms} />
                </span>
              ) : null],
              [t('observeWaitingFor'), state?.run_state === 'waiting' && (state.wait_children?.length || state.watched_tasks?.length) ? (
                <span>
                  {state.wait_children?.length ? <><Badge value={t('observeSubSessions')} tone="waiting" /> <Mono>{state.wait_children.join(', ')}</Mono> </> : null}
                  {state.watched_tasks?.length ? <><Badge value={t('observeTasks')} tone="waiting" /> <Mono>{state.watched_tasks.join(', ')}</Mono></> : null}
                </span>
              ) : status.needs_input ? <Badge value={t('observeInput')} tone="waiting" /> : null],
              [t('observePaused'), status.paused ? <Badge value={t(pauseLabel[status.paused])} tone="waiting" /> : null],
              [t('observeTurns'), state ? `${state.turns_completed} completed · seq ${state.turn_seq}` : null],
              [t('observeBehavior'), state?.current_behavior ? <Mono>{state.current_behavior}</Mono> : null],
              [t('observeProcessEntry'), state?.process_entry ? <Mono>{state.process_entry}</Mono> : null],
              [t('observeLiveRun'), state?.live_run ? <Mono>{state.live_run.run_id}</Mono> : null],
              [t('observeLastRun'), state?.last_run ? <Mono>{state.last_run}</Mono> : null],
              [t('observeStopRequested'), state?.stop_requested ? <Badge value={t('observeYes')} tone="bad" /> : null],
              [t('observeActivity'), state?.activity.summary || status.activity.summary || null],
              [t('observeTouching'), (state?.activity.touching ?? status.activity.touching)?.length ? (
                <span>
                  {(state?.activity.touching ?? status.activity.touching ?? []).map((t) => (
                    <div key={`${t.kind}:${t.ref}`}><Mono>{t.mode} {t.kind}:{t.ref}</Mono></div>
                  ))}
                </span>
              ) : null],
              [t('observeWatchedTasks'), state?.watched_tasks?.length ? <Mono>{state.watched_tasks.join(', ')}</Mono> : null],
              [t('observePendingDecision'), (state?.pending_decision ?? status.pending_decision) != null ? <Mono>{str(state?.pending_decision ?? status.pending_decision)}</Mono> : null],
            ]}
          />
        </Panel>
      </div>

      {lastError != null && (
        <Panel title={t('goalLastError')} testId="session-last-error">
          <Pre value={lastError} className="text-bad" />
        </Panel>
      )}

      <div className="grid gap-x-4 lg:grid-cols-2">
        <Panel title={t('observeProcessStack')} count={state?.process_stack?.length ?? 0} testId="session-process-stack">
          {state?.process_stack?.length ? (
            <table className="tbl">
              <thead>
                <tr>
                  <th>{t('observeEntry')}</th>
                  <th>{t('observeRole')}</th>
                  <th>{t('observeCalls')}</th>
                  <th>{t('observeRun')}</th>
                </tr>
              </thead>
              <tbody>
                {state.process_stack.map((f, i) => (
                  <tr key={`${f.entry}-${i}`}>
                    <td><Mono>{f.entry}</Mono></td>
                    <td><Badge value={f.role} /></td>
                    <td className="text-xs">
                      {f.call ? `${f.call.mode} → ${f.call.behavior} (${f.call.trigger.kind})${f.call.task ? `: ${f.call.task}` : ''}` : '–'}
                    </td>
                    <td><Mono>{f.run_id}</Mono></td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <Empty>{t('observeEmpty')}</Empty>
          )}
        </Panel>
        <Panel title={t('observePendingEvents')} count={state?.pending_events?.length ?? 0} testId="session-pending-events">
          {state?.pending_events?.length ? (
            <table className="tbl">
              <thead>
                <tr>
                  <th>{t('observeSource')}</th>
                  <th>{t('observeSubscription')}</th>
                  <th>{t('observeLatest')}</th>
                  <th>{t('observeTerminal')}</th>
                  <th>{t('observeSuperseded')}</th>
                </tr>
              </thead>
              <tbody>
                {state.pending_events.map((p, i) => (
                  <tr key={`${p.source.kind}:${p.source.id}-${i}`}>
                    <td><Mono>{p.source.kind}:{p.source.id}</Mono></td>
                    <td><Mono>{p.subscription_id ?? '–'}</Mono></td>
                    <td className="text-xs">{p.latest ? `${p.latest.event}: ${p.latest.summary}` : '–'}</td>
                    <td className="text-xs">{p.terminal ? `${p.terminal.event}: ${p.terminal.summary}` : '–'}</td>
                    <td>{p.superseded}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <Empty>{t('observeNoneLabel')}</Empty>
          )}
        </Panel>
      </div>

      <Panel title={t('observeOutbox')} count={state?.outbox?.length ?? 0} testId="session-outbox">
        {state?.outbox?.length ? <Outbox entries={state.outbox} /> : <Empty>{t('observeNoReplyQueued')}</Empty>}
        {state?.reply != null && <div className="mt-2 text-xs text-mute">{t('observeReplyRoute')} <Mono>{str(state.reply)}</Mono></div>}
      </Panel>

      <Panel
        title={t('observeWorklog')}
        count={worklog.length}
        testId="session-worklog"
        extra={
          <label className="flex items-center gap-2 text-xs text-mute">
            {t('observeNewestFirstTail')}
            <select className="input" aria-label={t('observeWorklogTail')} value={tail} onChange={(e) => setTail(Number(e.target.value))}>
              {[40, 100, 200, 500].map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </label>
        }
      >
        {worklog.length ? (
          <table className="tbl">
            <tbody>{worklog.map((w) => <WorklogRow key={w.seq} entry={w} />)}</tbody>
          </table>
        ) : (
          <Empty>{t('observeNoEntries')}</Empty>
        )}
      </Panel>

      <Panel title={t('observeReport')} testId="session-report">
        {data.report ? <Pre value={data.report} /> : <Empty>{status.report_brief || t('observeNoReport')}</Empty>}
      </Panel>

      <div className="grid gap-x-4 lg:grid-cols-2">
        <Panel title={t('observeRuntimeHosting')} testId="session-runtime">
          <Fields
            rows={[
              [t('observeBinding'), binding ? <span><Mono>{binding.runtime_id}</Mono> <span className="text-xs text-mute">· {binding.connector}</span></span> : null],
              [t('observeWorkdir'), binding ? <Mono>{binding.workdir}</Mono> : null],
              [t('observeTarget'), binding ? <Mono>{str(binding.target)}</Mono> : null],
              [t('observeBound'), binding ? <span><Time ms={binding.bound_at_ms} /> {t('observeBy')} <Mono>{binding.bound_by}</Mono></span> : null],
              [t('observeLeaseHolder'), lease ? (
                <span>
                  <Mono>{lease.holder.principal} · {lease.holder.runner_id} {t('observePid')} {lease.holder.pid}{lease.holder.host ? `@${lease.holder.host}` : ''} {t('observeEpoch')} {lease.epoch}</Mono>{' '}
                  {lease.released_at_ms ? <Badge value={t('observeReleased')} /> : <Badge value={t('observeHeld')} tone="running" />}
                </span>
              ) : null],
              [t('observeHosted'), hosted ? (
                <span>
                  <Badge value={hosted.loaded ? t('observeLoaded') : t('observeUnloaded')} tone={hosted.loaded ? 'ok' : undefined} /> {t('observeDrivesMeta')} {hosted.drives} {t('observeLastMeta')}{' '}
                  <Mono>{hosted.last_result?.kind ?? '–'}</Mono> <Time ms={hosted.last_result_at_ms} /> · {hosted.reason}
                </span>
              ) : <span className="text-xs text-mute">{t('observeNotHostedByThisLoader')}</span>],
              [t('observeRuns'), data.runs?.length ? <Mono>{data.runs.join(', ')}</Mono> : null],
              [t('observeStatistics'), data.statistics ? <Mono>{Object.entries(data.statistics).map(([k, v]) => `${k}=${v}`).join(' ')}</Mono> : null],
            ]}
          />
        </Panel>
        <Panel title={t('observeBehaviorsFrozen')} testId="session-behaviors">
          <Fields
            rows={[
              [t('observeEntryBehavior'), config?.behavior ? <Mono>{config.behavior}</Mono> : null],
              ['catalog_rev', config?.frozen ? <Mono>{config.frozen.catalog_rev}</Mono> : null],
              [t('observeFrozen'), config?.frozen ? <span><Time ms={config.frozen.frozen_at_ms} /> {t('observeBy')} <Mono>{config.frozen.frozen_by}</Mono></span> : null],
              [t('observeBehaviorsLabel'), config?.frozen?.behaviors.length ? <Mono>{config.frozen.behaviors.join(', ')}</Mono> : null],
            ]}
          />
          {config && (
            <details className="mt-3">
              <summary className="cursor-pointer text-xs text-mute transition-colors hover:text-fg">{t('observeSessionConfig')}</summary>
              <Pre value={{ session: config.session, runtime: config.runtime, workspace: config.workspace, workspace_binding: config.workspace_binding, subscriptions: config.subscriptions, channels: config.channels }} />
            </details>
          )}
        </Panel>
      </div>
    </>
  )
}
