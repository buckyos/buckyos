import clsx from 'clsx'
import {
  ChevronDown,
  ChevronRight,
  Clock,
  Cpu,
  FlaskConical,
  FolderClosed,
  FolderOpen,
  Globe,
  Hash,
  House,
  Library,
  Lock,
  LockOpen,
  Monitor,
  Pin,
  PinOff,
  Plus,
  Share2,
  Star,
} from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { isMockRuntime } from '../../runtime'
import type { CollectionSummary, DfsNode, HostInfo, Topic } from './types'
import type { FolderChildrenPage, SidebarSource } from './data/sidebarSources'
import { collectionUrl, displayPath, normalizeUrl } from './data/urls'
import { FolderTree, SectionError, type FolderTreeRoot } from './FolderTree'

type Section = 'dfs' | 'devices' | 'views' | 'collections'

interface SidebarProps {
  /** Per-source async states (§4.3) — one failing source never blanks the rest. */
  dfs: SidebarSource<DfsNode[]>
  /** The computer (host realm); null data = not offered to this user. */
  host: SidebarSource<HostInfo | null>
  /** The computer tree is unlocked (BuckyOS sudo held by this window). */
  hostUnlocked: boolean
  onUnlockHost: () => void
  /** Root of the unlocked computer tree. */
  hostRootUrl: string | null
  loadHostChildren: (url: string, cursor?: string) => Promise<FolderChildrenPage>
  topics: SidebarSource<Topic[]>
  /** Live store projection (mock in-memory today). */
  collections: CollectionSummary[]
  /** Canonical url of the active pane location. */
  activeUrl: string
  /** "My files": the signed-in user's own home. */
  homeUrl: string
  /** Where this window opened (the folder tree stays collapsed until the location moves on). */
  openedAt: string
  loadChildren: (url: string, cursor?: string) => Promise<FolderChildrenPage>
  /** Accepts bare paths and canonical urls alike (normalized by the pane). */
  onNavigate: (url: string) => void
  onCreateCollection: () => void
  compact?: boolean
  onAfterNavigate?: () => void
}

function nodeIcon(kind: DfsNode['kind']) {
  switch (kind) {
    case 'public':
      return <Globe size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
    case 'shared':
      return <Share2 size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
    case 'privacy':
      return <Lock size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
    case 'home':
      return <FolderOpen size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
    default:
      return <FolderClosed size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
  }
}

function SectionHeader({
  label,
  hint,
  collapsed,
  action,
}: {
  label: string
  hint?: string
  /** Set for a collapsible section: draws the disclosure chevron. */
  collapsed?: boolean
  action?: React.ReactNode
}) {
  return (
    <div className="flex h-7 items-center gap-2 px-2 pt-4 box-content">
      <span className="files-label min-w-0 flex-1 truncate">{label}</span>
      {hint ? (
        <span className="text-[12px] tabular-nums text-[color:var(--cp-muted)]">{hint}</span>
      ) : null}
      {collapsed !== undefined ? (
        collapsed ? <ChevronRight size={14} className="shrink-0 text-[color:var(--cp-muted)]" /> : <ChevronDown size={14} className="shrink-0 text-[color:var(--cp-muted)]" />
      ) : null}
      {action ? <span className="shrink-0">{action}</span> : null}
    </div>
  )
}

function SectionSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="space-y-2 px-2 py-2" data-testid="sidebar-section-skeleton">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex items-center gap-2">
          <span className="h-4 w-4 animate-pulse rounded-[4px] bg-[color:color-mix(in_srgb,var(--cp-border)_50%,transparent)]" />
          <span
            className="inline-block h-3 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_50%,transparent)]"
            style={{ width: `${64 - (i % 3) * 14}%` }}
          />
        </div>
      ))}
    </div>
  )
}

/**
 * Standard per-source rendering (§4.3): skeleton while first loading, inline
 * error + retry scoped to the section, last successful projection kept during
 * refreshes, and a section-specific empty message.
 */
