/* ── Request banner and entity relation section: “Add friend / Allow messages / Block” through RelationActions ── */

import { ExternalLink } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { RelationActions } from '../../components/network/RelationActions'
import { RelationBadge } from '../../components/network/RelationBadge'
import type { RelationActionId } from '../../components/network/relation'
import { openMyNetwork } from '../my-network/launch'
import { hasRelation, needsAdmission, useEntityRelation } from './relation'
import { useMessageHubStore } from './store'
import type { Entity, MessageHubContext } from './types'

const BANNER_EXCLUDED: RelationActionId[] = ['remove_contact', 'unfriend', 'remove_from_collection', 'cancel_request']

export function RequestBanner({ entity, context, requestCount }: { entity: Entity | null; context: MessageHubContext; requestCount: number }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const relation = useEntityRelation(hasRelation(context, entity) ? entity!.id : null)
  if (!entity || requestCount <= 0 || !relation || !needsAdmission(relation)) return null
  return (
    <div role="note" data-testid="request-banner" className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_12%,var(--cp-surface))] px-3 py-1.5 text-[13px]">
      <span className="min-w-0 flex-1 truncate font-medium">{t('messagehub.requestPending', undefined, { count: requestCount })}</span>
      <RelationBadge relation={relation} compact />
      <RelationActions
        person={{ did: entity.id, name: entity.name, avatar: entity.avatar, kind: entity.type === 'agent' ? 'agent' : 'user' }}
        relation={relation}
        exclude={BANNER_EXCLUDED}
        onDone={() => { void store.ensureOwner(context, true) }}
      />
    </div>
  )
}

export function RelationSection({ entity, context }: { entity: Entity; context: MessageHubContext }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const relation = useEntityRelation(hasRelation(context, entity) ? entity.id : null)
  if (!hasRelation(context, entity) || !relation) return null
  return (
    <div className="mb-4 space-y-2 rounded-xl p-3" style={{ background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)' }} data-testid="entity-relation">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.relation')}</p>
        <button type="button" className="inline-flex min-h-8 items-center gap-1 rounded-lg px-2 text-xs text-[color:var(--cp-accent)]" onClick={() => openMyNetwork({ entity: entity.id })} data-testid="entity-open-my-network">
          <ExternalLink size={12} aria-hidden />{t('messagehub.openInMyNetwork')}
        </button>
      </div>
      <RelationBadge relation={relation} />
      <RelationActions
        person={{ did: entity.id, name: entity.name, avatar: entity.avatar, kind: entity.type === 'agent' ? 'agent' : 'user' }}
        relation={relation}
        exclude={['remove_from_collection']}
        onDone={() => { void store.ensureOwner(context, true) }}
      />
    </div>
  )
}
