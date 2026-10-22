import { useEffect, useState } from 'react'
import { useI18n } from '../../../../../i18n/provider'
import { fetchUserGroups, type UserGroupListItem } from '../../../../../api/user_mgr'
import { useViewerIsAdmin } from '../../../../settings/hooks/use-viewer-is-admin'
import { useQuotaApi } from '../../../hooks/use-quota-api'
import { sortSubjects, type VisibilityState } from '../../../datamodel/quota'

/** Which groups can use the Provider being added; only shown to administrators. */
export function ProviderAccessSection({ selected, onChange }: { selected: string[]; onChange: (subjects: string[]) => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin() === true
  const api = useQuotaApi()
  const [state, setState] = useState<VisibilityState | null>(null)
  const [groups, setGroups] = useState<UserGroupListItem[]>([])
  useEffect(() => {
    if (!isAdmin) return
    void Promise.all([api.getVisibility(), fetchUserGroups()])
      .then(([visibility, list]) => { setState(visibility); setGroups(list.data?.groups ?? []) })
      .catch(() => undefined)
  }, [api, isAdmin])
  if (!isAdmin || !state) return null
  const following = state.subjects.filter((item) => item.entries.some((entry) => entry.type === 'all_providers')).map((item) => item.subject)
  const others = sortSubjects(['type:admin', 'type:user', 'type:limited', ...groups.map((group) => `group:${group.group_id}`)]).filter((subject) => !following.includes(subject))
  const name = (subject: string) => {
    if (subject === 'type:admin') return t('aiCenter.visibility.type.admin', 'Administrators')
    if (subject === 'type:user') return t('aiCenter.visibility.type.user', 'Users')
    if (subject === 'type:limited') return t('aiCenter.visibility.type.limited', 'Limited users')
    return groups.find((group) => `group:${group.group_id}` === subject)?.name ?? subject
  }
  return (
    <div className="mt-4 rounded-xl p-4" data-testid="provider-access" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.visibility.wizardTitle', 'Who can use this Provider')}</h3>
      <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
        {following.length > 0
          ? t('aiCenter.visibility.wizardFollowing', 'Gets it automatically (All Providers): {{groups}}', { groups: following.map(name).join(', ') })
          : t('aiCenter.visibility.wizardNone', 'No group follows all Providers; open it to groups below.')}
      </p>
      {others.length > 0 && (
        <div className="mt-3 flex flex-col gap-2">
          {others.map((subject) => (
            <label key={subject} className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                data-testid="provider-access-subject"
                data-subject={subject}
                checked={selected.includes(subject)}
                onChange={(event) => onChange(event.target.checked ? [...selected, subject] : selected.filter((item) => item !== subject))}
              />
              {t('aiCenter.visibility.wizardOpenTo', 'Open to {{group}} (all models, following the list)', { group: name(subject) })}
            </label>
          ))}
        </div>
      )}
    </div>
  )
}
