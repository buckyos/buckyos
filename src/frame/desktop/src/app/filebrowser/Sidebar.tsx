import clsx from 'clsx'
import {
  ChevronDown,
  ChevronRight,
  Clock,
  Cpu,
  FolderClosed,
  FolderOpen,
  Globe,
  Hash,
  HardDrive,
  Library,
  Lock,
  Plus,
  Share2,
  Sparkles,
} from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { CollectionSummary } from './mock/collections'
import { collectionUrl, normalizeUrl } from './data/urls'
import type { DeviceNode, DfsNode, Topic } from './types'

type Section = 'dfs' | 'devices' | 'views' | 'collections'

interface SidebarProps {
  dfsRoots: DfsNode[]
  devices: DeviceNode[]
  topics: Topic[]
  collections: CollectionSummary[]
  /** Canonical url of the active pane location. */
  activeUrl: string
  advancedMode: boolean
  onToggleAdvanced: (value: boolean) => void
  /** Accepts bare paths and canonical urls alike (normalized by the pane). */
  onNavigate: (url: string) => void
  onCreateCollection: () => void
  compact?: boolean
  onAfterNavigate?: () => void
}

function nodeIcon(kind: DfsNode['kind']) {
  switch (kind) {
    case 'public':
      return <Globe size={14} className="text-[color:var(--cp-success)]" />
    case 'shared':
      return <Share2 size={14} className="text-[color:var(--cp-accent)]" />
    case 'privacy':
      return <Lock size={14} className="text-[color:var(--cp-warning)]" />
    case 'home':
      return <FolderOpen size={14} className="text-[color:var(--cp-muted)]" />
    default:
      return <FolderClosed size={14} className="text-[color:var(--cp-muted)]" />
  }
}

function DfsTreeNode({
  node,
  activeUrl,
  onNavigate,
  depth,
}: {
  node: DfsNode
  activeUrl: string
  onNavigate: (path: string) => void
  depth: number
}) {
  const hasChildren = !!node.children?.length
  const nodeUrl = normalizeUrl(node.path)
  const [expanded, setExpanded] = useState(depth < 1 || activeUrl.startsWith(nodeUrl))
  const active = activeUrl === nodeUrl

  return (
    <div>
      <button
        type="button"
        className={clsx(
          'group flex w-full items-center gap-2 rounded-[14px] px-2 py-1.5 text-left text-sm transition',
          active
            ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-text)]'
            : 'text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]',
        )}
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        onClick={() => {
          if (hasChildren) setExpanded((v) => !v)
          onNavigate(node.path)
        }}
      >
        {hasChildren ? (
          expanded ? (
            <ChevronDown size={14} className="shrink-0" />
          ) : (
            <ChevronRight size={14} className="shrink-0" />
          )
        ) : (
          <span className="w-[14px] shrink-0" />
        )}
        {nodeIcon(node.kind)}
        <span className="min-w-0 truncate font-medium">{node.name}</span>
      </button>
      {hasChildren && expanded ? (
        <div className="space-y-0.5">
          {node.children!.map((child) => (
            <DfsTreeNode
              key={child.id}
              node={child}
              activeUrl={activeUrl}
              onNavigate={onNavigate}
              depth={depth + 1}
            />
          ))}
        </div>
      ) : null}
    </div>
  )
}

function SectionHeader({
  icon,
  label,
  hint,
  action,
}: {
  icon: React.ReactNode
  label: string
  hint?: string
  action?: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-2 px-2 pb-1.5 pt-3">
      <span className="text-[color:var(--cp-muted)]">{icon}</span>
      <span className="shell-kicker !text-[10px]">{label}</span>
      {hint ? (
        <span className="ml-auto text-[10px] text-[color:var(--cp-muted)]">{hint}</span>
      ) : null}
      {action ? <span className={hint ? '' : 'ml-auto'}>{action}</span> : null}
    </div>
  )
}

function navButton({
  key,
  icon,
  label,
  hint,
  active,
  onClick,
}: {
  key: string
  icon: React.ReactNode
  label: string
  hint?: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      key={key}
      type="button"
      onClick={onClick}
      className={clsx(
        'flex w-full items-center gap-2 rounded-[10px] px-2 py-1.5 text-left text-sm transition',
        active
          ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-text)]'
          : 'text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]',
      )}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
      {hint ? (
        <span className="shrink-0 text-[11px] text-[color:var(--cp-muted)]">{hint}</span>
      ) : null}
    </button>
  )
}

