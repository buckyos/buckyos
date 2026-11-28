import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { EffectiveQuery, PolicyApi } from '../../../../api/aicc_policy'
import {
  normalizeBody,
  parseSubject,
  startsCharging,
  type AiccPolicyView,
  type PolicyDraft,
  type PolicyEffectiveResponse,
} from '../../datamodel/policy'
import { Modal } from './PolicyWidgets'
import { buttonStyle, errorText, fieldStyle } from './policy-labels'

interface SaveDialogProps {
  api: PolicyApi
  viewerId: string
  draft: PolicyDraft
  stored?: AiccPolicyView
  /** The `type:admin` policy, asked about when `type:user` starts charging (Q17). */
  adminPolicy?: AiccPolicyView
  subjectLabel: (subject: string) => string
  onClose: () => void
  onSaved: () => void
}

/** The user (and App) a draft is previewed for: the subject's own user, or one of the users it covers. */
function previewTarget(subject: string, user?: string): EffectiveQuery {
  const parts = parseSubject(subject)
  if (parts?.kind === 'user') return { userId: parts.userId }
  if (parts?.kind === 'user_app') return { userId: parts.userId, appId: parts.appId }
  if (parts?.kind === 'app') return { userId: user, appId: parts.appId }
  return { userId: user }
}

function ModelList({ models, empty }: { models?: string[]; empty: string }) {
  if (!models?.length) return <span style={{ color: 'var(--cp-muted)' }}>{empty}</span>
  return <span className="break-all">{models.join(', ')}</span>
}

/** Save with a preview: what the draft opens and removes for a covered user, what the viewer loses, and the
 * unpriced models that stop working when a free layer starts charging. */
export function PolicySaveDialog({ api, viewerId, draft, stored, adminPolicy, subjectLabel, onClose, onSaved }: SaveDialogProps) {
  const { t } = useI18n()
  const [previewUser, setPreviewUser] = useState<string | undefined>(undefined)
  const [preview, setPreview] = useState<PolicyEffectiveResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [chargeAdmins, setChargeAdmins] = useState(false)
  const charging = startsCharging(stored?.policy.body, draft.body ?? {})
  const newCoefficient = draft.body?.price_coefficient
  const askAdmins = charging && draft.subject === 'type:user' && newCoefficient !== undefined && newCoefficient > 0 && adminPolicy?.policy.body.price_coefficient === 0

  useEffect(() => {
    let live = true
    const run = async () => {
      try {
        let result = await api.effective({ ...previewTarget(draft.subject, previewUser), draft })
        // A group or user type the viewer is not in: preview it for one of its members instead.
        const members = result.affected_users ?? []
        if (previewUser === undefined && !parseSubject(draft.subject)?.userId && members.length > 0 && !members.includes(viewerId)) {
          result = await api.effective({ ...previewTarget(draft.subject, members[0]), draft })
          if (live) setPreviewUser(members[0])
        }
        if (live) { setPreview(result); setError(null) }
      } catch (reason) {
        if (live) setError(errorText(reason))
      }
    }
    void run()
    return () => { live = false }
  }, [api, draft, previewUser, viewerId])

  const save = async () => {
    setSaving(true)
    setError(null)
    try {
      await api.upsertPolicy({ ...draft, body: normalizeBody(draft.body ?? {}) }, stored?.policy.version)
      if (chargeAdmins && adminPolicy && newCoefficient !== undefined) {
        await api.upsertPolicy({ source: 'admin', subject: 'type:admin', enabled: adminPolicy.policy.enabled, body: normalizeBody({ ...adminPolicy.policy.body, price_coefficient: newCoefficient }) }, adminPolicy.policy.version)
      }
      onSaved()
    } catch (reason) {
      const code = (reason as { code?: string }).code
      setError(code === 'settings_revision_conflict'
        ? t('aiCenter.policy.conflict', 'Someone else changed this policy meanwhile. Close, reload and edit again.')
        : errorText(reason))
    } finally {
      setSaving(false)
    }
  }

  const members = preview?.affected_users ?? []
  const target = preview?.user_id ?? previewUser ?? viewerId
  return (
    <Modal title={t('aiCenter.policy.confirmTitle', 'Save this policy?')} onClose={onClose} busy={saving} testId="policy-confirm">
      <div className="flex flex-col gap-3 text-sm" style={{ color: 'var(--cp-text)' }}>
        <p style={{ color: 'var(--cp-muted)' }}>{subjectLabel(draft.subject)} · {draft.subject}</p>
        {!preview && !error && <p style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.previewing', 'Working out what changes…')}</p>}
        {preview && (
          <>
            {(preview.caller_loses?.length ?? 0) > 0 && (
              <p data-testid="policy-self-loss" style={{ color: 'var(--cp-danger)' }}>
                {t('aiCenter.policy.selfLoss', 'You yourself will no longer be able to use: {{models}}', { models: preview.caller_loses!.join(', ') })}
              </p>
            )}
            {charging && (
              <div data-testid="policy-unpriced-warning" className="rounded-lg p-3" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)' }}>
                <p className="font-medium">{t('aiCenter.policy.chargingStarts', 'This starts charging AI Credit.')}</p>
                <p className="mt-1 text-xs">
                  {(preview.unpriced_models?.length ?? 0) > 0
                    ? t('aiCenter.policy.chargingUnpriced', 'These models have no price, so users here can no longer use them: {{models}}. Price them under Price list, or keep their Provider free.', { models: preview.unpriced_models!.join(', ') })
                    : t('aiCenter.policy.chargingAllPriced', 'Every model these users can see has a price.')}
                </p>
              </div>
            )}
            {askAdmins && (
              <label className="flex items-start gap-2 text-sm">
                <input type="checkbox" data-testid="policy-charge-admins" checked={chargeAdmins} onChange={(event) => setChargeAdmins(event.target.checked)} className="mt-0.5" />
                <span>{t('aiCenter.policy.chargeAdmins', 'Also charge administrators (the Administrators policy is free now)')}</span>
              </label>
            )}
            {members.length > 1 && !parseSubject(draft.subject)?.userId && (
              <label className="flex flex-col gap-1 text-xs">
                <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.previewFor', 'Preview for')}</span>
                <select data-testid="policy-confirm-user" value={target} onChange={(event) => { setPreview(null); setPreviewUser(event.target.value) }} className="min-h-10 rounded-lg px-3 text-sm" style={fieldStyle}>
                  {[...new Set([target, ...members])].map((user) => <option key={user} value={user}>{user}</option>)}
                </select>
              </label>
            )}
            <div className="flex flex-col gap-1" data-testid="policy-diff">
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.diffFor', 'For {{user}}:', { user: target })}</p>
              <p>{t('aiCenter.policy.added', 'Opened')}: <ModelList models={preview.added} empty="—" /></p>
              <p>{t('aiCenter.policy.removed', 'Removed or no longer usable')}: <ModelList models={preview.removed} empty="—" /></p>
            </div>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.policy.affected', 'Applies to: {{users}}', { users: members.length ? members.join(', ') : '—' })}
            </p>
          </>
        )}
        {error && <p className="text-sm" data-testid="policy-save-error" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      </div>
      <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <button type="button" onClick={onClose} disabled={saving} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
        <button type="button" data-testid="policy-save" disabled={saving || !preview} onClick={() => void save()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>
          {saving ? t('aiCenter.policy.saving', 'Saving…') : t('aiCenter.policy.save', 'Save')}
        </button>
      </div>
    </Modal>
  )
}
