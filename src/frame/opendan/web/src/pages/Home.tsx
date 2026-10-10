import { useMemo, useRef, useState, type ReactNode } from 'react'
import clsx from 'clsx'
import {
  Check,
  ChevronRight,
  Copy,
  ImagePlus,
  MessageCircle,
  Minus,
  Package,
  Pencil,
  Plus,
  Settings,
  Trash2,
} from 'lucide-react'
import { useSWRConfig } from 'swr'
import { ago, t, type TextKey } from '../i18n'
import { didName, fmtTokens, messageHubHref, sessionHref, useQuery } from '../lib'
import {
  dataModel,
  type AgentProfile,
  type ArtifactHead,
  type ModelUsage,
  type RegistryEntry,
  type UiBinding,
} from '../model'
import { ErrorBox } from '../ui'
import { WorkspaceList } from './Workspaces'

const AVATAR_SIZE = 192
const NAME_MAX = 64
const BIO_MAX = 500
const TOP_MODELS = 3
const RECENT_ITEMS = 5

// Inside the desktop's app window the page is framed: a link to MessageHub
// leaves the frame.
const outerTarget = window.top === window.self ? undefined : '_top'

function Avatar({ profile, src, className }: { profile: AgentProfile; src?: string | null; className?: string }) {
  const image = src === undefined ? profile.avatar : src
  return image ? (
    <img src={image} alt="" className={clsx('shrink-0 rounded-full object-cover', className)} />
  ) : (
    <div
      aria-hidden
      className={clsx('flex shrink-0 items-center justify-center rounded-full font-semibold text-white', className)}
      style={{ background: 'linear-gradient(135deg, var(--accent), color-mix(in srgb, var(--accent) 55%, var(--ok)))' }}
    >
      {[...profile.display_name.trim()][0]?.toUpperCase() ?? '?'}
    </div>
  )
}

/** A square, centre-cropped copy small enough to keep in the profile. */
const toAvatar = async (file: File): Promise<string> => {
  const bitmap = await createImageBitmap(file)
  const side = Math.min(bitmap.width, bitmap.height)
  const canvas = document.createElement('canvas')
  canvas.width = canvas.height = AVATAR_SIZE
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('no canvas')
  ctx.drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, AVATAR_SIZE, AVATAR_SIZE)
  bitmap.close()
  return canvas.toDataURL('image/jpeg', 0.85)
}

