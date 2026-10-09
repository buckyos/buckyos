import { useI18n } from '../../../i18n/provider'
import type { CardView } from '../datamodel/types'

export function ReasonDetail({ view }: { view: CardView }) {
  const { t } = useI18n()
  const reading = view.reading
  if (!reading) return <p className="text-sm">{t('homestation.reason.none', 'This item is not in your reading list.')}</p>
  const refKind = (kind: string) => ({ source: t('homestation.reason.kindSource', 'Source'), topic: t('homestation.reason.kindTopic', 'Topic'), rule: t('homestation.reason.kindRule', 'Rule'), intent: t('homestation.reason.kindIntent', 'Subscription intent'), collector: t('homestation.reason.kindCollector', 'Collector') })[kind] ?? kind
  return (
    <div className="space-y-2 text-xs leading-5">
      <ul className="space-y-1">
        {reading.reason.refs.map(ref => (
          <li key={`${ref.kind}-${ref.id}`} className="flex gap-2">
            <span className="w-28 flex-shrink-0" style={{ color: 'var(--cp-muted)' }}>{refKind(ref.kind)}</span>
            <span>{ref.label}</span>
          </li>
        ))}
        {reading.topics.length ? (
          <li className="flex gap-2">
            <span className="w-28 flex-shrink-0" style={{ color: 'var(--cp-muted)' }}>{t('homestation.reason.matchedTopics', 'Matched topics')}</span>
            <span>{reading.topics.map(topic => topic.replace('topic-', '')).join(', ')}</span>
          </li>
        ) : null}
        {reading.filteredBy.length ? (
          <li className="flex gap-2">
            <span className="w-28 flex-shrink-0" style={{ color: 'var(--cp-muted)' }}>{t('homestation.reason.kindRule', 'Rule')}</span>
            <span>{t('homestation.reason.filteredBy', 'Hidden by {{n}} of your filter rules; shown because you asked to see filtered items.', { n: reading.filteredBy.length })}</span>
          </li>
        ) : null}
      </ul>
      <p style={{ color: 'var(--cp-muted)' }}>{t('homestation.reason.serviceNote', 'Explanation from your recommender: {{text}}', { text: reading.reason.text })}</p>
      <p style={{ color: 'var(--cp-muted)' }}>{t('homestation.reason.privateNote', 'Reasons are private to you and never shown to visitors or the author.')}</p>
    </div>
  )
}
