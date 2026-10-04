import { useMemo, useState } from 'react'
import { fmtAgo, useQuery } from '../lib'
import { dataModel, type HostedStatus, type RegistryEntry } from '../model'
import { Badge, Empty, ErrorBox, Mono, Panel, Sid, Time } from '../ui'

interface Row {
  entry: RegistryEntry
  depth: number
}

const toTree = (entries: RegistryEntry[]): Row[] => {
  const ids = new Set(entries.map((e) => e.session_id))
  const children = new Map<string, RegistryEntry[]>()
  const roots: RegistryEntry[] = []
  for (const e of entries) {
    const parent = e.origin?.parent_session
    if (parent && ids.has(parent) && parent !== e.session_id) {
      children.set(parent, [...(children.get(parent) ?? []), e])
    } else {
      roots.push(e)
    }
  }
  const byUpdate = (a: RegistryEntry, b: RegistryEntry) => b.status.updated_at_ms - a.status.updated_at_ms
  const rows: Row[] = []
  const seen = new Set<string>()
  const walk = (e: RegistryEntry, depth: number) => {
    if (seen.has(e.session_id)) return
    seen.add(e.session_id)
    rows.push({ entry: e, depth })
    for (const c of (children.get(e.session_id) ?? []).sort(byUpdate)) walk(c, depth + 1)
  }
  for (const r of roots.sort(byUpdate)) walk(r, 0)
  for (const e of entries) walk(e, 0)
  return rows
}

function Select({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange: (v: string) => void }) {
  return (
    <label className="flex items-center gap-1 text-xs text-mute">
      {label}
      <select className="input" aria-label={label} value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">all</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </label>
  )
}

function HostedMark({ hosted }: { hosted?: HostedStatus }) {
  if (!hosted) return <span className="text-xs text-mute">–</span>
  return <Badge value={hosted.loaded ? 'loaded' : 'unloaded'} tone={hosted.loaded ? 'ok' : undefined} />
}

export default function SessionsPage() {
  const sessions = useQuery(['sessions.query'], () => dataModel.sessions())
  const active = useQuery(['activity.active'], () => dataModel.activeSessions())
  const status = useQuery(['loader.status'], () => dataModel.loaderStatus())
  const [kind, setKind] = useState('')
  const [cls, setCls] = useState('')
  const [runState, setRunState] = useState('')

  const entries = sessions.data
  const rows = useMemo(() => toTree(entries ?? []), [entries])
  const hosted = useMemo(() => new Map((status.data?.hosted ?? []).map((h) => [h.session_id, h])), [status.data])
  const kinds = [...new Set(rows.map((r) => r.entry.kind))].sort()
  const classes = [...new Set(rows.map((r) => r.entry.class).filter(Boolean))].sort()
  const filtered = kind || cls || runState
  const visible = rows.filter(
    ({ entry }) => (!kind || entry.kind === kind) && (!cls || entry.class === cls) && (!runState || entry.status.run_state === runState),
  )

  return (
    <>
      <ErrorBox error={sessions.error ?? active.error} />
      <Panel title={`Active (${active.data?.length ?? 0})`} testId="active-panel">
        {active.data?.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>sid</th>
                <th>kind</th>
                <th>state</th>
                <th>doing</th>
                <th>touching</th>
                <th>driver</th>
                <th>heartbeat</th>
              </tr>
            </thead>
            <tbody>
              {active.data.map((a) => (
                <tr key={a.session_id}>
                  <td>
                    <Sid sid={a.session_id} />
                  </td>
                  <td>{a.kind}</td>
                  <td>
                    <Badge value={a.run_state} /> {a.possibly_interrupted && <Badge value="possibly interrupted" tone="bad" />}
                  </td>
                  <td>{a.activity.summary || a.objective}</td>
                  <td>
                    {(a.activity.touching ?? []).map((t) => (
                      <div key={`${t.kind}:${t.ref}`}>
                        <Mono>
                          {t.mode} {t.kind}:{t.ref}
                        </Mono>
                      </div>
                    ))}
                    {a.overlap.length > 0 && <Badge value={`overlap: ${a.overlap.join(', ')}`} tone="pending" />}
                  </td>
                  <td>
                    <Mono>{a.driver}</Mono>
                  </td>
                  <td className="whitespace-nowrap text-xs text-mute">{fmtAgo(a.activity.heartbeat_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>No session is running or waiting.</Empty>
        )}
      </Panel>
      <Panel
        title={`Sessions (${visible.length}${filtered ? ` of ${rows.length}` : ''})`}
        testId="sessions-panel"
        extra={
          <div className="flex flex-wrap gap-3">
            <Select label="kind" value={kind} options={kinds} onChange={setKind} />
            <Select label="class" value={cls} options={classes} onChange={setCls} />
            <Select label="run_state" value={runState} options={['created', 'ready', 'running', 'waiting', 'finished']} onChange={setRunState} />
          </div>
        }
      >
        {visible.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>sid</th>
                <th>kind / class</th>
                <th>run_state</th>
                <th>outcome</th>
                <th>status</th>
                <th>driver</th>
                <th>hosted</th>
                <th>updated</th>
              </tr>
            </thead>
            <tbody>
              {visible.map(({ entry, depth }) => (
                <tr key={entry.session_id} data-testid={`session-row-${entry.session_id}`}>
                  <td style={{ paddingLeft: 8 + (filtered ? 0 : depth * 18) }} className="whitespace-nowrap">
                    {depth > 0 && <span className="mr-1 text-mute">{filtered ? '↳' : '└'}</span>}
                    <Sid sid={entry.session_id} />
                    {filtered && entry.origin?.parent_session && <span className="ml-1 text-xs text-mute">of {entry.origin.parent_session}</span>}
                    {entry.unreachable && <span className="ml-1"><Badge value="unreachable" tone="bad" /></span>}
                  </td>
                  <td className="whitespace-nowrap">
                    {entry.kind}
                    {entry.class && entry.class !== entry.kind && <span className="text-mute"> / {entry.class}</span>}
                  </td>
                  <td className="whitespace-nowrap">
                    <Badge value={entry.status.run_state} />
                    {entry.status.waiting_for && <span className="ml-1 text-xs text-mute">for {entry.status.waiting_for}</span>}
                    {entry.status.turn_open && <span className="ml-1"><Badge value="turn open" tone="running" /></span>}
                  </td>
                  <td className="whitespace-nowrap">
                    {entry.status.outcome && <Badge value={entry.status.outcome} />}
                    {entry.status.acceptance !== 'n/a' && <span className="ml-1"><Badge value={entry.status.acceptance} /></span>}
                  </td>
                  <td>
                    {entry.status.one_line_status || <span className="text-mute">{entry.objective}</span>}
                    {entry.status.last_error != null && <span className="ml-1"><Badge value="error" tone="bad" /></span>}
                  </td>
                  <td>
                    <Mono>{entry.driver.principal}</Mono>
                  </td>
                  <td>
                    <HostedMark hosted={hosted.get(entry.session_id)} />
                  </td>
                  <td>
                    <Time ms={entry.status.updated_at_ms} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{sessions.isLoading ? 'Loading…' : 'No sessions.'}</Empty>
        )}
      </Panel>
    </>
  )
}