function ProfileDialog({ profile, onClose }: { profile: AgentProfile; onClose: () => void }) {
  const { mutate } = useSWRConfig()
  const [name, setName] = useState(profile.display_name)
  const [bio, setBio] = useState(profile.bio)
  const [avatar, setAvatar] = useState(profile.avatar)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const file = useRef<HTMLInputElement>(null)

  const pick = async (picked?: File) => {
    if (!picked) return
    try {
      setAvatar(await toAvatar(picked))
      setError(null)
    } catch {
      setError(new Error(t('imageFailed')))
    }
  }

  const save = async () => {
    if (!name.trim()) {
      setError(new Error(t('nameRequired')))
      return
    }
    setBusy(true)
    setError(null)
    try {
      // Unchanged fields are not sent: a name that is only the fallback
      // (the agent's user name) stays a fallback.
      const saved = await dataModel.setProfile({
        ...(name.trim() !== profile.display_name ? { display_name: name.trim() } : {}),
        ...(bio.trim() !== profile.bio ? { bio: bio.trim() } : {}),
        ...(avatar !== profile.avatar ? { avatar: avatar ?? '' } : {}),
      })
      await mutate(['agent.profile'], saved, { revalidate: false })
      onClose()
    } catch (e) {
      setError(e)
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-end justify-center bg-black/40 sm:items-center sm:p-4">
      <form
        role="dialog"
        aria-modal="true"
        aria-label={t('editProfile')}
        className="home max-h-[92dvh] w-full overflow-y-auto rounded-t-2xl border border-line bg-panel p-5 sm:max-w-md sm:rounded-2xl"
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
      >
        <h3 className="mb-4 text-base font-semibold">{t('editProfile')}</h3>
        <div className="mb-4 flex items-center gap-4">
          <Avatar profile={{ ...profile, display_name: name || profile.display_name }} src={avatar} className="h-20 w-20 text-3xl" />
          <div className="flex flex-col items-start gap-1">
            <input ref={file} type="file" accept="image/*" className="hidden" aria-label={t('changePhoto')} onChange={(e) => void pick(e.target.files?.[0])} />
            <button type="button" className="tap border border-line" onClick={() => file.current?.click()}>
              <ImagePlus size={16} />
              {t('changePhoto')}
            </button>
            {avatar && (
              <button type="button" className="tap text-bad" onClick={() => setAvatar(null)}>
                <Trash2 size={16} />
                {t('removePhoto')}
              </button>
            )}
          </div>
        </div>
        <label className="mb-3 block text-sm text-mute">
          {t('name')}
          <input
            className="mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg"
            value={name}
            maxLength={NAME_MAX}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="mb-4 block text-sm text-mute">
          <span className="flex justify-between">
            {t('bio')}
            <span className="tabular-nums">{[...bio].length}/{BIO_MAX}</span>
          </span>
          <textarea
            className="mt-1 block w-full rounded-xl border border-line bg-bg px-3 py-2 text-base text-fg"
            rows={4}
            value={bio}
            maxLength={BIO_MAX}
            placeholder={t('bioPlaceholder')}
            onChange={(e) => setBio(e.target.value)}
          />
        </label>
        <ErrorBox error={error} />
        <div className="flex gap-2">
          <button type="button" className="tap flex-1 border border-line" onClick={onClose} disabled={busy}>
            {t('cancel')}
          </button>
          <button type="submit" className="tap flex-1 bg-accent text-panel" disabled={busy}>
            {t('save')}
          </button>
        </div>
      </form>
    </div>
  )
}

function AgentCard({ profile }: { profile: AgentProfile }) {
  const [editing, setEditing] = useState(false)
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(profile.agent_did)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      // no clipboard here (insecure origin); the DID stays selectable
    }
  }
  return (
    <section className="card overflow-hidden" data-testid="agent-card">
      <div className="flex items-start gap-4 p-4">
        <Avatar profile={profile} className="h-16 w-16 text-2xl" />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-xl font-semibold leading-tight">{profile.display_name}</h1>
          <p className={clsx('mt-1 line-clamp-3 whitespace-pre-line text-sm', profile.bio ? 'text-fg' : 'text-mute')}>
            {profile.bio || t('noBio')}
          </p>
        </div>
        {profile.editable && (
          <button className="tap -mr-2 -mt-1 px-3 text-accent" onClick={() => setEditing(true)} aria-label={t('editProfile')}>
            <Pencil size={16} />
            <span className="hidden sm:inline">{t('edit')}</span>
          </button>
        )}
      </div>
      <div className="border-t border-line">
        <button className="flex min-h-11 w-full items-center gap-3 px-4 text-left" onClick={() => void copy()} aria-label={t('copyDid')}>
          <span className="shrink-0 text-xs font-medium text-mute">DID</span>
          <span className="min-w-0 flex-1 truncate font-mono text-xs" data-testid="agent-did">{profile.agent_did}</span>
          <span className="flex shrink-0 items-center gap-1 text-xs text-mute">
            {copied ? <Check size={14} className="text-ok" /> : <Copy size={14} />}
            {copied && t('copied')}
          </span>
        </button>
        <a href="#/agent" className="flex min-h-12 items-center gap-3 border-t border-line px-4">
          <Settings size={18} className="shrink-0 text-mute" />
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-medium">{t('settings')}</span>
            <span className="block truncate text-xs text-mute">{t('settingsHint')}</span>
          </span>
          <ChevronRight size={18} className="shrink-0 text-mute" />
        </a>
      </div>
      {editing && <ProfileDialog profile={profile} onClose={() => setEditing(false)} />}
    </section>
  )
}

// Working: a Turn is open or about to be. Pending: something waits for the
// owner (a result to accept) or has not started.
const isWorking = (e: RegistryEntry) =>
  e.status.run_state === 'running' ||
  e.status.run_state === 'ready' ||
  Boolean(e.status.turn_open) ||
  (e.status.run_state === 'waiting' && e.status.waiting_for !== 'input')
const isPending = (e: RegistryEntry) => e.status.acceptance === 'pending' || e.status.run_state === 'created'

