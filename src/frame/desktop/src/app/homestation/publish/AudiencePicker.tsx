import { Globe, Lock, Users, UserCheck, UsersRound } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { AudienceSpec } from '../datamodel/types'
import { useStoreSelector } from '../store/context'
import type { HomeStationStore } from '../store/types'

const selectGroups = (store: HomeStationStore) => store.peekGroups()

export function AudiencePicker({ value, onChange, compact = false, idPrefix }: { value: AudienceSpec; onChange: (value: AudienceSpec) => void; compact?: boolean; idPrefix: string }) {
  const { t } = useI18n()
  const groups = useStoreSelector(selectGroups)
  const [didText, setDidText] = useState(value.kind === 'dids' ? value.dids.join(', ') : '')

  const choose = useCallback((kind: AudienceSpec['kind']) => {
    if (kind === 'group') onChange({ kind, groupId: groups[0]?.id ?? '' })
    else if (kind === 'dids') onChange({ kind, dids: didText.split(/[\s,]+/).filter(Boolean) })
    else onChange({ kind })
  }, [didText, groups, onChange])

  const options: { kind: AudienceSpec['kind']; label: string; icon: React.ReactNode }[] = [
    { kind: 'public', label: t('homestation.audience.public', 'Public'), icon: <Globe size={13} /> },
    { kind: 'followers', label: t('homestation.audience.followers', 'Followers'), icon: <UserCheck size={13} /> },
    { kind: 'friends', label: t('homestation.audience.friends', 'Friends'), icon: <Users size={13} /> },
    { kind: 'group', label: t('homestation.audience.group', 'Contact group'), icon: <UsersRound size={13} /> },
    { kind: 'dids', label: t('homestation.audience.dids', 'Specific DIDs'), icon: <Lock size={13} /> },
  ]

  return (
    <fieldset className="min-w-0" data-testid={`${idPrefix}-audience`}>
      <legend className="mb-1.5 text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
        {t('homestation.audience.label', 'Audience')}
      </legend>
      <div className={compact ? 'hs-scroll-x flex gap-1.5 overflow-x-auto' : 'flex flex-wrap gap-1.5'} role="radiogroup" aria-label={t('homestation.audience.label', 'Audience')}>
        {options.map(option => (
          <button
            key={option.kind}
            type="button"
            role="radio"
            aria-checked={value.kind === option.kind}
            className="hs-chip"
            onClick={() => choose(option.kind)}
          >
            {option.icon}
            {option.label}
          </button>
        ))}
      </div>
      {value.kind === 'group' ? (
        <label className="mt-2 flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.audience.chooseGroup', 'Group')}
          <select
            className="hs-input max-w-[220px] py-1.5 text-sm"
            value={value.groupId}
            onChange={event => onChange({ kind: 'group', groupId: event.target.value })}
          >
            {groups.map(group => (
              <option key={group.id} value={group.id}>{group.name}</option>
            ))}
          </select>
        </label>
      ) : null}
      {value.kind === 'dids' ? (
        <input
          className="hs-input mt-2 text-sm"
          value={didText}
          placeholder={t('homestation.audience.didsPlaceholder', 'did:bns:alice, did:bns:bob')}
          aria-label={t('homestation.audience.dids', 'Specific DIDs')}
          onChange={event => {
            setDidText(event.target.value)
            onChange({ kind: 'dids', dids: event.target.value.split(/[\s,]+/).filter(Boolean) })
          }}
        />
      ) : null}
      {value.kind !== 'public' ? (
        <p className="mt-1.5 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.audience.restrictedHint', 'Restricted posts can’t be reposted, and comments on them go only to you.')}
        </p>
      ) : null}
    </fieldset>
  )
}
