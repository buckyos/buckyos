import type { Translate } from '../datamodel/format'
import type { EffectiveTag, FeedItemView, ReaderIdentity, ReadingEntry } from '../datamodel/types'
import type { PreviewReaderKey } from '../store/context'

export const GENERATION_TAGS = new Set(['ai_full', 'ai_assisted', 'low_quality'])

export function previewReaderLabel(t: Translate, key: PreviewReaderKey, reader: ReaderIdentity | undefined, nameOf: (did: string) => string): string {
  const name = reader?.kind === 'did' ? nameOf(reader.did) : ''
  if (key === 'follower') return t('homestation.profile.asFollower', 'Follower ({{name}})', { name })
  if (key === 'friend') return t('homestation.profile.asFriend', 'Friend ({{name}})', { name })
  return t('homestation.profile.asAnonymous', 'Anonymous')
}

export function actionText(t: Translate, item: FeedItemView): string | null {
  const type = item.object.comment_type
  if (type === 'repost') return t('homestation.action.reposted', 'reposted')
  if (type === 'quote') return t('homestation.action.quoted', 'quoted')
  if (type === 'text') return t('homestation.action.commented', 'commented')
  if (type === 'like') return t('homestation.action.liked', 'liked')
  if (type === 'bookmark') return t('homestation.action.bookmarked', 'bookmarked publicly')
  if (item.isCapture && !item.isPrivateCapture) return t('homestation.action.shared', 'shared')
  return null
}

export function tagLabel(t: Translate, tag: EffectiveTag): string {
  const inferred = tag.status === 'inferred'
  switch (tag.tag) {
    case 'ai_full':
      return inferred ? t('homestation.tags.aiFullInferred', 'Possibly AI-generated · local inference') : t('homestation.tags.aiFull', 'AI-generated')
    case 'ai_assisted':
      return inferred ? t('homestation.tags.aiAssistedInferred', 'Possibly AI-assisted · local inference') : t('homestation.tags.aiAssisted', 'AI-assisted')
    case 'low_quality':
      return inferred ? t('homestation.tags.lowQualityInferred', 'Possibly low quality · local inference') : t('homestation.tags.lowQuality', 'Low quality')
    default:
      return `#${tag.label}`
  }
}

export function reasonLabel(t: Translate, reading: ReadingEntry): string {
  const ref = reading.reason.refs.find(entry => entry.kind !== 'intent') ?? reading.reason.refs[0]
  const intent = reading.reason.refs.find(entry => entry.kind === 'intent')
  const name = ref?.label ?? ''
  switch (reading.reason.code) {
    case 'followed':
      return t('homestation.reason.followed', 'Because you follow {{name}}', { name })
    case 'friend':
      return t('homestation.reason.friend', 'From your friend {{name}}', { name })
    case 'subscription':
      return intent ? t('homestation.reason.intent', 'Matches your subscription “{{name}}”', { name: intent.label }) : t('homestation.reason.subscription', 'From your subscription {{name}}', { name })
    case 'collector':
      return t('homestation.reason.collector', 'Recommended by collector {{name}}', { name })
    case 'topic':
      return t('homestation.reason.topic', 'Matches your topic {{name}}', { name })
    case 'rule':
      return t('homestation.reason.rule', 'Shown by your rule {{name}}', { name })
  }
}
