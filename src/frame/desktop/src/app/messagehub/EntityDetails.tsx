import {
  X,
  Bot,
  Users,
  User,
  Bell,
  BellOff,
  Pin,
  PinOff,
  Tag,
  Link2,
  Edit3,
  Trash2,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { EntityDetail } from './types'

interface EntityDetailsProps {
  entity: EntityDetail
  onClose: () => void
}

function DetailAvatar({ entity }: { entity: EntityDetail }) {
  const colors: Record<string, string> = {
    person: 'var(--cp-accent)',
    agent: 'var(--cp-success)',
    group: 'var(--cp-warning)',
    service: 'var(--cp-danger)',
  }
  const icons: Record<string, React.ReactNode> = {
    person: <User size={32} />,
    agent: <Bot size={32} />,
    group: <Users size={32} />,
    service: <Tag size={32} />,
  }

  return (
    <div
      className="flex items-center justify-center rounded-full mx-auto"
      style={{
        width: 80,
        height: 80,
        background: `color-mix(in srgb, ${colors[entity.type]} 18%, transparent)`,
        color: colors[entity.type],
      }}
    >
      {icons[entity.type]}
    </div>
  )
}

function InfoRow({
  label,
  value,
  icon,
}: {
  label: string
  value: string
  icon?: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-3 py-2.5">
      {icon && (
        <span style={{ color: 'var(--cp-muted)' }}>{icon}</span>
      )}
      <div className="min-w-0 flex-1">
        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {label}
        </p>
        <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
          {value}
        </p>
      </div>
    </div>
  )
}

export function EntityDetails({ entity, onClose }: EntityDetailsProps) {
  const { t } = useI18n()

  const typeLabels: Record<string, string> = {
    person: t('messagehub.entityType.person', 'Person'),
    agent: t('messagehub.entityType.agent', 'Agent'),
    group: t('messagehub.entityType.group', 'Group'),
    service: t('messagehub.entityType.service', 'Service'),
  }

  return (
    <div
      className="flex flex-col h-full"
      style={{ background: 'var(--cp-surface)' }}
    >
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 py-3 flex-shrink-0"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        <h2
          className="text-sm font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          {t('messagehub.details', 'Details')}
        </h2>
        <button
          onClick={onClose}
          className="p-1 rounded-lg"
          style={{ color: 'var(--cp-muted)' }}
        >
          <X size={18} />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 py-4 shell-scrollbar">
        {/* Avatar & Name */}
        <div className="text-center mb-4">
          <DetailAvatar entity={entity} />
          <h3
            className="text-lg font-bold mt-3"
            style={{ color: 'var(--cp-text)' }}
          >
            {entity.name}
          </h3>
          <p className="text-xs mt-1" style={{ color: 'var(--cp-muted)' }}>
            {typeLabels[entity.type]}
            {entity.isOnline && (
              <span style={{ color: 'var(--cp-success)' }}> &bull; online</span>
            )}
          </p>
          {entity.statusText && (
            <p
              className="text-xs mt-0.5"
              style={{ color: 'var(--cp-muted)' }}
            >
              {entity.statusText}
            </p>
          )}
        </div>

        {/* Bio */}
        {entity.bio && (
          <div
            className="rounded-xl p-3 mb-4"
            style={{
              background:
                'color-mix(in srgb, var(--cp-text) 4%, transparent)',
            }}
          >
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('messagehub.bio', 'Bio')}
            </p>
            <p className="text-sm mt-1" style={{ color: 'var(--cp-text)' }}>
              {entity.bio}
            </p>
          </div>
        )}

        {/* Note */}
        {entity.note && (
          <div
            className="rounded-xl p-3 mb-4"
            style={{
              background:
                'color-mix(in srgb, var(--cp-text) 4%, transparent)',
            }}
          >
            <div className="flex items-center justify-between">
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {t('messagehub.note', 'Note')}
              </p>
              <button style={{ color: 'var(--cp-muted)' }}>
                <Edit3 size={12} />
              </button>
            </div>
            <p className="text-sm mt-1" style={{ color: 'var(--cp-text)' }}>
              {entity.note}
            </p>
          </div>
        )}

        {/* Info section */}
        <div
          className="rounded-xl px-3 mb-4"
          style={{
            background:
              'color-mix(in srgb, var(--cp-text) 4%, transparent)',
          }}
        >
          {entity.memberCount !== undefined && (
            <InfoRow
              label={t('messagehub.members', 'Members')}
              value={`${entity.memberCount}`}
              icon={<Users size={16} />}
            />
          )}
          {entity.tags.length > 0 && (
            <div className="py-2.5">
              <p className="text-xs mb-1.5" style={{ color: 'var(--cp-muted)' }}>
                {t('messagehub.tags', 'Tags')}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {entity.tags.map((tag) => (
                  <span
                    key={tag}
                    className="px-2 py-0.5 rounded-full text-xs"
                    style={{
                      background:
                        'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
                      color: 'var(--cp-accent)',
                    }}
                  >
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Bindings */}
        {entity.bindings && entity.bindings.length > 0 && (
          <div
            className="rounded-xl px-3 mb-4"
            style={{
              background:
                'color-mix(in srgb, var(--cp-text) 4%, transparent)',
            }}
          >
            <p
              className="text-xs pt-2.5"
              style={{ color: 'var(--cp-muted)' }}
            >
              {t('messagehub.accounts', 'Linked Accounts')}
            </p>
            {entity.bindings.map((b) => (
              <InfoRow
                key={`${b.platform}-${b.accountId}`}
                label={b.platform}
                value={b.displayId}
                icon={<Link2 size={14} />}
              />
            ))}
          </div>
        )}

        {/* Actions */}
        <div
          className="rounded-xl overflow-hidden mb-4"
          style={{
            background:
              'color-mix(in srgb, var(--cp-text) 4%, transparent)',
          }}
        >
          <button
            className="flex items-center gap-3 w-full px-3 py-2.5 text-sm text-left transition-colors"
            style={{ color: 'var(--cp-text)' }}
          >
            {entity.isMuted ? (
              <>
                <Bell size={16} />
                {t('messagehub.unmute', 'Unmute')}
              </>
            ) : (
              <>
                <BellOff size={16} />
                {t('messagehub.mute', 'Mute')}
              </>
            )}
          </button>
          <button
            className="flex items-center gap-3 w-full px-3 py-2.5 text-sm text-left transition-colors"
            style={{ color: 'var(--cp-text)' }}
          >
            {entity.isPinned ? (
              <>
                <PinOff size={16} />
                {t('messagehub.unpin', 'Unpin')}
              </>
            ) : (
              <>
                <Pin size={16} />
                {t('messagehub.pin', 'Pin')}
              </>
            )}
          </button>
        </div>

        {/* Danger zone */}
        <button
          className="flex items-center gap-3 w-full px-3 py-2.5 rounded-xl text-sm text-left"
          style={{
            color: 'var(--cp-danger)',
            background:
              'color-mix(in srgb, var(--cp-danger) 6%, transparent)',
          }}
        >
          <Trash2 size={16} />
          {t('messagehub.deleteChat', 'Delete Chat')}
        </button>
      </div>
    </div>
  )
}
