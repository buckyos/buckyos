import { useEffect, useState } from 'react'
import { useI18n } from '../../../../../i18n/provider'
import { fetchUserGroups } from '../../../../../api/user_mgr'
import { useViewerIsAdmin } from '../../../../settings/hooks/use-viewer-is-admin'
import { usePolicyApi } from '../../../hooks/use-policy-api'
import { USER_TYPE_SUBJECTS, policyOf, sortSubjects, type AiccPolicyView } from '../../../datamodel/policy'
import { useSubjectLabel } from '../../policy/policy-labels'
import type { ProviderAccess } from './providerAccess'

/** Which user types and groups can use the Provider being added, and whether it costs them AI Credit;
 * only shown to administrators. */
export function ProviderAccessSection({ access, onChange }: { access: ProviderAccess; onChange: (access: ProviderAccess) => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin() === true
  const api = usePolicyApi()
  const [policies, setPolicies] = useState<AiccPolicyView[] | null>(null)
  const [groups, setGroups] = useState<{ id: string; name: string }[]>([])
  useEffect(() => {
    if (!isAdmin) return
    void Promise.all([api.listPolicies('manage'), fetchUserGroups()])
      .then(([list, groupList]) => { setPolicies(list); setGroups((groupList.data?.groups ?? []).map((group) => ({ id: group.group_id, name: group.name }))) })
      .catch(() => undefined)
  }, [api, isAdmin])
  const name = useSubjectLabel(Object.fromEntries(groups.map((group) => [group.id, group.name])))
  if (!isAdmin || !policies) return null
  const layers = sortSubjects([...USER_TYPE_SUBJECTS, ...groups.map((group) => `group:${group.id}`)])
  const following = layers.filter((subject) => policyOf(policies, subject, 'admin')?.policy.body.all_providers === true)
  const charged = following.filter((subject) => (policyOf(policies, subject, 'admin')?.policy.body.price_coefficient ?? 1) !== 0)
  const others = layers.filter((subject) => !following.includes(subject))
  return (
    <div className="mt-4 flex flex-col gap-3 rounded-xl p-4" data-testid="provider-access" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div>
        <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.wizardTitle', 'Who can use this Provider')}</h3>
        <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
          {following.length > 0
            ? t('aiCenter.policy.wizardFollowing', 'Gets it automatically (All Providers): {{groups}}', { groups: following.map(name).join(', ') })
            : t('aiCenter.policy.wizardNone', 'No user type or group follows all Providers; open it below.')}
        </p>
      </div>
      {others.length > 0 && (
        <div className="flex flex-col gap-2">
          {others.map((subject) => (
            <label key={subject} className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                data-testid="provider-access-subject"
                data-subject={subject}
                checked={access.openTo.includes(subject)}
                onChange={(event) => onChange({ ...access, openTo: event.target.checked ? [...access.openTo, subject] : access.openTo.filter((item) => item !== subject) })}
              />
              {t('aiCenter.policy.wizardOpenTo', 'Open to {{group}} (all models, following the list)', { group: name(subject) })}
            </label>
          ))}
        </div>
      )}
      <label className="flex items-start gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
        <input type="checkbox" data-testid="provider-access-free" checked={access.free} onChange={(event) => onChange({ ...access, free: event.target.checked })} className="mt-0.5" />
        <span>
          {t('aiCenter.policy.wizardFree', 'Free for everyone who can use it')}
          <span className="block text-xs" style={{ color: 'var(--cp-muted)' }}>
            {access.free
              ? t('aiCenter.policy.wizardFreeHint', 'Its Provider coefficient is set to 0, so it costs no AI Credit whether its models are priced or not.')
              : charged.length > 0
                ? t('aiCenter.policy.wizardCharged', 'Otherwise {{groups}} pay its models from the price list; models without a price cannot be used by them.', { groups: charged.map(name).join(', ') })
                : t('aiCenter.policy.wizardAllFree', 'The layers that get it are free now anyway.')}
          </span>
        </span>
      </label>
    </div>
  )
}
