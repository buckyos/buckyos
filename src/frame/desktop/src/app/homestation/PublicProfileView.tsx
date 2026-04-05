import { useState } from 'react'
import { ShieldCheck } from 'lucide-react'
import { FeedCard } from './FeedCard'
import type { FeedObject, ProfileTab, UserProfile } from './types'

interface PublicProfileViewProps {
  profile: UserProfile
  feeds: FeedObject[]
  t: (key: string, fallback: string) => string
  onSelectFeed: (id: string) => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
  onRepost: (id: string) => void
}

const profileTabs: { id: ProfileTab; labelKey: string; fallback: string }[] = [
  { id: 'posts', labelKey: 'homestation.tabPosts', fallback: 'Posts' },
  { id: 'works', labelKey: 'homestation.tabWorks', fallback: 'Works' },
  { id: 'products', labelKey: 'homestation.tabProducts', fallback: 'Products' },
  { id: 'featured', labelKey: 'homestation.tabFeatured', fallback: 'Featured' },
]

export function PublicProfileView({
  profile,
  feeds,
  t,
  onSelectFeed,
  onToggleLike,
  onToggleBookmark,
  onRepost,
}: PublicProfileViewProps) {
  const [activeTab, setActiveTab] = useState<ProfileTab>('posts')

  return (
    <div>
      {/* Cover area */}
      <div
        className="relative h-32 w-full md:h-48"
        style={{ background: 'linear-gradient(135deg, color-mix(in srgb, var(--cp-accent) 30%, var(--cp-surface)), color-mix(in srgb, var(--cp-accent-soft) 40%, var(--cp-surface-2)))' }}
      />

      {/* Profile info */}
      <div className="relative px-4 pb-4">
        {/* Avatar */}
        <div
          className="-mt-10 flex h-20 w-20 items-center justify-center rounded-full border-4 text-2xl font-bold md:-mt-12 md:h-24 md:w-24"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 20%, var(--cp-surface))',
            borderColor: 'var(--cp-bg)',
            color: 'var(--cp-accent)',
          }}
        >
          {profile.name.charAt(0)}
        </div>

        <div className="mt-3">
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold" style={{ color: 'var(--cp-text)' }}>{profile.name}</h2>
            <ShieldCheck size={18} style={{ color: 'var(--cp-success)' }} />
          </div>
          {profile.bio ? (
            <p className="mt-1 text-sm" style={{ color: 'color-mix(in srgb, var(--cp-text) 75%, transparent)' }}>
              {profile.bio}
            </p>
          ) : null}
          <div className="mt-2 flex items-center gap-4 text-sm" style={{ color: 'var(--cp-muted)' }}>
            <span><strong style={{ color: 'var(--cp-text)' }}>{profile.postCount}</strong> {t('homestation.posts', 'posts')}</span>
            <span><strong style={{ color: 'var(--cp-text)' }}>{profile.followerCount}</strong> {t('homestation.followers', 'followers')}</span>
            <span><strong style={{ color: 'var(--cp-text)' }}>{profile.followingCount}</strong> {t('homestation.following', 'following')}</span>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div
        className="flex gap-0.5 px-4"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        {profileTabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            onClick={() => setActiveTab(tab.id)}
            className="relative px-4 py-2.5 text-sm font-medium transition-colors"
            style={{
              color: activeTab === tab.id ? 'var(--cp-accent)' : 'var(--cp-muted)',
            }}
          >
            {t(tab.labelKey, tab.fallback)}
            {activeTab === tab.id ? (
              <span
                className="absolute bottom-0 left-0 right-0 h-0.5 rounded-full"
                style={{ background: 'var(--cp-accent)' }}
              />
            ) : null}
          </button>
        ))}
      </div>

      {/* Content */}
      {activeTab === 'posts' ? (
        feeds.length > 0 ? (
          feeds.map((feed) => (
            <FeedCard
              key={feed.id}
              feed={feed}
              readingMode="standard"
              t={t}
              onSelect={onSelectFeed}
              onToggleLike={onToggleLike}
              onToggleBookmark={onToggleBookmark}
              onRepost={onRepost}
            />
          ))
        ) : (
          <div className="flex h-40 items-center justify-center">
            <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
              {t('homestation.noPosts', 'No posts yet')}
            </p>
          </div>
        )
      ) : (
        <div className="flex h-40 items-center justify-center">
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('homestation.comingSoon', 'Coming soon')}
          </p>
        </div>
      )}
    </div>
  )
}
