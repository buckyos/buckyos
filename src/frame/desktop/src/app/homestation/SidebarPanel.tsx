import {
  Bookmark,
  Clock,
  Hash,
  Rss,
  User,
} from 'lucide-react'
import type { Source, Topic, UserProfile } from './types'

interface SidebarPanelProps {
  profile: UserProfile
  topics: Topic[]
  sources: Source[]
  activeTopicId: string | null
  t: (key: string, fallback: string) => string
  onSelectTopic: (id: string | null) => void
  onViewProfile: () => void
  headerActions?: React.ReactNode
}

function SourceTypeBadge({ type }: { type: string }) {
  const colors: Record<string, string> = {
    person: 'var(--cp-accent)',
    channel: 'var(--cp-warning)',
    rss: 'var(--cp-success)',
    website: 'var(--cp-muted)',
    topic: 'var(--cp-accent-soft)',
    'agent-curated': 'var(--cp-danger)',
  }

  return (
    <span
      className="rounded px-1 py-0.5 text-[9px] font-semibold uppercase"
      style={{
        background: `color-mix(in srgb, ${colors[type] ?? 'var(--cp-muted)'} 15%, transparent)`,
        color: colors[type] ?? 'var(--cp-muted)',
      }}
    >
      {type}
    </span>
  )
}

export function SidebarPanel({
  profile,
  topics,
  sources,
  activeTopicId,
  t,
  onSelectTopic,
  onViewProfile,
  headerActions,
}: SidebarPanelProps) {
  const followingSources = sources.filter((s) => s.isFollowing)

  return (
    <div
      className="desktop-scrollbar flex h-full flex-col overflow-y-auto"
      style={{
        background:
          'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
      }}
    >
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <button
          type="button"
          onClick={onViewProfile}
          className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full text-sm font-semibold"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)',
            color: 'var(--cp-accent)',
          }}
        >
          {profile.name.charAt(0)}
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{profile.name}</p>
          <p className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
            {profile.followerCount} {t('homestation.followers', 'followers')}
          </p>
        </div>
        {headerActions}
      </div>

      {/* Quick actions */}
      <div className="flex flex-col gap-0.5 px-2 py-2">
        <button
          type="button"
          onClick={onViewProfile}
          className="flex items-center gap-3 rounded-xl px-3 py-2 text-sm transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
          style={{ color: 'var(--cp-text)' }}
        >
          <User size={16} style={{ color: 'var(--cp-muted)' }} />
          {t('homestation.myProfile', 'My Profile')}
        </button>
        <button
          type="button"
          className="flex items-center gap-3 rounded-xl px-3 py-2 text-sm transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
          style={{ color: 'var(--cp-text)' }}
        >
          <Bookmark size={16} style={{ color: 'var(--cp-muted)' }} />
          {t('homestation.bookmarks', 'Bookmarks')}
        </button>
        <button
          type="button"
          className="flex items-center gap-3 rounded-xl px-3 py-2 text-sm transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
          style={{ color: 'var(--cp-text)' }}
        >
          <Clock size={16} style={{ color: 'var(--cp-muted)' }} />
          {t('homestation.readLater', 'Read Later')}
        </button>
      </div>

      {/* Topics section */}
      <div className="px-2 pt-2">
        <p className="px-3 pb-1 text-[11px] font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.topics', 'Topics')}
        </p>
        {topics.map((topic) => (
          <button
            key={topic.id}
            type="button"
            onClick={() => onSelectTopic(activeTopicId === topic.id ? null : topic.id)}
            className="flex w-full items-center gap-2 rounded-xl px-3 py-1.5 text-sm transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
            style={{
              color: 'var(--cp-text)',
              background: activeTopicId === topic.id
                ? 'color-mix(in srgb, var(--cp-accent) 10%, transparent)'
                : 'transparent',
            }}
          >
            <Hash size={14} style={{ color: activeTopicId === topic.id ? 'var(--cp-accent)' : 'var(--cp-muted)' }} />
            <span className="flex-1 truncate text-left">{topic.name}</span>
            <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>{topic.feedCount}</span>
          </button>
        ))}
      </div>

      {/* Sources section */}
      <div className="px-2 pt-4">
        <p className="px-3 pb-1 text-[11px] font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.sources', 'Sources')} ({followingSources.length})
        </p>
        {followingSources.slice(0, 6).map((source) => (
          <div
            key={source.id}
            className="flex items-center gap-2 rounded-xl px-3 py-1.5"
          >
            <Rss size={12} style={{ color: 'var(--cp-muted)' }} />
            <span className="flex-1 truncate text-xs" style={{ color: 'var(--cp-text)' }}>
              {source.name}
            </span>
            <SourceTypeBadge type={source.type} />
          </div>
        ))}
        {followingSources.length > 6 ? (
          <p className="px-3 py-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
            +{followingSources.length - 6} {t('homestation.more', 'more')}
          </p>
        ) : null}
      </div>
    </div>
  )
}