export function Sidebar({
  dfsRoots,
  devices,
  topics,
  collections,
  activeUrl,
  advancedMode,
  onToggleAdvanced,
  onNavigate,
  onCreateCollection,
  compact = false,
  onAfterNavigate,
}: SidebarProps) {
  const { t } = useI18n()
  const [activeSection, setActiveSection] = useState<Section | null>(compact ? 'dfs' : null)

  const sections: { id: Section; label: string; icon: React.ReactNode }[] = [
    { id: 'dfs', label: t('filebrowser.sidebar.dfs', 'DFS'), icon: <FolderOpen size={14} /> },
    { id: 'views', label: t('filebrowser.sidebar.views', 'Views'), icon: <Sparkles size={14} /> },
    {
      id: 'collections',
      label: t('filebrowser.sidebar.collections', 'Collections'),
      icon: <Library size={14} />,
    },
    { id: 'devices', label: t('filebrowser.sidebar.devices', 'Devices'), icon: <Cpu size={14} /> },
  ]

  const goto = (url: string) => {
    onNavigate(url)
    onAfterNavigate?.()
  }

  // Views — recent + AI topics, all read-only query results.
  const renderViews = (
    <div className="space-y-0.5">
      {navButton({
        key: 'view-recent',
        icon: <Clock size={12} className="shrink-0 text-[color:var(--cp-accent)]" />,
        label: t('filebrowser.sidebar.recent', 'Recent'),
        active: activeUrl === 'view://recent',
        onClick: () => goto('view://recent'),
      })}
      {topics.map((topic) =>
        navButton({
          key: topic.id,
          icon: <Hash size={12} className="shrink-0 text-[color:var(--cp-accent)]" />,
          label: topic.title,
          hint: `${topic.coverageCount}`,
          active: activeUrl === `view://topic/${topic.id}`,
          onClick: () => goto(`view://topic/${topic.id}`),
        }),
      )}
    </div>
  )

  const renderCollections = (
    <div className="space-y-0.5">
      {collections.map((collection) =>
        navButton({
          key: collection.id,
          icon: <Library size={12} className="shrink-0 text-[color:var(--cp-accent)]" />,
          label: collection.title,
          hint: `${collection.refCount}`,
          active: activeUrl.startsWith(collectionUrl(collection.id)),
          onClick: () => goto(collectionUrl(collection.id)),
        }),
      )}
      <button
        type="button"
        onClick={onCreateCollection}
        className="flex w-full items-center gap-2 rounded-[10px] px-2 py-1.5 text-left text-sm text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]"
      >
        <Plus size={12} className="shrink-0" />
        <span className="min-w-0 flex-1 truncate">
          {t('filebrowser.sidebar.newCollection', 'New collection…')}
        </span>
      </button>
    </div>
  )

  const renderDevices = (
    <div className="space-y-2">
      {!advancedMode ? (
        <div className="rounded-[16px] border border-dashed border-[color:var(--cp-border)] px-3 py-3 text-[11px] leading-5 text-[color:var(--cp-muted)]">
          {t(
            'filebrowser.sidebar.devices.hint',
            'Device view is only available to advanced users. Enable advanced mode to browse raw device paths.',
          )}
          <button
            type="button"
            onClick={() => onToggleAdvanced(true)}
            className="mt-2 inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-[11px] font-semibold text-[color:var(--cp-accent)] hover:border-[color:var(--cp-accent)]"
          >
            {t('filebrowser.sidebar.devices.enable', 'Enable advanced mode')}
          </button>
        </div>
      ) : (
        devices.map((device) => (
          <div
            key={device.id}
            className="rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_80%,transparent)] px-2.5 py-2"
          >
            <div className="flex items-center gap-2 text-sm font-semibold text-[color:var(--cp-text)]">
              <HardDrive size={14} className="text-[color:var(--cp-muted)]" />
              <span className="truncate">{device.name}</span>
              <span
                className={clsx(
                  'ml-auto text-[10px] font-semibold uppercase tracking-wider',
                  device.status === 'online'
                    ? 'text-[color:var(--cp-success)]'
                    : device.status === 'syncing'
                      ? 'text-[color:var(--cp-warning)]'
                      : 'text-[color:var(--cp-muted)]',
                )}
              >
                {device.status}
              </span>
            </div>
            <div className="mt-1 text-[10px] text-[color:var(--cp-muted)]">{device.host}</div>
            <div className="mt-1.5 space-y-0.5">
              {device.roots.map((root) => (
                <button
                  key={root.path}
                  type="button"
                  className="block w-full truncate rounded-[10px] px-2 py-1 text-left font-mono text-[11px] text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]"
                  onClick={() => goto(`${device.name}:${root.path}`)}
                >
                  {root.label}
                </button>
              ))}
            </div>
          </div>
        ))
      )}
    </div>
  )

  const bodies: Record<Section, React.ReactNode> = {
    dfs: (
      <div className="space-y-0.5">
        {dfsRoots.map((root) => (
          <DfsTreeNode
            key={root.id}
            node={root}
            activeUrl={activeUrl}
            onNavigate={goto}
            depth={0}
          />
        ))}
      </div>
    ),
    views: renderViews,
    collections: renderCollections,
    devices: renderDevices,
  }

  // Compact mode (mobile) — section pills + single active body
  if (compact) {
    return (
      <div className="flex h-full w-full flex-col gap-3">
        <div className="flex gap-1 overflow-x-auto pb-1">
          {sections.map((section) => (
            <button
              key={section.id}
              type="button"
              onClick={() => setActiveSection(section.id)}
              className={clsx(
                'flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-semibold',
                activeSection === section.id
                  ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] text-[color:var(--cp-text)]'
                  : 'border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)] text-[color:var(--cp-muted)]',
              )}
            >
              {section.icon}
              {section.label}
            </button>
          ))}
        </div>
        <div className="flex-1 overflow-y-auto pr-1">
          {activeSection ? bodies[activeSection] : null}
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <div className="flex-1 space-y-1 overflow-y-auto pr-1">
        <SectionHeader
          icon={<Sparkles size={13} className="text-[color:var(--cp-accent)]" />}
          label={t('filebrowser.sidebar.views', 'AI Topics')}
          hint={`${topics.length}`}
        />
        {renderViews}

        <SectionHeader
          icon={<Library size={13} className="text-[color:var(--cp-accent)]" />}
          label={t('filebrowser.sidebar.collections', 'Collections')}
          hint={`${collections.length}`}
        />
        {renderCollections}

        <SectionHeader
          icon={<FolderOpen size={13} />}
          label={t('filebrowser.sidebar.dfs', 'DFS · Logical view')}
        />
        {bodies.dfs}

        <SectionHeader
          icon={<Cpu size={13} />}
          label={t('filebrowser.sidebar.devices', 'Devices')}
          hint={advancedMode ? t('filebrowser.sidebar.advanced', 'Advanced') : ''}
        />
        {renderDevices}
      </div>
    </div>
  )
}
