import { Globe, Lock, Users, UserCheck, UserPlus, UsersRound } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { PersonName } from '../../../components/network/Person'
import type { AudienceSpec } from '../datamodel/types'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import type { HomeStationStore } from '../store/types'

const selectGroups = (store: HomeStationStore) => store.peekGroups()

export function AudiencePicker({ value, onChange, compact = false, idPrefix }: { value: AudienceSpec; onChange: (value: AudienceSpec) => void; compact?: boolean; idPrefix: string }) {
  const { t } = useI18n()
  const groups = useStoreSelector(selectGroups)
  const dialogs = useNetworkDialogs()
  const store = useHomeStationStore()
  const [dids, setDids] = useState(value.kind === 'dids' ? value.dids : [])
  const [names, setNames] = useState<Record<string, string>>({})

  const choose = useCallback((kind: AudienceSpec['kind']) => {
    if (kind === 'group') onChange({ kind, groupId: groups[0]?.id ?? '' })
    else if (kind === 'dids') onChange({ kind, dids })
    else onChange({ kind })
  }, [dids, groups, onChange])

  const pickPeople = async () => {
    const result = await dialogs.pickPeople({
      title: t('homestation.audience.choosePeople', 'Who can see it'),
      confirmLabel: t('homestation.audience.useThese', 'Use these people'),
      limit: 20,
      filter: { kinds: ['user', 'agent'] },
      initialSelection: value.kind === 'dids' ? value.dids : dids,
    })
    if (result.action !== 'selected') return
    const next = result.people.map(person => person.did)
    setNames(current => ({ ...current, ...Object.fromEntries(result.people.map(person => [person.did, person.name])) }))
    setDids(next)
    onChange({ kind: 'dids', dids: next })
  }

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
        <div className="mt-2 flex flex-wrap items-center gap-1.5" data-testid={`${idPrefix}-audience-people`}>
          {value.dids.map(did => <span key={did} className="hs-badge"><PersonName person={{ did, name: names[did] ?? store.peekIdentity(did).name }} /></span>)}
          <button type="button" className="hs-chip" onClick={() => void pickPeople()} data-testid={`${idPrefix}-audience-choose`}>
            <UserPlus size={13} />
            {value.dids.length ? t('homestation.audience.changePeople', 'Change') : t('homestation.audience.choosePeople', 'Who can see it')}
          </button>
        </div>
      ) : null}
      {value.kind !== 'public' ? (
        <p className="mt-1.5 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.audience.restrictedHint', 'Restricted posts can’t be reposted, and comments on them go only to you.')}
        </p>
      ) : null}
    </fieldset>
  )
}
