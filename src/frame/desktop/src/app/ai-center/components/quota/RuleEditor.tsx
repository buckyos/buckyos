import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { QuotaApi } from '../../../../api/aicc_quota'
import {
  QUOTA_PERIODS,
  quotaWindow,
  type QuotaKind,
  type QuotaObject,
  type QuotaPeriod,
  type QuotaResource,
  type QuotaRuleDraft,
  type QuotaRulePreview,
  type QuotaRuleView,
} from '../../datamodel/quota'
import {
  Field,
  ResourceLabel,
  Segmented,
  SidePanel,
  StatusChip,
  Tokens,
  TokenInput,
} from './QuotaWidgets'
import { buttonStyle, formatLocal, useNow, useObjectLabel, usePeriodLabel } from './quota-labels'

export interface QuotaInventory { provider_instance_name: string; exact_models: string[] }

interface EditorProps {
  api: QuotaApi
  isAdmin: boolean
  viewerId: string
  inventory: QuotaInventory[]
  users: string[]
  apps: string[]
  /** Prefilled draft (from Usage) or `null`. */
  initial?: Partial<QuotaRuleDraft> | null
  /** Editing an existing rule instead of creating one. */
  editing?: QuotaRuleView | null
  onClose: () => void
  onSaved: () => void
}

type ObjectType = QuotaObject['type']
type ResourceType = 'all' | 'provider' | 'model'

