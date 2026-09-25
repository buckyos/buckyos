import { useId, useState } from 'react'
import { Check, ChevronDown, Eye, EyeOff, MapPin } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ProviderSetupCatalog, WizardDraft } from '../../../../../api/aicc_mgr'

interface StepConnectionProps {
  draft: WizardDraft
  catalog: ProviderSetupCatalog | null
  onUpdate: (partial: Partial<WizardDraft>) => void
}

function InputField({
  label,
  value,
  onChange,
  placeholder,
  required,
  type = 'text',
}: {
  label: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  required?: boolean
  type?: string
}) {
  const [showPassword, setShowPassword] = useState(false)
  const isPassword = type === 'password'

  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
        {label}
        {required && <span style={{ color: 'var(--cp-danger)' }}> *</span>}
      </label>
      <div className="relative">
        <input
          type={isPassword && !showPassword ? 'password' : 'text'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          autoComplete={isPassword ? 'new-password' : undefined}
          className={`w-full rounded-lg px-3 py-2.5 text-sm outline-none${isPassword ? ' aicc-password-input' : ''}`}
          style={{
            background: 'var(--cp-bg)',
            border: '1px solid var(--cp-border)',
            color: 'var(--cp-text)',
            height: 44,
            paddingRight: isPassword ? 44 : 12,
          }}
        />
        {isPassword && (
          <button
            type="button"
            onClick={() => setShowPassword(!showPassword)}
            className="absolute right-3 top-1/2 -translate-y-1/2"
            style={{ color: 'var(--cp-muted)' }}
          >
            {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        )}
      </div>
    </div>
  )
}

interface UrlOption {
  key: string
  label: string
  url: string
}

function EditableUrlCombobox({
  label,
  value,
  onChange,
  options,
  placeholder,
  required,
}: {
  label: string
  value: string
  onChange: (value: string, selectedKey?: string) => void
  options: UrlOption[]
  placeholder?: string
  required?: boolean
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const fieldId = `provider-url-${useId().replace(/:/g, '')}`
  const listId = `${fieldId}-options`

  return (
    <div
      className="flex flex-col gap-1.5"
      onBlur={(event) => {
        const nextTarget = event.relatedTarget as Node | null
        if (!nextTarget || !event.currentTarget.contains(nextTarget)) setOpen(false)
      }}
    >
      <label htmlFor={fieldId} className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
        {label}
        {required && <span style={{ color: 'var(--cp-danger)' }}> *</span>}
      </label>
      <div className="relative">
        <input
          id={fieldId}
          role="combobox"
          aria-autocomplete="none"
          aria-expanded={open}
          aria-controls={options.length > 0 ? listId : undefined}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' && options.length > 0) setOpen(true)
            if (event.key === 'Escape') setOpen(false)
          }}
          placeholder={placeholder}
          className="w-full rounded-lg py-2.5 pl-3 text-sm outline-none"
          style={{
            background: 'var(--cp-bg)',
            border: '1px solid var(--cp-border)',
            color: 'var(--cp-text)',
            height: 44,
            paddingRight: options.length > 0 ? 44 : 12,
          }}
        />
        {options.length > 0 && (
          <button
            type="button"
            aria-label={`${label}: ${t('aiCenter.wizard.showEndpointOptions', 'Show configured endpoints')}`}
            aria-expanded={open}
            aria-controls={listId}
            onClick={() => setOpen((current) => !current)}
            className="absolute right-0 top-0 flex h-11 w-11 items-center justify-center"
            style={{ color: 'var(--cp-muted)' }}
          >
            <ChevronDown size={16} className={open ? 'rotate-180' : undefined} />
          </button>
        )}
        {open && options.length > 0 && (
          <div
            id={listId}
            role="listbox"
            className="absolute left-0 right-0 top-full z-20 mt-1 max-h-56 overflow-y-auto rounded-lg py-1 shadow-lg"
            style={{ background: 'var(--cp-surface-opaque)', border: '1px solid var(--cp-border)' }}
          >
            {options.map((option) => {
              const selected = option.url === value
              return (
                <button
                  key={`${option.key}:${option.url}`}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => {
                    onChange(option.url, option.key)
                    setOpen(false)
                  }}
                  className="flex w-full min-w-0 items-start gap-2 px-3 py-2 text-left hover:opacity-80"
                  style={{ color: 'var(--cp-text)' }}
                >
                  <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center">
                    {selected && <Check size={14} style={{ color: 'var(--cp-accent)' }} />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">{option.label}</span>
                    <span className="block truncate text-xs" title={option.url} style={{ color: 'var(--cp-muted)' }}>
                      {option.url}
                    </span>
                  </span>
                </button>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}

export function StepConnection({ draft, catalog, onUpdate }: StepConnectionProps) {
  const { t } = useI18n()
  const [showPolicyRegion, setShowPolicyRegion] = useState(
    Boolean(draft.policy_region && draft.policy_region !== 'unknown'),
  )
  const providerType = draft.provider_profile_id
  const profile = catalog?.providers.find((item) => item.provider_profile_id === providerType)
  const isDynamicSn = providerType === 'sn' && draft.auth_mode === 'dynamic_login'
  const operationBaseUrls = Object.entries(draft.operation_base_urls)
  const regionField = profile?.connection_fields.region
  const policyRegionField = profile?.connection_fields.policy_region
  const defaultRegion = regionField?.default_value ?? regionField?.allowed_values[0]
  const endpointRegions = regionField?.allowed_values.length
    ? [
      ...(defaultRegion ? [defaultRegion] : []),
      ...regionField.allowed_values.filter((region) => region !== defaultRegion),
    ]
    : []
  const endpointOptions = profile
    ? endpointRegions.length > 0
      ? endpointRegions.map((region) => ({
        key: region,
        url: profile.region_base_urls[region] ?? profile.base_url,
        label: profile.endpoint_hints[region]?.label ?? region,
      }))
      : [{ key: 'unknown', url: profile.base_url, label: t('aiCenter.wizard.defaultEndpoint', 'Default') }]
    : []
  const selectedEndpointHint = profile?.endpoint_hints[draft.region ?? 'unknown']

  const updateBaseUrl = (baseUrl: string, selectedRegion?: string) => {
    const endpoint = selectedRegion
      ? endpointOptions.find((option) => option.key === selectedRegion)
      : endpointOptions.find((option) => option.url === baseUrl)
    onUpdate({
      base_url: baseUrl,
      region: regionField ? endpoint?.key ?? 'unknown' : undefined,
    })
  }

  return (
    <div className="flex flex-col gap-4 max-w-lg">
      {/* Provider Name */}
      <InputField
        label={t('aiCenter.wizard.instanceName', 'Instance Name')}
        value={draft.provider_instance_name ?? ''}
        onChange={(v) => onUpdate({ provider_instance_name: v })}
        placeholder={`${providerType ?? 'provider'}-main`}
      />

      {isDynamicSn && (
        <div
          className="rounded-lg px-4 py-3 text-sm"
          style={{
            background: 'color-mix(in oklch, var(--cp-success), transparent 90%)',
            color: 'var(--cp-success)',
          }}
        >
          {t('aiCenter.wizard.snRouterHint', 'Account is activated')}
        </div>
      )}

      {draft.auth_mode === 'api_key' && (
        <InputField
          label={t('aiCenter.wizard.apiKey', 'API Key')}
          value={draft.api_key}
          onChange={(v) => onUpdate({ api_key: v })}
          type="password"
          placeholder="sk-..."
          required
        />
      )}

      {!isDynamicSn && (
        <>
          <EditableUrlCombobox
            label={t('aiCenter.wizard.baseUrl', 'Base URL')}
            value={draft.base_url}
            onChange={updateBaseUrl}
            options={endpointOptions}
            placeholder={profile?.base_url || 'https://'}
            required
          />
          {profile && (
            <p className="-mt-2 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.wizard.endpointHelp', 'Choose a configured endpoint or enter a custom Base URL. The endpoint does not describe where the user lives.')}
            </p>
          )}
          {selectedEndpointHint?.description && (
            <p className="-mt-2 text-xs leading-5" style={{ color: 'var(--cp-warning)' }}>
              {selectedEndpointHint.description}
            </p>
          )}
          {operationBaseUrls.map(([operation, url]) => (
            <EditableUrlCombobox
              key={operation}
              label={`${t('aiCenter.wizard.operationBaseUrl', 'Operation Base URL')} · ${operation}`}
              value={url}
              onChange={(next) => onUpdate({
                operation_base_urls: { ...draft.operation_base_urls, [operation]: next },
              })}
              options={profile?.operation_base_urls?.[operation]
                ? [{
                  key: operation,
                  label: t('aiCenter.wizard.defaultEndpoint', 'Default'),
                  url: profile.operation_base_urls[operation],
                }]
                : []}
              placeholder={profile?.operation_base_urls?.[operation] || 'https://'}
              required
            />
          ))}
        </>
      )}

      {profile && (['workspace', 'account'] as const).map((name) => {
        const field = profile.connection_fields[name]
        if (!field) return null
        const label = t(`aiCenter.wizard.${name}`, name)
        const value = draft[name] ?? ''
        if (field.allowed_values.length > 0) {
          return (
            <div key={name} className="flex flex-col gap-1.5">
              <label className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
                {label}{field.mode === 'required' && <span style={{ color: 'var(--cp-danger)' }}> *</span>}
              </label>
              <select
                value={value}
                onChange={(event) => onUpdate({ [name]: event.target.value } as Partial<WizardDraft>)}
                className="w-full appearance-none rounded-lg px-3 py-2.5 text-sm outline-none"
                style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)', height: 44 }}
              >
                {field.mode === 'optional' && !field.default_value && <option value="">{t('common.default', 'Default')}</option>}
                {field.allowed_values.map((option) => <option key={option} value={option}>{option}</option>)}
              </select>
            </div>
          )
        }
        return <InputField key={name} label={label} value={value} onChange={(next) => onUpdate({ [name]: next } as Partial<WizardDraft>)} required={field.mode === 'required'} />
      })}

      {policyRegionField && !showPolicyRegion && (
        <button
          type="button"
          onClick={() => setShowPolicyRegion(true)}
          aria-expanded="false"
          className="flex min-h-11 items-center justify-center gap-2 rounded-lg px-3 py-2 text-sm font-medium"
          style={{ border: '1px dashed var(--cp-border)', color: 'var(--cp-muted)' }}
        >
          <MapPin size={15} />
          {t('aiCenter.wizard.setPolicyRegion', 'Set residence / account region')}
        </button>
      )}

      {policyRegionField && showPolicyRegion && (
        <div className="flex flex-col gap-1.5" aria-label={t('aiCenter.wizard.policy_region', 'Residence / account region')}>
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="provider-policy-region" className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.wizard.policy_region', 'Residence / account region')}
            </label>
            <button
              type="button"
              onClick={() => {
                onUpdate({ policy_region: 'unknown' })
                setShowPolicyRegion(false)
              }}
              className="text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {t('aiCenter.wizard.useUnknownRegion', 'Use unknown')}
            </button>
          </div>
          {policyRegionField.allowed_values.length > 0 ? (
            <select
              id="provider-policy-region"
              value={draft.policy_region ?? 'unknown'}
              onChange={(event) => onUpdate({ policy_region: event.target.value })}
              className="w-full appearance-none rounded-lg px-3 py-2.5 text-sm outline-none"
              style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)', height: 44 }}
            >
              {policyRegionField.allowed_values.map((option) => (
                <option key={option} value={option}>
                  {option === 'unknown' ? t('aiCenter.wizard.unknownRegion', 'Unknown') : option}
                </option>
              ))}
            </select>
          ) : (
            <InputField
              label=""
              value={draft.policy_region ?? 'unknown'}
              onChange={(policy_region) => onUpdate({ policy_region })}
            />
          )}
          <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.wizard.policyRegionHelp', 'Used only for provider policy and model availability. Unknown keeps models routable, but the provider may reject a request.')}
          </p>
        </div>
      )}

      {/* Protocol Type (custom only) */}
      {providerType === 'custom' && (
        <div className="flex flex-col gap-1.5">
          <label className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.wizard.protocolFamily', 'Protocol Family')}
            <span style={{ color: 'var(--cp-danger)' }}> *</span>
          </label>
          <select
            value={draft.protocol_family_id ?? ''}
            onChange={(e) => onUpdate({ protocol_family_id: e.target.value || null })}
            className="w-full rounded-lg px-3 py-2.5 text-sm outline-none appearance-none"
            style={{
              background: 'var(--cp-bg)',
              border: '1px solid var(--cp-border)',
              color: 'var(--cp-text)',
              height: 44,
            }}
          >
            <option value="">{t('aiCenter.wizard.selectProtocolFamily', 'Select protocol family...')}</option>
            {(catalog?.protocol_families ?? []).map((family) => <option key={family.protocol_family_id} value={family.protocol_family_id}>{family.display_name}</option>)}
          </select>
        </div>
      )}
    </div>
  )
}
