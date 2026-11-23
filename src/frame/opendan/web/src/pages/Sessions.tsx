import { stateText, t } from '../i18n'
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
    <label className="flex items-center gap-2 text-xs text-mute">
      {label}
      <select className="input" aria-label={label} value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">{t('observeAll')}</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {stateText(o)}
          </option>
        ))}
      </select>
    </label>
  )
}

function HostedMark({ hosted }: { hosted?: HostedStatus }) {
  if (!hosted) return <span className="text-xs text-mute">–</span>
  return <Badge value={hosted.loaded ? t('observeLoaded') : t('observeUnloaded')} tone={hosted.loaded ? 'ok' : undefined} />
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
      <Panel title={t('gsActive')} count={active.data?.length ?? 0} testId="active-panel">
        {active.data?.length ? (
          <ul className="divide-y divide-line">
            {active.data.map((a) => (
              <li key={a.session_id} className="grid gap-x-6 gap-y-2 py-3 first:pt-1 last:pb-0 md:grid-cols-[minmax(0,13rem)_minmax(0,1fr)_minmax(0,14rem)]" data-testid="active-row">
                <div className="flex min-w-0 flex-col gap-1">
                  <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <Badge value={a.run_state} />
                    {a.possibly_interrupted && <Badge value={t('observePossiblyInterrupted')} tone="bad" />}
                  </span>
                  <span className="flex items-baseline gap-2">
                    <Sid sid={a.session_id} />
                    <span className="text-xs text-mute">{a.kind}</span>
                  </span>
                </div>
                <div className="min-w-0">
                  <p className="text-sm leading-5">{a.activity.summary || a.objective}</p>
                  {(a.activity.touching ?? []).length > 0 && (
                    <ul className="mt-1 font-mono text-xs leading-5">
                      {(a.activity.touching ?? []).map((t) => (
                        <li key={`${t.kind}:${t.ref}`} className="flex gap-2">
                          <span className="w-10 shrink-0 text-mute">{t.mode}</span>
                          <span className="min-w-0 break-words">{t.kind}:{t.ref}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                  {a.overlap.length > 0 && <div className="mt-1"><Badge value={t('observeOverlap', { items: a.overlap.join(', ') })} tone="pending" /></div>}
                </div>
                <div className="flex min-w-0 flex-col gap-1 text-xs md:items-end md:text-right">
                  <span className="tabular-nums text-mute" title={t('observeHeartbeat')}>{fmtAgo(a.activity.heartbeat_ms)}</span>
                  <Mono className="text-mute">{a.driver}</Mono>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <Empty>{t('observeNoSessionIsRunningOrWaiting')}</Empty>
        )}
      </Panel>
      <Panel
        title={t('sessions')}
        count={filtered ? t('observePartsOf', { count: visible.length, total: rows.length }) : visible.length}
        testId="sessions-panel"
        extra={
          <>
            <Select label={t('observeKind')} value={kind} options={kinds} onChange={setKind} />
            <Select label={t('observeClass')} value={cls} options={classes} onChange={setCls} />
            <Select label={t('observeRunState')} value={runState} options={['created', 'ready', 'running', 'waiting', 'finished']} onChange={setRunState} />
          </>
        }
      >
        {visible.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>sid</th>
                <th>{t('observeKindClass')}</th>
                <th>{t('observeRunState')}</th>
                <th>{t('observeOutcome')}</th>
                <th>{t('observeStatus')}</th>
                <th>{t('observeDriver')}</th>
                <th>{t('observeHosted')}</th>
                <th>{t('observeUpdated')}</th>
              </tr>
            </thead>
            <tbody>
              {visible.map(({ entry, depth }) => (
                <tr key={entry.session_id} data-testid={`session-row-${entry.session_id}`}>
                  <td style={{ paddingLeft: filtered ? 0 : depth * 16 }} className="whitespace-nowrap">
                    {depth > 0 && <span className="mr-1.5 font-mono text-xs text-bar">{filtered ? '↳' : '└'}</span>}
                    <Sid sid={entry.session_id} />
                    {filtered && entry.origin?.parent_session && <span className="ml-1.5 text-xs text-mute">{t('observeOf')} {entry.origin.parent_session}</span>}
                    {entry.unreachable && <span className="ml-2"><Badge value={t('observeUnreachable')} tone="bad" /></span>}
                  </td>
                  <td className="whitespace-nowrap">
                    {entry.kind}
                    {entry.class && entry.class !== entry.kind && <span className="text-mute"> / {entry.class}</span>}
                  </td>
                  <td className="whitespace-nowrap">
                    <span className="inline-flex items-baseline gap-x-2">
                      <Badge value={entry.status.run_state} />
                      {entry.status.needs_input && <span className="text-xs text-mute">{t('observeNeedsInput')}</span>}
                      {entry.status.turn_open && <span className="text-xs text-mute">{t('observeTurnOpen')}</span>}
                      {entry.status.paused && <Badge value={`paused: ${entry.status.paused}`} tone="waiting" />}
                    </span>
                  </td>
                  <td className="whitespace-nowrap">
                    <span className="inline-flex items-baseline gap-x-3">
                      {entry.status.outcome && <Badge value={entry.status.outcome} />}
                      {entry.status.acceptance !== 'n/a' && <Badge value={entry.status.acceptance} />}
                    </span>
                  </td>
                  <td>
                    {entry.status.one_line_status || <span className="text-mute">{entry.objective}</span>}
                    {entry.status.last_error != null && <span className="ml-2"><Badge value={t('observeError')} tone="bad" /></span>}
                    {entry.runtime_unavailable && (
                      <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-xs text-mute"><Badge value={t('observeRuntimeUnavailable')} tone="waiting" /> {entry.runtime_unavailable}</span>
                    )}
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
