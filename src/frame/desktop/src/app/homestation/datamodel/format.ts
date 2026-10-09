import type { AudienceSpec, ContactGroup, StatsViewKey } from './types'

export type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

export function formatCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
  if (count >= 1_000) return `${(count / 1_000).toFixed(1).replace(/\.0$/, '')}K`
  return String(count)
}

export function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

export function formatTimeAgo(t: Translate, timestamp: number, now: number): string {
  const minutes = Math.floor((now - timestamp) / 60_000)
  if (minutes < 1) return t('homestation.time.justNow', 'just now')
  if (minutes < 60) return t('homestation.time.minutes', '{{n}}m', { n: minutes })
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return t('homestation.time.hours', '{{n}}h', { n: hours })
  return t('homestation.time.days', '{{n}}d', { n: Math.floor(hours / 24) })
}

export function formatDateTime(timestamp: number, locale: string): string {
  return new Date(timestamp).toLocaleString(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function audienceLabel(t: Translate, spec: AudienceSpec, groups: ContactGroup[], nameOf: (did: string) => string): string {
  switch (spec.kind) {
    case 'public':
      return t('homestation.audience.public', 'Public')
    case 'followers':
      return t('homestation.audience.followers', 'Followers')
    case 'friends':
      return t('homestation.audience.friends', 'Friends')
    case 'group': {
      const group = groups.find(entry => entry.id === spec.groupId)
      return group ? t('homestation.audience.groupNamed', 'Group: {{name}}', { name: group.name }) : t('homestation.audience.group', 'Contact group')
    }
    case 'dids':
      return spec.dids.length === 1
        ? t('homestation.audience.onlyOne', 'Only {{name}}', { name: nameOf(spec.dids[0]) })
        : t('homestation.audience.people', '{{n}} people', { n: spec.dids.length })
  }
}

export function restrictedLabel(t: Translate, spec: AudienceSpec): string {
  switch (spec.kind) {
    case 'followers':
      return t('homestation.audience.followersOnly', 'Followers only')
    case 'friends':
      return t('homestation.audience.friendsOnly', 'Friends only')
    case 'group':
      return t('homestation.audience.groupOnly', 'Group only')
    case 'dids':
      return t('homestation.audience.specificPeople', 'Specific people')
    default:
      return t('homestation.audience.public', 'Public')
  }
}

export function viewLabel(t: Translate, view: StatsViewKey, collectorName: string): string {
  if (view === 'local') return t('homestation.views.local', 'Local merged')
  if (view === 'author') return t('homestation.views.author', 'Author view')
  return t('homestation.views.collector', 'Collector: {{name}}', { name: collectorName })
}

export function shortObjId(objId: string) {
  const [prefix, hash = ''] = objId.split(':')
  return `${prefix}:${hash.slice(0, 6)}…${hash.slice(-4)}`
}

const AUDIENCE_RANK: Record<AudienceSpec['kind'], number> = { public: 4, followers: 3, friends: 2, group: 1, dids: 0 }

export function isNarrowerAudience(next: AudienceSpec, previous: AudienceSpec) {
  if (AUDIENCE_RANK[next.kind] !== AUDIENCE_RANK[previous.kind]) return AUDIENCE_RANK[next.kind] < AUDIENCE_RANK[previous.kind]
  if (next.kind === 'group' && previous.kind === 'group') return next.groupId !== previous.groupId
  if (next.kind === 'dids' && previous.kind === 'dids') return previous.dids.some(did => !next.dids.includes(did))
  return false
}
