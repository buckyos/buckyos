import { useState } from 'react'
import { Eye, EyeOff } from 'lucide-react'
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

export function StepConnection({ draft, catalog, onUpdate }: StepConnectionProps) {
  const { t } = useI18n()
  const providerType = draft.provider_profile_id
  const profile = catalog?.providers.find((item) => item.provider_profile_id === providerType)

  return (
    <div className="flex flex-col gap-4 max-w-lg">
      {/* Provider Name */}
      <InputField
        label={t('aiCenter.wizard.instanceName', 'Instance Name')}
        value={draft.provider_instance_name ?? ''}
        onChange={(v) => onUpdate({ provider_instance_name: v })}
        placeholder={`${providerType ?? 'provider'}-main`}
      />

      {/* SN Router: just show status */}
      {providerType === 'sn' && (
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

      {providerType !== 'sn' && draft.auth_mode === 'api_key' && (
        <InputField
          label={t('aiCenter.wizard.apiKey', 'API Key')}
          value={draft.api_key}
          onChange={(v) => onUpdate({ api_key: v })}
          type="password"
          placeholder="sk-..."
          required
        />
      )}

      {/* Endpoint */}
      {providerType !== 'sn' && (
        <InputField
          label={t('aiCenter.wizard.baseUrl', 'Base URL')}
          value={draft.base_url}
          onChange={(v) => onUpdate({ base_url: v })}
          placeholder={profile?.base_url || 'https://'}
          required
        />
      )}

      {profile && (['region', 'workspace', 'account'] as const).map((name) => {
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
