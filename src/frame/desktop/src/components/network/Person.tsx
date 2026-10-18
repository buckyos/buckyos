/* ── Person and group visuals shared by every network component ── */

import clsx from 'clsx'
import { AtSign, Bot, CircleHelp, Globe, MonitorSmartphone, User, Users } from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { ObjectAvatar } from '../../presentation/ObjectIcon'
import { shortDid, usePersonPresentation, type PersonRef } from './personPresentation'
import type { LookupKind, Reachability } from './types'

const KIND_TONE: Record<LookupKind, string> = {
  user: 'var(--cp-accent)',
  agent: 'var(--cp-success)',
  group: 'var(--cp-warning)',
  zone: 'var(--cp-accent)',
  device: 'var(--cp-muted)',
  msgtunnel: 'var(--cp-muted)',
  unknown: 'var(--cp-muted)',
}

function KindGlyph({ kind, size }: { kind: LookupKind; size: number }) {
  switch (kind) {
    case 'agent': return <Bot size={size} />
    case 'group': return <Users size={size} />
    case 'zone': return <Globe size={size} />
    case 'device': return <MonitorSmartphone size={size} />
    case 'msgtunnel': return <AtSign size={size} />
    case 'unknown': return <CircleHelp size={size} />
    default: return <User size={size} />
  }
}

export function PersonAvatar({ person, size = 36 }: { person: PersonRef; size?: number }) {
  const presentation = usePersonPresentation(person)
  const kind = person.kind ?? 'user'
  return (
    <ObjectAvatar
      title={presentation.title}
      icons={presentation.icons}
      size={size}
      tone={KIND_TONE[kind]}
      fallbackIcon={kind === 'user' ? undefined : <KindGlyph kind={kind} size={Math.round(size * 0.48)} />}
      decorative
    />
  )
}

export function PersonName({ person, className }: { person: PersonRef; className?: string }) {
  const title = usePersonPresentation(person).title
  return <span className={clsx('truncate', className)} title={title}>{title}</span>
}

export function KindLabel({ kind }: { kind: LookupKind }) {
  const { t } = useI18n()
  return <>{t(`network.kind.${kind}`)}</>
}

export function Pill({ tone, children, testId, title }: { tone: string; children: ReactNode; testId?: string; title?: string }) {
  return (
    <span
      className="inline-flex max-w-full items-center gap-1 truncate rounded-full px-2 py-0.5 text-[11px] font-medium leading-4"
      style={{ background: `color-mix(in srgb, ${tone} 15%, transparent)`, color: `color-mix(in srgb, ${tone} 70%, var(--cp-text))` }}
      data-testid={testId}
      title={title}
    >
      {children}
    </span>
  )
}

const REACH_TONE: Record<Reachability['state'], string> = {
  local: 'var(--cp-success)',
  tunnel: 'var(--cp-muted)',
  routable: 'var(--cp-accent)',
  unknown: 'var(--cp-muted)',
  unreachable: 'var(--cp-danger)',
}

export function ReachabilityChip({ reachability }: { reachability: Reachability }) {
  const { t } = useI18n()
  const label = t(`network.reach.${reachability.state}`, reachability.state, { zone: reachability.zone ?? '' })
  return <Pill tone={REACH_TONE[reachability.state]} testId="network-reachability" title={reachability.reason}>{label}</Pill>
}

/** One dense row: picture, name, a secondary line and trailing content (badges, actions, a checkbox). */
export function PersonRow({
  person,
  secondary,
  leading,
  trailing,
  selected = false,
  disabled = false,
  onClick,
  testId,
}: {
  person: PersonRef
  secondary?: ReactNode
  leading?: ReactNode
  trailing?: ReactNode
  selected?: boolean
  disabled?: boolean
  onClick?: () => void
  testId?: string
}) {
  const title = usePersonPresentation(person).title
  const body = (
    <>
      {leading}
      <PersonAvatar person={person} size={34} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-[color:var(--cp-text)]" title={title}>{title}</span>
        <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{secondary ?? shortDid(person.did)}</span>
      </span>
    </>
  )
  const style = selected
    ? { background: 'color-mix(in srgb, var(--cp-accent-soft) 20%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))' }
    : undefined
  return (
    <div
      className={clsx('flex min-h-12 items-center gap-2.5 rounded-xl border border-transparent px-2.5 py-1.5', disabled && 'opacity-60')}
      style={style}
      data-testid={testId}
      data-did={person.did}
    >
      {onClick ? (
        <button type="button" className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg text-left" onClick={onClick} disabled={disabled} aria-label={title}>
          {body}
        </button>
      ) : (
        <span className="flex min-w-0 flex-1 items-center gap-2.5">{body}</span>
      )}
      {trailing ? <span className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">{trailing}</span> : null}
    </div>
  )
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 px-1 pb-1 pt-3">
      <h3 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{children}</h3>
      {aside}
    </div>
  )
}

export function StateMessage({ tone = 'muted', children, action, testId }: { tone?: 'muted' | 'danger' | 'success' | 'warning'; children: ReactNode; action?: ReactNode; testId?: string }) {
  const color = tone === 'danger' ? 'var(--cp-danger)' : tone === 'success' ? 'var(--cp-success)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-muted)'
  return (
    <div
      role={tone === 'danger' ? 'alert' : 'status'}
      data-testid={testId}
      className="flex flex-wrap items-center justify-between gap-2 rounded-xl border px-3 py-2.5 text-sm"
      style={{
        borderColor: `color-mix(in srgb, ${color} 30%, var(--cp-border))`,
        background: `color-mix(in srgb, ${color} 8%, var(--cp-surface))`,
        color: tone === 'muted' ? 'var(--cp-muted)' : `color-mix(in srgb, ${color} 62%, var(--cp-text))`,
      }}
    >
      <span className="min-w-0 flex-1">{children}</span>
      {action}
    </div>
  )
}

export function ListSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div className="space-y-1.5" aria-hidden="true" data-testid="network-loading">
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="flex min-h-12 items-center gap-2.5 px-2.5">
          <span className="h-[34px] w-[34px] shrink-0 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-muted)_16%,var(--cp-surface))]" />
          <span className="h-3 flex-1 animate-pulse rounded bg-[color:color-mix(in_srgb,var(--cp-muted)_14%,var(--cp-surface))]" />
        </div>
      ))}
    </div>
  )
}
