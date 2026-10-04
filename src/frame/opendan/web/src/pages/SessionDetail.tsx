import { useState } from 'react'
import { useSWRConfig } from 'swr'
import { useQuery } from '../lib'
import { dataModel, type OutboxEntry, type SessionDetail, type WorklogEntry } from '../model'
import { Badge, ConfirmDialog, Empty, ErrorBox, Fields, Mono, Panel, Pre, Sid, Time, type ConfirmRequest } from '../ui'

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
  return (
    <>
      <tr className="cursor-pointer hover:bg-bg" onClick={() => setOpen(!open)} data-testid="worklog-row">
        <td className="w-px text-xs text-mute">{entry.seq}</td>
        <td className="w-px">
          <Badge value={entry.t} tone={WORKLOG_TONE[entry.t] ?? (entry.t === 'turn_ended' ? str(entry.status) : undefined)} />
        </td>
        <td className="text-xs">
          {summarize(entry)}
          {at && <span className="ml-2"><Time ms={at} /></span>}
        </td>
      </tr>
      {open && (
        <tr>
          <td colSpan={3}>
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
          <th>key</th>
          <th>turn</th>
          <th>status</th>
          <th>attempts</th>
          <th>msg_id / deliveries</th>
          <th>error</th>
          <th>updated</th>
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
  // Every operation is a record on the session's input bus: no queue, no operation.
  const queued = Boolean(entry.input_queue)
  const decidable = queued && finished && entry.status.acceptance === 'pending'

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
        title: `${decision === 'accept' ? 'Accept' : 'Discard'} the result of ${sid}?`,
        detail: 'Posts control(decide) to the session input bus.',
        inputLabel: 'Note (optional)',
        confirmLabel: decision === 'accept' ? 'Accept' : 'Discard',
        danger: decision === 'discard',
      },
      async (note) => `decide(${decision}) posted at index ${(await dataModel.decideSession(sid, decision, note || undefined)).index}`,
    )

  return (
    <Panel title="Operations" testId="operations">
      <div className="flex flex-wrap items-start gap-2">
        {queued && !finished && (
          <button
            className="btn border-bad text-bad"
            onClick={() =>
              ask(
                {
                  title: `Stop session ${sid}?`,
                  detail: 'Posts control(stop) to the session input bus. The session ends as stopped.',
                  inputLabel: 'Reason (optional)',
                  confirmLabel: 'Stop session',
                  danger: true,
                },
                async (reason) => `stop posted at index ${(await dataModel.stopSession(sid, reason || undefined)).index}`,
              )
            }
          >
            Stop
          </button>
        )}
        {decidable && (
          <>
            <button className="btn border-ok text-ok" onClick={() => decide('accept')}>
              Accept
            </button>
            <button className="btn border-bad text-bad" onClick={() => decide('discard')}>
              Discard
            </button>
          </>
        )}
        {entry.input_queue && (
          <div className="flex min-w-[280px] flex-1 items-start gap-2">
            <textarea
              className="input min-h-[28px] flex-1"
              rows={1}
              placeholder="Message to the session"
              aria-label="Message"
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            <button
              className="btn"
              disabled={!text.trim()}
              onClick={() =>
                ask(
                  { title: `Post a message to ${sid}?`, detail: <Pre value={text} />, confirmLabel: 'Post message' },
                  async () => {
                    const r = await dataModel.postMessage(sid, text)
                    setText('')
                    return `message posted at index ${r.index} (${r.key})`
                  },
                )
              }
            >
              Post
            </button>
          </div>
        )}
        {!queued && <Empty>This session has no input queue: no operation applies.</Empty>}
      </div>
      {done && (
        <div className="mt-2 text-xs text-ok" data-testid="operation-result">
          {done}
        </div>
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
        <a href="#/sessions" className="text-xs text-accent">← Sessions</a>
        {error ? <ErrorBox error={error} /> : <Empty>Loading…</Empty>}
      </>
    )
  }

  const { entry, state, hosted, config, binding, lease_holder: lease } = data
  const status = entry.status
  const lastError = state?.last_error ?? status.last_error
  const worklog = [...(data.worklog ?? [])].sort((a, b) => b.seq - a.seq)

  return (
    <>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <a href="#/sessions" className="text-xs text-accent">← Sessions</a>
        <Mono className="text-sm font-semibold">{entry.session_id}</Mono>
        <Badge value={entry.kind} />
        <Badge value={status.run_state} />
        {status.outcome && <Badge value={status.outcome} />}
        {status.acceptance !== 'n/a' && <Badge value={`acceptance: ${status.acceptance}`} tone={status.acceptance} />}
        {entry.unreachable && <Badge value="unreachable" tone="bad" />}
      </div>
      <ErrorBox error={error} />
      {data.note && <div className="mb-3 rounded border border-warn px-3 py-2 text-xs text-warn">{data.note}</div>}
      <Operations detail={data} onDone={() => mutate(() => true)} />

      <div className="grid gap-x-3 lg:grid-cols-2">
        <Panel title="Session" testId="session-summary">
          <Fields
            rows={[
              ['objective', entry.objective],
              ['status', status.one_line_status],
              ['class', entry.class],
              ['driver', <Mono>{entry.driver.principal}</Mono>],
              ['created_by', <Mono>{entry.created_by.principal} via {entry.created_by.via}</Mono>],
              ['parent', entry.origin?.parent_session ? <Sid sid={entry.origin.parent_session} /> : null],
              ['children', data.children.length ? <span className="flex flex-wrap gap-2">{data.children.map((c) => <Sid key={c} sid={c} />)}</span> : null],
              ['route_key', entry.route_key ? <Mono>{entry.route_key}</Mono> : null],
              ['input_queue', entry.input_queue ? <Mono>{entry.input_queue}</Mono> : null],
              ['artifact', entry.artifact_id ? <Mono>{entry.artifact_id}</Mono> : null],
              ['location', <Mono>{entry.location}</Mono>],
              ['rev', `registry ${status.rev}${state ? ` · state ${state.rev}` : ''}`],
              ['updated', <Time ms={state?.updated_at_ms ?? status.updated_at_ms} />],
            ]}
          />
        </Panel>
        <Panel title="Turn" testId="session-turn">
          <Fields
            rows={[
              ['open turn', state?.open_turn ? (
                <span>
                  #{state.open_turn.index} <Mono>{state.open_turn.run_id}</Mono> input_seq {state.open_turn.input_seq}
                  {state.open_turn.hook && ` hook ${state.open_turn.hook}`}
                  {state.open_turn.inputs?.length ? <Mono> [{state.open_turn.inputs.join(', ')}]</Mono> : null} <Time ms={state.open_turn.at_ms} />
                </span>
              ) : null],
              ['waiting for', state?.waiting_for ? (
                <span>
                  <Badge value={state.waiting_for.kind} tone="waiting" /> <Mono>{(state.waiting_for.refs ?? []).join(', ')}</Mono>
                  {state.waiting_for.deadline_ms ? <span className="ml-1 text-xs text-mute">deadline <Time ms={state.waiting_for.deadline_ms} /></span> : null}
                </span>
              ) : status.waiting_for ? <Badge value={status.waiting_for} tone="waiting" /> : null],
              ['turns', state ? `${state.turns_completed} completed · seq ${state.turn_seq}` : null],
              ['behavior', state?.current_behavior ? <Mono>{state.current_behavior}</Mono> : null],
              ['process entry', state?.process_entry ? <Mono>{state.process_entry}</Mono> : null],
              ['live run', state?.live_run ? <Mono>{state.live_run.run_id}</Mono> : null],
              ['last run', state?.last_run ? <Mono>{state.last_run}</Mono> : null],
              ['stop requested', state?.stop_requested ? <Badge value="yes" tone="bad" /> : null],
              ['activity', state?.activity.summary || status.activity.summary || null],
              ['touching', (state?.activity.touching ?? status.activity.touching)?.length ? (
                <span>
                  {(state?.activity.touching ?? status.activity.touching ?? []).map((t) => (
                    <div key={`${t.kind}:${t.ref}`}><Mono>{t.mode} {t.kind}:{t.ref}</Mono></div>
                  ))}
                </span>
              ) : null],
              ['watched tasks', state?.watched_tasks?.length ? <Mono>{state.watched_tasks.join(', ')}</Mono> : null],
              ['pending decision', (state?.pending_decision ?? status.pending_decision) != null ? <Mono>{str(state?.pending_decision ?? status.pending_decision)}</Mono> : null],
            ]}
          />
        </Panel>
      </div>

      {lastError != null && (
        <Panel title="Last error" testId="session-last-error">
          <Pre value={lastError} className="text-bad" />
        </Panel>
      )}

      <div className="grid gap-x-3 lg:grid-cols-2">
        <Panel title={`Process stack (${state?.process_stack?.length ?? 0})`} testId="session-process-stack">
          {state?.process_stack?.length ? (
            <table className="tbl">
              <thead>
                <tr>
                  <th>entry</th>
                  <th>role</th>
                  <th>calls</th>
                  <th>run</th>
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
            <Empty>Empty.</Empty>
          )}
        </Panel>
        <Panel title={`Pending events (${state?.pending_events?.length ?? 0})`} testId="session-pending-events">
          {state?.pending_events?.length ? (
            <table className="tbl">
              <thead>
                <tr>
                  <th>source</th>
                  <th>subscription</th>
                  <th>latest</th>
                  <th>terminal</th>
                  <th>superseded</th>
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
            <Empty>None.</Empty>
          )}
        </Panel>
      </div>

      <Panel title={`Outbox (${state?.outbox?.length ?? 0})`} testId="session-outbox">
        {state?.outbox?.length ? <Outbox entries={state.outbox} /> : <Empty>No reply queued.</Empty>}
        {state?.reply != null && <div className="mt-1 text-xs text-mute">reply route: <Mono>{str(state.reply)}</Mono></div>}
      </Panel>

      <Panel
        title={`Worklog (last ${worklog.length}, newest first)`}
        testId="session-worklog"
        extra={
          <label className="flex items-center gap-1 text-xs text-mute">
            tail
            <select className="input" aria-label="Worklog tail" value={tail} onChange={(e) => setTail(Number(e.target.value))}>
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
          <Empty>No entries.</Empty>
        )}
      </Panel>

      <Panel title="Report" testId="session-report">
        {data.report ? <Pre value={data.report} /> : <Empty>{status.report_brief || 'No report.'}</Empty>}
      </Panel>

      <div className="grid gap-x-3 lg:grid-cols-2">
        <Panel title="Runtime & hosting" testId="session-runtime">
          <Fields
            rows={[
              ['binding', binding ? <span><Badge value={binding.kind} /> <Mono>{binding.runtime_id}</Mono></span> : null],
              ['workdir', binding ? <Mono>{binding.workdir}</Mono> : null],
              ['target', binding ? <Mono>{str(binding.target)}</Mono> : null],
              ['bound', binding ? <span><Time ms={binding.bound_at_ms} /> by <Mono>{binding.bound_by}</Mono></span> : null],
              ['lease holder', lease ? (
                <span>
                  <Mono>{lease.holder.principal} · {lease.holder.runner_id} · pid {lease.holder.pid}{lease.holder.host ? `@${lease.holder.host}` : ''} · epoch {lease.epoch}</Mono>{' '}
                  {lease.released_at_ms ? <Badge value="released" /> : <Badge value="held" tone="running" />}
                </span>
              ) : null],
              ['hosted', hosted ? (
                <span>
                  <Badge value={hosted.loaded ? 'loaded' : 'unloaded'} tone={hosted.loaded ? 'ok' : undefined} /> drives {hosted.drives} · last{' '}
                  <Badge value={hosted.last_result?.kind ?? '–'} /> <Time ms={hosted.last_result_at_ms} /> · {hosted.reason}
                </span>
              ) : <span className="text-xs text-mute">not hosted by this loader</span>],
              ['runs', data.runs?.length ? <Mono>{data.runs.join(', ')}</Mono> : null],
              ['statistics', data.statistics ? <Mono>{Object.entries(data.statistics).map(([k, v]) => `${k}=${v}`).join(' ')}</Mono> : null],
            ]}
          />
        </Panel>
        <Panel title="Behaviors (frozen)" testId="session-behaviors">
          <Fields
            rows={[
              ['entry behavior', config?.behavior ? <Mono>{config.behavior}</Mono> : null],
              ['catalog_rev', config?.frozen ? <Mono>{config.frozen.catalog_rev}</Mono> : null],
              ['frozen', config?.frozen ? <span><Time ms={config.frozen.frozen_at_ms} /> by <Mono>{config.frozen.frozen_by}</Mono></span> : null],
              ['behaviors', config?.frozen?.behaviors.length ? <Mono>{config.frozen.behaviors.join(', ')}</Mono> : null],
            ]}
          />
          {config && (
            <details className="mt-2">
              <summary className="cursor-pointer text-xs text-mute">session config</summary>
              <Pre value={{ session: config.session, runtime: config.runtime, workspace: config.workspace, subscriptions: config.subscriptions, channels: config.channels }} />
            </details>
          )}
        </Panel>
      </div>
    </>
  )
}