const sessionStatus = (e: RegistryEntry): { key: TextKey; tone: string } => {
  if (e.status.acceptance === 'pending') return { key: 'stPendingAcceptance', tone: 'text-warn' }
  if (e.status.run_state === 'finished') {
    if (e.status.outcome === 'failed') return { key: 'stFailed', tone: 'text-bad' }
    if (e.status.outcome === 'stopped') return { key: 'stStopped', tone: 'text-mute' }
    return { key: 'stDone', tone: 'text-ok' }
  }
  if (isWorking(e)) return { key: 'stRunning', tone: 'text-accent' }
  if (e.status.run_state === 'created') return { key: 'stCreated', tone: 'text-mute' }
  return { key: 'stWaitingInput', tone: 'text-mute' }
}

function Stat({ label, value, tone, testId }: { label: string; value: number | undefined; tone?: string; testId: string }) {
  return (
    <div className="rounded-xl bg-soft px-3 py-2.5" data-testid={testId}>
      <div className={clsx('text-2xl font-semibold tabular-nums leading-tight', tone)}>{value ?? '–'}</div>
      <div className="text-xs text-mute">{label}</div>
    </div>
  )
}

function ModelRow({ usage, max }: { usage: ModelUsage; max: number }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_3.5rem_3.5rem_3.75rem] items-center gap-x-2 py-2" data-testid="model-row">
      <div className="min-w-0">
        <div className="line-clamp-2 break-all text-sm font-medium" title={usage.model}>{usage.model.split('@')[0]}</div>
        <div className="mt-1 h-1 overflow-hidden rounded-full bg-soft">
          <div className="h-full rounded-full bg-accent" style={{ width: `${max ? Math.max(2, (usage.all.total / max) * 100) : 0}%` }} />
        </div>
      </div>
      {[usage.hour, usage.day, usage.all].map((w, i) => (
        <div
          key={i}
          className={clsx('text-right text-sm tabular-nums', i === 2 ? 'font-semibold' : w.total ? '' : 'text-mute')}
          title={`${w.total.toLocaleString()} ${t('tokens')} (in ${w.input.toLocaleString()} / out ${w.output.toLocaleString()})`}
        >
          {fmtTokens(w.total)}
        </div>
      ))}
    </div>
  )
}

interface RecentItem {
  key: string
  title: string
  sub: string
  at: number
  href: string
}

const recentItems = (artifacts: ArtifactHead[], entries: RegistryEntry[]): RecentItem[] => {
  const latest = (pick: (e: RegistryEntry) => boolean) =>
    entries.filter(pick).sort((a, b) => b.status.updated_at_ms - a.status.updated_at_ms)[0]
  const items: RecentItem[] = artifacts.map((a) => {
    const session = latest((e) => e.artifact_id === a.aid)
    return {
      key: `artifact:${a.aid}`,
      title: a.aid,
      sub: a.head ? t('acceptedVersion', { ver: a.head }) : t('notAccepted'),
      at: a.updated_at_ms,
      href: session ? sessionHref(session.session_id) : '#/agent',
    }
  })
  return items.sort((a, b) => b.at - a.at).slice(0, RECENT_ITEMS)
}

function SubHeading({ children }: { children: ReactNode }) {
  return <h3 className="mb-1 mt-4 text-xs font-medium uppercase tracking-wide text-mute">{children}</h3>
}

