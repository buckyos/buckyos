import { useState } from 'react'
import {
  Link,
  MessageSquare,
  Plus,
  Rss,
  Search,
  UserPlus,
} from 'lucide-react'
import type { Source } from '../types'

interface SourceManagerProps {
  sources: Source[]
  t: (key: string, fallback: string) => string
}

const sourceTypeColors: Record<string, string> = {
  person: 'var(--cp-accent)',
  channel: 'var(--cp-warning)',
  rss: 'var(--cp-success)',
  website: 'var(--cp-muted)',
  topic: 'var(--cp-accent-soft)',
  'agent-curated': 'var(--cp-danger)',
}

type AddMode = 'follow' | 'url' | 'natural' | null

export function SourceManager({
  sources,
  t,
}: SourceManagerProps) {
  const [addMode, setAddMode] = useState<AddMode>(null)
  const [inputValue, setInputValue] = useState('')

  const followingSources = sources.filter((s) => s.isFollowing)
  const suggestedSources = sources.filter((s) => !s.isFollowing)

  return (
    <div className="p-4">
      <h2 className="mb-4 text-lg font-bold" style={{ color: 'var(--cp-text)' }}>
        {t('homestation.sources', 'Sources')}
      </h2>

      {/* Add source buttons */}
      <div className="mb-4 flex flex-col gap-2">
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => setAddMode(addMode === 'follow' ? null : 'follow')}
            className="flex flex-1 items-center justify-center gap-1.5 rounded-xl py-2.5 text-xs font-medium transition-colors"
            style={{
              background: addMode === 'follow' ? 'color-mix(in srgb, var(--cp-accent) 15%, transparent)' : 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
              color: addMode === 'follow' ? 'var(--cp-accent)' : 'var(--cp-text)',
            }}
          >
            <UserPlus size={14} />
            {t('homestation.follow', 'Follow')}
          </button>
          <button
            type="button"
            onClick={() => setAddMode(addMode === 'url' ? null : 'url')}
            className="flex flex-1 items-center justify-center gap-1.5 rounded-xl py-2.5 text-xs font-medium transition-colors"
            style={{
              background: addMode === 'url' ? 'color-mix(in srgb, var(--cp-accent) 15%, transparent)' : 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
              color: addMode === 'url' ? 'var(--cp-accent)' : 'var(--cp-text)',
            }}
          >
            <Link size={14} />
            {t('homestation.pasteUrl', 'Paste URL')}
          </button>
          <button
            type="button"
            onClick={() => setAddMode(addMode === 'natural' ? null : 'natural')}
            className="flex flex-1 items-center justify-center gap-1.5 rounded-xl py-2.5 text-xs font-medium transition-colors"
            style={{
              background: addMode === 'natural' ? 'color-mix(in srgb, var(--cp-accent) 15%, transparent)' : 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
              color: addMode === 'natural' ? 'var(--cp-accent)' : 'var(--cp-text)',
            }}
          >
            <MessageSquare size={14} />
            {t('homestation.describe', 'Describe')}
          </button>
        </div>

        {/* Input for selected mode */}
        {addMode ? (
          <div
            className="flex items-center gap-2 rounded-xl px-3 py-2"
            style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}
          >
            <Search size={14} style={{ color: 'var(--cp-muted)' }} />
            <input
              type="text"
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              placeholder={
                addMode === 'follow' ? t('homestation.followPlaceholder', 'Enter a name or DID...')
                  : addMode === 'url' ? t('homestation.urlPlaceholder', 'Paste a URL, RSS feed, or account link...')
                    : t('homestation.naturalPlaceholder', 'Describe what you want to see...')
              }
              className="flex-1 bg-transparent text-sm outline-none"
              style={{ color: 'var(--cp-text)' }}
              autoFocus
            />
            <button
              type="button"
              disabled={!inputValue.trim()}
              className="flex h-7 w-7 items-center justify-center rounded-lg disabled:opacity-40"
              style={{ background: 'var(--cp-accent)', color: 'white' }}
            >
              <Plus size={14} />
            </button>
          </div>
        ) : null}
      </div>

      {/* Following sources */}
      <div className="mb-4">
        <p className="mb-2 text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.followingSources', 'Following')} ({followingSources.length})
        </p>
        <div className="flex flex-col gap-1">
          {followingSources.map((source) => (
            <SourceRow key={source.id} source={source} t={t} />
          ))}
        </div>
      </div>

      {/* Suggested sources */}
      {suggestedSources.length > 0 ? (
        <div>
          <p className="mb-2 text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            {t('homestation.suggestedSources', 'Suggested')}
          </p>
          <div className="flex flex-col gap-1">
            {suggestedSources.map((source) => (
              <SourceRow key={source.id} source={source} t={t} showFollowButton />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  )
}

function SourceRow({
  source,
  t,
  showFollowButton,
}: {
  source: Source
  t: (key: string, fallback: string) => string
  showFollowButton?: boolean
}) {
  return (
    <div
      className="flex items-center gap-3 rounded-xl px-3 py-2 transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_4%,transparent)]"
    >
      <div
        className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full"
        style={{
          background: `color-mix(in srgb, ${sourceTypeColors[source.type] ?? 'var(--cp-muted)'} 15%, transparent)`,
          color: sourceTypeColors[source.type] ?? 'var(--cp-muted)',
        }}
      >
        <Rss size={16} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
          {source.name}
        </p>
        <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {source.description ?? source.type}
        </p>
      </div>
      <div className="flex items-center gap-2">
        <span
          className="rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase"
          style={{
            background: `color-mix(in srgb, ${sourceTypeColors[source.type] ?? 'var(--cp-muted)'} 12%, transparent)`,
            color: sourceTypeColors[source.type] ?? 'var(--cp-muted)',
          }}
        >
          {source.type}
        </span>
        {showFollowButton ? (
          <button
            type="button"
            className="rounded-lg px-2.5 py-1 text-[11px] font-semibold"
            style={{ background: 'var(--cp-accent)', color: 'white' }}
          >
            {t('homestation.follow', 'Follow')}
          </button>
        ) : null}
      </div>
    </div>
  )
}