const selectStyle = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function RuleEditor({ api, isAdmin, viewerId, inventory, users, apps, initial, editing, onClose, onSaved }: EditorProps) {
  const { t } = useI18n()
  const periodLabel = usePeriodLabel()
  const objectLabel = useObjectLabel()
  const prefillObject = initial?.object
  const prefillResource = initial?.resource
  const [objectType, setObjectType] = useState<ObjectType>(isAdmin ? prefillObject?.type ?? 'user' : 'user_app')
  const [userId, setUserId] = useState((prefillObject && prefillObject.type !== 'app' ? prefillObject.user_id : '') || (isAdmin ? users[0] ?? '' : viewerId))
  const [appId, setAppId] = useState(prefillObject && prefillObject.type !== 'user' ? prefillObject.app_id : '')
  const [resourceType, setResourceType] = useState<ResourceType>(prefillResource && prefillResource.type !== 'hidden' ? prefillResource.type : 'all')
  const [provider, setProvider] = useState(prefillResource?.type === 'provider' ? prefillResource.provider_instance_name : inventory[0]?.provider_instance_name ?? '')
  const [model, setModel] = useState(prefillResource?.type === 'model' ? prefillResource.exact_model : inventory.flatMap((p) => p.exact_models)[0] ?? '')
  const [kind, setKind] = useState<QuotaKind>(editing?.rule.kind ?? 'periodic')
  const [period, setPeriod] = useState<QuotaPeriod>(editing?.rule.period ?? '24h')
  const [limit, setLimit] = useState<number | null>(editing?.rule.limit_tokens ?? null)
  const [initialGrant, setInitialGrant] = useState<number | null>(null)
  const [enabled, setEnabled] = useState(editing?.rule.enabled ?? true)
  const [fetchedPreview, setPreview] = useState<QuotaRulePreview | null>(null)
  const [fetchedPreviewError, setPreviewError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const object: QuotaObject | null = useMemo(() => {
    const user = isAdmin ? userId.trim() : viewerId
    const app = appId.trim()
    if (objectType === 'user') return user ? { type: 'user', user_id: user } : null
    if (objectType === 'app') return app ? { type: 'app', app_id: app } : null
    return user && app ? { type: 'user_app', user_id: user, app_id: app } : null
  }, [objectType, userId, appId, isAdmin, viewerId])

  const resource: QuotaResource | null = useMemo(() => {
    if (resourceType === 'all') return { type: 'all' }
    if (resourceType === 'provider') return provider ? { type: 'provider', provider_instance_name: provider } : null
    return model ? { type: 'model', exact_model: model } : null
  }, [resourceType, provider, model])

  const draft: QuotaRuleDraft | null = useMemo(() => {
    if (!object || !resource) return null
    const source = isAdmin ? 'admin' as const : 'user' as const
    if (kind === 'periodic') return limit === null ? null : { source, object, resource, kind, period, limit_tokens: limit }
    return { source, object, resource, kind, initial_tokens: initialGrant ?? 0 }
  }, [object, resource, kind, period, limit, initialGrant, isAdmin])

  useEffect(() => {
    if (editing || !draft) return
    let live = true
    const timer = setTimeout(() => {
      api.previewRule(draft)
        .then((result) => { if (live) { setPreview(result); setPreviewError(null) } })
        .catch((reason: unknown) => { if (live) { setPreview(null); setPreviewError(errorText(reason)) } })
    }, 250)
    return () => { live = false; clearTimeout(timer) }
  }, [api, draft, editing])
  const preview = editing || !draft ? null : fetchedPreview
  const previewError = editing || !draft ? null : fetchedPreviewError

  const now = useNow()
  const window = quotaWindow(period, now)

  const save = async () => {
    setSaving(true)
    setError(null)
    try {
      if (editing) {
        const rule = editing.rule
        await api.updateRule({
          rule_id: rule.rule_id,
          expected_version: rule.version,
          ...(rule.kind === 'periodic' && limit !== null && limit !== rule.limit_tokens ? { limit_tokens: limit } : {}),
          ...(rule.kind === 'periodic' && period !== rule.period ? { period } : {}),
          ...(enabled !== rule.enabled ? { enabled } : {}),
        })
      } else if (draft) {
        await api.createRule(draft)
      }
      onSaved()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setSaving(false)
    }
  }

  const models = inventory.flatMap((p) => p.exact_models)
  const canSave = editing ? !saving : !saving && draft !== null && !preview?.conflict_rule_id

  return (
    <SidePanel
      title={editing ? t('aiCenter.quota.editTitle', 'Edit token quota') : t('aiCenter.quota.newTitle', 'New token quota')}
      onClose={onClose}
      testId="quota-editor"
    >
      <div className="flex flex-col gap-5">
        {editing ? (
          <div className="grid grid-cols-1 gap-3 rounded-xl p-4 sm:grid-cols-2" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <Field label={t('aiCenter.quota.objectLabel', 'Applies to')}>{objectLabel(editing.rule.object)}</Field>
            <Field label={t('aiCenter.quota.resourceLabel', 'Resource')}><ResourceLabel resource={editing.rule.resource} /></Field>
            <div className="text-xs sm:col-span-2" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.quota.identityFixed', 'Object and resource identify a rule; to change them create a new rule and disable this one.')}
            </div>
          </div>
        ) : (
          <>
            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.step.object', '1. Who is limited')}</h3>
              {isAdmin ? (
                <Segmented
                  label={t('aiCenter.quota.objectLabel', 'Applies to')}
                  value={objectType}
                  onChange={setObjectType}
                  options={[
                    { value: 'user', label: t('aiCenter.quota.objectType.user', 'A user') },
                    { value: 'app', label: t('aiCenter.quota.objectType.app', 'An App (all users share)') },
                    { value: 'user_app', label: t('aiCenter.quota.objectType.userApp', "A user's App") },
                  ]}
                />
              ) : (
                <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.ownAppHint', 'Your own limit on one of your Apps or Agents. Administrator limits still apply.')}</p>
              )}
              {isAdmin && objectType !== 'app' && (
                <label className="flex flex-col gap-1 text-sm">
                  <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.user', 'User')}</span>
                  <select data-testid="quota-user" value={userId} onChange={(event) => setUserId(event.target.value)} className="min-h-10 rounded-lg px-3" style={selectStyle}>
                    <option value="">{t('aiCenter.quota.pickUser', 'Pick a user')}</option>
                    {[...new Set([...users, ...(userId ? [userId] : [])])].map((user) => <option key={user} value={user}>{user}</option>)}
                  </select>
                </label>
              )}
              {objectType !== 'user' && (
                <label className="flex flex-col gap-1 text-sm">
                  <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.app', 'App or Agent')}</span>
                  <input data-testid="quota-app" list="quota-app-options" value={appId} onChange={(event) => setAppId(event.target.value)} placeholder="aiworkspace" className="min-h-10 rounded-lg px-3" style={selectStyle} />
                  <datalist id="quota-app-options">{apps.map((app) => <option key={app} value={app} />)}</datalist>
                </label>
              )}
            </section>
            <section className="flex flex-col gap-2">
              <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.step.resource', '2. Which models')}</h3>
              <Segmented
                label={t('aiCenter.quota.resourceLabel', 'Resource')}
                value={resourceType}
                onChange={setResourceType}
                options={[
                  { value: 'all', label: t('aiCenter.quota.resourceType.all', 'All token models') },
                  { value: 'provider', label: t('aiCenter.quota.resourceType.provider', 'A Provider'), disabled: inventory.length === 0 },
                  { value: 'model', label: t('aiCenter.quota.resourceType.model', 'A model'), disabled: models.length === 0 },
                ]}
              />
              {resourceType === 'all' && (
                <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.allHint', 'Counts every permitted token model; it grants no extra access.')}</p>
              )}
              {resourceType === 'provider' && (
                <select data-testid="quota-provider" value={provider} onChange={(event) => setProvider(event.target.value)} className="min-h-10 rounded-lg px-3" style={selectStyle}>
                  {inventory.map((p) => <option key={p.provider_instance_name} value={p.provider_instance_name}>{p.provider_instance_name}</option>)}
                </select>
              )}
              {resourceType === 'model' && (
                <select data-testid="quota-model" value={model} onChange={(event) => setModel(event.target.value)} className="min-h-10 rounded-lg px-3" style={selectStyle}>
                  {models.map((m) => <option key={m} value={m}>{m}</option>)}
                </select>
              )}
            </section>
            {isAdmin && (
              <section className="flex flex-col gap-2">
                <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.step.kind', '3. Kind')}</h3>
                <Segmented
                  label={t('aiCenter.quota.kindLabel', 'Kind')}
                  value={kind}
                  onChange={setKind}
                  options={[
                    { value: 'periodic', label: t('aiCenter.quota.kind.periodic', 'Periodic') },
                    { value: 'total', label: t('aiCenter.quota.kind.total', 'Total') },
                  ]}
                />
              </section>
            )}
          </>
        )}

        <section className="flex flex-col gap-3">
          {!editing && <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{isAdmin ? t('aiCenter.quota.step.amount', '4. Amount') : t('aiCenter.quota.step.amountUser', '3. Amount')}</h3>}
          {kind === 'periodic' ? (
            <>
              <TokenInput label={t('aiCenter.quota.limit', 'Token limit per period')} value={limit} onChange={setLimit} testId="quota-limit" />
              <label className="flex flex-col gap-1 text-sm">
                <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.periodLabel', 'Resets')}</span>
                <select data-testid="quota-period" value={period} onChange={(event) => setPeriod(event.target.value as QuotaPeriod)} className="min-h-10 rounded-lg px-3" style={selectStyle}>
                  {QUOTA_PERIODS.map((p) => <option key={p} value={p}>{periodLabel(p)}</option>)}
                </select>
                <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {editing && editing.rule.period && period !== editing.rule.period
                    ? t('aiCenter.quota.periodSwitch', 'Takes effect when the current window ends ({{time}})', { time: formatLocal(quotaWindow(editing.rule.period, now).endMs) })
                    : t('aiCenter.quota.windowNow', 'Current window {{start}} – {{end}}; fixed UTC boundaries, unused tokens do not carry over', { start: formatLocal(window.startMs), end: formatLocal(window.endMs) })}
                </span>
              </label>
            </>
          ) : editing ? (
            <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.totalEditHint', 'A total grant only grows by recharge from the rule details.')}</p>
          ) : (
            <>
              <TokenInput label={t('aiCenter.quota.initialGrant', 'Initial grant')} value={initialGrant} onChange={setInitialGrant} testId="quota-initial" />
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.totalHint', 'Counts from now on and never resets by itself; only an administrator recharge adds tokens.')}</p>
            </>
          )}
          {editing && editing.rule.kind === 'periodic' && limit !== null && limit < editing.state.settled_tokens + editing.state.reserved_tokens && (
            <p className="text-xs" style={{ color: 'var(--cp-warning)' }}>
              {t('aiCenter.quota.lowerWarning', 'Below what this window already used and reserved: new calls are blocked until the window resets; calls already admitted keep their reservation.')}
            </p>
          )}
          {editing && (
            <label className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} data-testid="quota-enabled" />
              {t('aiCenter.quota.enabled', 'Enforced')}
            </label>
          )}
        </section>

        {!editing && (
          <section className="flex flex-col gap-2 rounded-xl p-4" data-testid="quota-preview" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.previewTitle', 'Preview')}</h3>
            {previewError && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{previewError}</p>}
            {!preview && !previewError && <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.previewEmpty', 'Fill in the rule to see its current state.')}</p>}
            {preview && (
              <>
                <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                  <Field label={kind === 'total' ? t('aiCenter.quota.usedSinceStart', 'Used since start') : t('aiCenter.quota.usedThisWindow', 'Used this window')}><Tokens value={preview.state.settled_tokens} /></Field>
                  <Field label={t('aiCenter.quota.reserved', 'Reserved')}><Tokens value={preview.state.reserved_tokens} /></Field>
                  <Field label={t('aiCenter.quota.availableAfter', 'Available after saving')}><Tokens value={preview.state.available_tokens} strong /></Field>
                  <Field label={t('aiCenter.quota.statusLabel', 'Status')}><StatusChip status={preview.state.status} /></Field>
                </div>
                {preview.conflict_rule_id && (
                  <p className="text-xs" data-testid="quota-conflict" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.quota.conflict', 'A rule with the same object, resource and period exists; edit that rule instead.')}</p>
                )}
                {preview.existing_account && (
                  <p className="text-xs" style={{ color: 'var(--cp-warning)' }}>
                    {t('aiCenter.quota.accountRestored', 'This scope already has a total account (granted {{granted}}); saving restores it without a new initial grant. Add tokens by recharge.', { granted: preview.existing_account.granted_tokens.toLocaleString('en-US') })}
                  </p>
                )}
                {preview.other_constraints.length > 0 && (
                  <div className="flex flex-col gap-1">
                    <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.otherConstraints', 'Other limits that also apply')}</span>
                    {preview.other_constraints.map((view) => (
                      <div key={view.rule.rule_id} className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-text)' }}>
                        <span className="truncate">{objectLabel(view.rule.object)} · <ResourceLabel resource={view.rule.resource} /></span>
                        <span className="ml-auto"><StatusChip status={view.state.status} /></span>
                      </div>
                    ))}
                  </div>
                )}
                {!isAdmin && (
                  <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.adminStillApplies', 'A higher own limit can be saved; administrator limits still apply.')}</p>
                )}
              </>
            )}
          </section>
        )}

        {error && <p className="text-sm" data-testid="quota-editor-error" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onClose} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
          <button type="button" data-testid="quota-save" disabled={!canSave} onClick={() => void save()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>
            {saving ? t('aiCenter.quota.saving', 'Saving…') : t('aiCenter.quota.save', 'Save')}
          </button>
        </div>
      </div>
    </SidePanel>
  )
}