function Overview({ entries }: { entries?: RegistryEntry[] }) {
  const usage = useQuery(['usage.models'], () => dataModel.usageModels())
  const artifacts = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const models = usage.data?.models.slice(0, TOP_MODELS) ?? []
  const recent = useMemo(() => recentItems(artifacts.data ?? [], entries ?? []), [artifacts.data, entries])
  return (
    <section className="card p-4" data-testid="work-overview">
      <h2 className="mb-3 text-base font-semibold">{t('overview')}</h2>
      <div className="grid grid-cols-3 gap-2">
        <Stat testId="stat-working" label={t('working')} value={entries?.filter(isWorking).length} tone="text-accent" />
        <Stat testId="stat-pending" label={t('pending')} value={entries?.filter(isPending).length} tone="text-warn" />
        <Stat testId="stat-total" label={t('total')} value={entries?.length} />
      </div>

      <div data-testid="model-usage">
        <div className="mt-4 grid grid-cols-[minmax(0,1fr)_3.5rem_3.5rem_3.75rem] gap-x-2 text-xs text-mute">
          <span className="font-medium uppercase tracking-wide">{t('topModels')}</span>
          <span className="text-right">{t('lastHour')}</span>
          <span className="text-right">{t('lastDay')}</span>
          <span className="text-right">{t('allTime')}</span>
        </div>
        <ErrorBox error={usage.error} />
        {models.length ? (
          <div className="divide-y divide-line">
            {models.map((m) => (
              <ModelRow key={m.model} usage={m} max={models[0].all.total} />
            ))}
          </div>
        ) : (
          <p className="py-3 text-sm text-mute">
            {usage.isLoading ? t('loading') : usage.data === null ? t('usageUnsupported') : t('noUsage')}
          </p>
        )}
        {usage.data?.since_ms != null && (
          <p className="text-xs text-mute">
            {t('usageSince', { date: new Date(usage.data.since_ms).toLocaleDateString() })} · {t('tokens')}
          </p>
        )}
      </div>

      <SubHeading>{t('recent')}</SubHeading>
      <ErrorBox error={artifacts.error} />
      {recent.length ? (
        <ul className="-mx-2" data-testid="recent-items">
          {recent.map((item) => (
            <li key={item.key}>
              <a href={item.href} className="flex min-h-12 items-center gap-3 rounded-xl px-2 py-1.5 active:bg-soft">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-soft text-accent" title={t('artifact')}>
                  <Package size={18} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{item.title}</span>
                  <span className="block truncate text-xs text-mute">
                    {t('artifact')} · {item.sub}
                  </span>
                </span>
                <span className="shrink-0 text-xs text-mute">{ago(item.at)}</span>
              </a>
            </li>
          ))}
        </ul>
      ) : (
        <p className="py-2 text-sm text-mute">{artifacts.isLoading ? t('loading') : t('noRecent')}</p>
      )}
    </section>
  )
}

interface Tree {
  roots: RegistryEntry[]
  children: Map<string, RegistryEntry[]>
}

const toTree = (entries: RegistryEntry[]): Tree => {
  const ids = new Set(entries.map((e) => e.session_id))
  const byUpdate = (a: RegistryEntry, b: RegistryEntry) => b.status.updated_at_ms - a.status.updated_at_ms
  const children = new Map<string, RegistryEntry[]>()
  const roots: RegistryEntry[] = []
  for (const e of [...entries].sort(byUpdate)) {
    const parent = e.origin?.parent_session
    if (parent && parent !== e.session_id && ids.has(parent)) children.set(parent, [...(children.get(parent) ?? []), e])
    else roots.push(e)
  }
  return { roots, children }
}

const KIND_LABEL: Partial<Record<RegistryEntry['kind'], TextKey>> = {
  work: 'kindWork',
  self_check: 'kindSelfCheck',
  self_improve: 'kindSelfImprove',
}

const sessionTitle = (e: RegistryEntry, binding?: UiBinding): string => {
  if (e.kind === 'ui' && binding) return t(binding.kind === 'group_msg' ? 'groupChat' : 'chatWith', { name: didName(binding.to) })
  return e.objective || e.session_id
}