function SourceSection<T>({
  source,
  empty,
  children,
}: {
  source: SidebarSource<T[]>
  empty: string
  children: (data: T[]) => React.ReactNode
}) {
  const { t } = useI18n()
  const { state, retry } = source
  if (state.data) {
    return (
      <>
        {state.status === 'error' && state.error ? (
          <SectionError message={t(state.error.messageKey, state.error.fallback)} onRetry={retry} />
        ) : null}
        {state.data.length ? (
          children(state.data)
        ) : (
          <p className="px-2 py-1 text-[12px] leading-4 text-[color:var(--cp-muted)]">{empty}</p>
        )}
      </>
    )
  }
  if (state.status === 'error' && state.error) {
    return (
      <SectionError message={t(state.error.messageKey, state.error.fallback)} onRetry={retry} />
    )
  }
  return <SectionSkeleton />
}

function navButton({
  key,
  icon,
  label,
  hint,
  title,
  active,
  onClick,
}: {
  key: string
  icon: React.ReactNode
  label: string
  hint?: string
  title?: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      key={key}
      type="button"
      onClick={onClick}
      title={title}
      aria-current={active ? 'page' : undefined}
      className={clsx('fb-nav-row flex h-8 w-full items-center gap-2 rounded-[8px] px-2 text-left text-[14px]', active ? 'font-medium' : 'text-[color:var(--cp-text)]')}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {hint ? (
        <span className="shrink-0 text-[12px] tabular-nums text-[color:var(--cp-muted)]">{hint}</span>
      ) : null}
    </button>
  )
}