function SessionRow({
  entry,
  depth,
  tree,
  profile,
  bindings,
  open,
  toggle,
}: {
  entry: RegistryEntry
  depth: number
  tree: Tree
  profile: AgentProfile
  bindings: Map<string, UiBinding>
  open: Set<string>
  toggle: (sid: string) => void
}) {
  const kids = tree.children.get(entry.session_id) ?? []
  const expanded = open.has(entry.session_id)
  const binding = bindings.get(entry.session_id)
  const hub = messageHubHref(profile, entry, binding)
  const status = sessionStatus(entry)
  const kind = KIND_LABEL[entry.kind]
  const line = entry.kind === 'ui' ? entry.status.one_line_status : entry.status.one_line_status || entry.status.report_brief
  return (
    <li>
      <div className="flex items-stretch" style={{ paddingLeft: depth * 20 }} data-testid={`home-session-${entry.session_id}`}>
        {kids.length ? (
          <button
            className="flex w-10 shrink-0 items-center justify-center text-mute"
            onClick={() => toggle(entry.session_id)}
            aria-expanded={expanded}
            aria-label={`${t(expanded ? 'collapse' : 'expand')} (${kids.length})`}
          >
            <span className="flex h-6 w-6 items-center justify-center rounded-md border border-line">
              {expanded ? <Minus size={14} /> : <Plus size={14} />}
            </span>
          </button>
        ) : (
          <span className="w-10 shrink-0" />
        )}
        <a
          href={hub ?? sessionHref(entry.session_id)}
          target={hub ? outerTarget : undefined}
          title={t(hub ? 'opensMessageHub' : 'opensDetail')}
          data-target={hub ? 'messagehub' : 'detail'}
          className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-xl py-2 pr-2 active:bg-soft"
        >
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">{sessionTitle(entry, binding)}</span>
            <span className="block truncate text-xs text-mute">
              <span className={clsx('font-medium', status.tone)}>{t(status.key)}</span>
              {kind && ` · ${t(kind)}`}
              {line && ` · ${line}`}
            </span>
          </span>
          <span className="shrink-0 text-xs text-mute">{ago(entry.status.updated_at_ms)}</span>
          {hub ? <MessageCircle size={18} className="shrink-0 text-accent" /> : <ChevronRight size={18} className="shrink-0 text-mute" />}
        </a>
      </div>
      {expanded && (
        <ul>
          {kids.map((c) => (
            <SessionRow key={c.session_id} entry={c} depth={depth + 1} tree={tree} profile={profile} bindings={bindings} open={open} toggle={toggle} />
          ))}
        </ul>
      )}
    </li>
  )
}

function SessionList({ entries, profile, loading }: { entries?: RegistryEntry[]; profile: AgentProfile; loading: boolean }) {
  const bindingList = useQuery(['ui.bindings'], () => dataModel.uiBindings())
  const bindings = useMemo(() => new Map((bindingList.data ?? []).map((b) => [b.session_id, b])), [bindingList.data])
  const tree = useMemo(() => toTree(entries ?? []), [entries])
  const [open, setOpen] = useState<Set<string>>(new Set())
  const toggle = (sid: string) =>
    setOpen((previous) => {
      const next = new Set(previous)
      if (!next.delete(sid)) next.add(sid)
      return next
    })
  // Outside a zone there is no MessageHub: the default conversation is the
  // latest UI session, shown on its detail page.
  const latestUi = tree.roots.find((e) => e.kind === 'ui')
  const defaultHref = messageHubHref(profile) ?? (latestUi ? sessionHref(latestUi.session_id) : undefined)
  return (
    <section className="card p-4" data-testid="session-list">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-base font-semibold">
          {t('sessions')}
          {entries && <span className="ml-1.5 text-sm font-normal text-mute">{tree.roots.length}</span>}
        </h2>
      </div>
      <a
        href={defaultHref}
        target={profile.desktop_url ? outerTarget : undefined}
        aria-disabled={!defaultHref}
        data-testid="open-default-session"
        className={clsx('tap mb-2 w-full bg-accent text-panel', !defaultHref && 'pointer-events-none opacity-50')}
      >
        <MessageCircle size={18} />
        {t('openDefault')}
      </a>
      <ErrorBox error={bindingList.error} />
      {tree.roots.length ? (
        <ul className="-mx-2 divide-y divide-line">
          {tree.roots.map((e) => (
            <SessionRow key={e.session_id} entry={e} depth={0} tree={tree} profile={profile} bindings={bindings} open={open} toggle={toggle} />
          ))}
        </ul>
      ) : (
        <p className="py-3 text-sm text-mute">{loading ? t('loading') : t('noSessions')}</p>
      )}
    </section>
  )
}

export default function HomePage() {
  const profile = useQuery(['agent.profile'], () => dataModel.profile())
  const sessions = useQuery(['sessions.query'], () => dataModel.sessions())
  return (
    <div className="home mx-auto max-w-5xl">
      <ErrorBox error={profile.error ?? sessions.error} />
      {profile.data ? (
        <div className="grid grid-cols-[minmax(0,1fr)] items-start gap-3 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
          <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
            <AgentCard profile={profile.data} />
            <Overview entries={sessions.data} />
            <WorkspaceList entries={sessions.data} />
          </div>
          <SessionList entries={sessions.data} profile={profile.data} loading={sessions.isLoading} />
        </div>
      ) : (
        !profile.error && <p className="py-10 text-center text-sm text-mute">{t('loading')}</p>
      )}
    </div>
  )
}