export function Sidebar({
  dfs,
  host,
  hostUnlocked,
  onUnlockHost,
  hostRootUrl,
  loadHostChildren,
  topics,
  collections,
  activeUrl,
  homeUrl,
  openedAt,
  loadChildren,
  onNavigate,
  onCreateCollection,
  compact = false,
  onAfterNavigate,
}: SidebarProps) {
  const { t } = useI18n()
  const [pins, setPins] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('files.pins') ?? '[]') } catch { return [] } })
  const [groups, setGroups] = useState<Record<string, boolean>>(() => { try { return JSON.parse(localStorage.getItem('files.sidebarGroups') ?? '{}') } catch { return {} } })
  const toggleGroup = (key: string) => setGroups((prev) => { const next = { ...prev, [key]: !(prev[key] ?? true) }; localStorage.setItem('files.sidebarGroups', JSON.stringify(next)); return next })
  const pinCurrent = () => setPins((prev) => { const next = prev.includes(activeUrl) ? prev.filter((url) => url !== activeUrl) : [...prev, activeUrl]; localStorage.setItem('files.pins', JSON.stringify(next)); return next })
  const hostInfo = host.state.data ?? null
  const [activeSection, setActiveSection] = useState<Section | null>(compact ? 'dfs' : null)

  const sections: { id: Section; label: string; icon: React.ReactNode }[] = [
    { id: 'dfs', label: t('filebrowser.sidebar.myFiles', 'My files'), icon: <FolderOpen size={16} /> },
    { id: 'views', label: t('filebrowser.sidebar.views', 'Views'), icon: <Hash size={16} /> },
    {
      id: 'collections',
      label: t('filebrowser.sidebar.collections', 'Collections'),
      icon: <Library size={16} />,
    },
    ...(hostInfo ? [{ id: 'devices' as const, label: t('filebrowser.sidebar.devices', 'Devices'), icon: <Cpu size={16} /> }] : []),
  ]

  const goto = (url: string) => {
    onNavigate(url)
    onAfterNavigate?.()
  }

  const favorites = (
    <div className="space-y-0.5">
      {navButton({
        key: 'my-files',
        icon: <House size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
        label: t('filebrowser.sidebar.myFiles', 'My files'),
        active: activeUrl === homeUrl,
        onClick: () => goto(homeUrl),
      })}
      {navButton({
        key: 'recent',
        icon: <Clock size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
        label: t('filebrowser.sidebar.recent', 'Recent'),
        active: activeUrl === 'view://recent',
        onClick: () => goto('view://recent'),
      })}
      {pins.map((url) =>
        navButton({
          key: `pin:${url}`,
          icon: <Star size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
          label: url.split('/').filter(Boolean).at(-1) ?? url,
          title: displayPath(url),
          active: normalizeUrl(url) === activeUrl,
          onClick: () => goto(url),
        }),
      )}
      <button type="button" className="fb-nav-row flex h-8 w-full items-center gap-2 rounded-[8px] px-2 text-left text-[13px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]" onClick={pinCurrent}>
        {pins.includes(activeUrl) ? <PinOff size={16} className="shrink-0" /> : <Pin size={16} className="shrink-0" />}
        <span className="min-w-0 flex-1 truncate">{pins.includes(activeUrl) ? t('filebrowser.sidebar.unpin', 'Unpin current folder') : t('filebrowser.sidebar.pin', 'Pin current folder')}</span>
      </button>
    </div>
  )
  // Views — recent + AI topics, all read-only query results.
  const renderViews = (
    <div className="space-y-0.5">
      <SourceSection
        source={topics}
        empty={t('filebrowser.sidebar.topicsEmpty', 'No AI topics yet')}
      >
        {(list) => (
          <>
            {list.map((topic) =>
              navButton({
                key: topic.id,
                icon: <Hash size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
                label: topic.title,
                hint: `${topic.coverageCount}`,
                active: activeUrl === `view://topic/${topic.id}`,
                onClick: () => goto(`view://topic/${topic.id}`),
              }),
            )}
          </>
        )}
      </SourceSection>
      {isMockRuntime() ? (
        <div>
          <SectionHeader label={t('filebrowser.sidebar.diagnostics', 'Diagnostics (mock)')} />
          {(
            [
              ['view://demo/unknown-total', t('filebrowser.sidebar.unknownTotal')],
              ['view://demo/flaky', t('filebrowser.sidebar.flaky')],
              ['view://demo/error', t('filebrowser.sidebar.error')],
            ] as const
          ).map(([url, label]) =>
            navButton({
              key: url,
              icon: <FlaskConical size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
              label,
              active: activeUrl === url,
              onClick: () => goto(url),
            }),
          )}
        </div>
      ) : null}
    </div>
  )

  const renderCollections = (
    <div className="space-y-0.5">
      {collections.map((collection) =>
        navButton({
          key: collection.id,
          icon: <Library size={16} className="shrink-0 text-[color:var(--cp-muted)]" />,
          label: collection.title,
          hint: `${collection.refCount}`,
          active: activeUrl.startsWith(collectionUrl(collection.id)),
          onClick: () => goto(collectionUrl(collection.id)),
        }),
      )}
      <button
        type="button"
        onClick={onCreateCollection}
        className="fb-nav-row flex h-8 w-full items-center gap-2 rounded-[8px] px-2 text-left text-[13px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
      >
        <Plus size={16} className="shrink-0" />
        <span className="min-w-0 flex-1 truncate">
          {t('filebrowser.sidebar.newCollection', 'New collection…')}
        </span>
      </button>
    </div>
  )

  const renderDevices = hostInfo ? (
    hostUnlocked && hostRootUrl ? (
      <FolderTree
        key={hostRootUrl}
        roots={[{ name: hostInfo.label, url: hostRootUrl, defaultExpanded: true, icon: <Monitor size={16} className="shrink-0 text-[color:var(--cp-muted)]" /> }]}
        activeUrl={activeUrl}
        openedAt={openedAt}
        onNavigate={goto}
        loadChildren={loadHostChildren}
      />
    ) : (
      <button
        type="button"
        data-testid="sidebar-unlock-host"
        onClick={onUnlockHost}
        className="fb-nav-row flex h-8 w-full items-center gap-2 rounded-[8px] px-2 text-left text-[14px] text-[color:var(--cp-text)]"
        title={t('filebrowser.host.unlockHint', 'Confirm your password to see every folder on this computer')}
      >
        <Monitor size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
        <span className="min-w-0 flex-1 truncate">{hostInfo.label}</span>
        <Lock size={14} className="shrink-0 text-[color:var(--cp-muted)]" aria-label={t('filebrowser.host.locked', 'Locked')} />
      </button>
    )
  ) : null

  const dfsRoots = (roots: DfsNode[]): FolderTreeRoot[] =>
    roots.map((root) => ({
      name: root.name,
      url: normalizeUrl(root.path),
      icon: nodeIcon(root.kind),
      leaf: !normalizeUrl(root.path).startsWith('dfs://'),
    }))

  const bodies: Record<Section, React.ReactNode> = {
    dfs: (
      <SourceSection
        source={dfs}
        empty={t('filebrowser.sidebar.dfsEmpty', 'No accessible locations — check your access.')}
      >
        {(roots) => (
          <FolderTree
            roots={dfsRoots(roots)}
            activeUrl={activeUrl}
            openedAt={openedAt}
            onNavigate={goto}
            loadChildren={loadChildren}
          />
        )}
      </SourceSection>
    ),
    views: renderViews,
    collections: renderCollections,
    devices: renderDevices,
  }

  // Compact mode (mobile) — section pills + single active body
  if (compact) {
    return (
      <div className="flex h-full w-full flex-col gap-3" data-testid="files-places">
        <div className="flex gap-1 overflow-x-auto">
          {sections.map((section) => (
            <button
              key={section.id}
              type="button"
              aria-pressed={activeSection === section.id}
              onClick={() => setActiveSection(section.id)}
              className={clsx(
                'flex shrink-0 items-center gap-2 rounded-[8px] px-3 text-[14px]',
                activeSection === section.id
                  ? 'bg-[color:var(--files-pressed)] font-medium text-[color:var(--cp-text)]'
                  : 'text-[color:var(--cp-muted)]',
              )}
            >
              {section.icon}
              {section.label}
            </button>
          ))}
        </div>
        <div className="flex-1 overflow-y-auto pr-1">
          {activeSection === 'dfs' && favorites}
          {activeSection === 'dfs' && <SectionHeader label={t('filebrowser.sidebar.folders', 'Folders')} />}
          {activeSection ? bodies[activeSection] : null}
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden" data-testid="files-places">
      <div className="flex-1 space-y-0.5 overflow-y-auto pb-3 pr-1">
        {favorites}
        <SectionHeader label={t('filebrowser.sidebar.folders', 'Folders')} />
        {bodies.dfs}
        <button className="w-full rounded-[8px] text-left" onClick={() => toggleGroup('topics')} aria-expanded={groups.topics ?? true}><SectionHeader label={t('filebrowser.sidebar.views', 'AI Topics')} hint={`${topics.state.data?.length ?? '…'}`} collapsed={groups.topics === false} /></button>
        {(groups.topics ?? true) && renderViews}
        <button className="w-full rounded-[8px] text-left" onClick={() => toggleGroup('collections')} aria-expanded={groups.collections ?? true}><SectionHeader label={t('filebrowser.sidebar.collections', 'Collections')} hint={`${collections.length}`} collapsed={groups.collections === false} /></button>
        {(groups.collections ?? true) && renderCollections}
        {hostInfo ? (
          <>
            <SectionHeader
              label={t('filebrowser.sidebar.devices', 'Devices')}
              action={hostUnlocked ? <LockOpen size={14} className="text-[color:var(--cp-muted)]" aria-label={t('filebrowser.host.unlocked', 'Unlocked')} /> : undefined}
            />
            {renderDevices}
          </>
        ) : null}
      </div>
    </div>
  )
}
