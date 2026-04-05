import { useState } from 'react'
import { Eye, EyeOff } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ProtocolType, WizardDraft } from '../../../mock/types'

interface StepConnectionProps {
  draft: WizardDraft
  onUpdate: (partial: Partial<WizardDraft>) => void
}

const defaultNames: Record<string, string> = {
  sn_router: 'SN Router',
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  google: 'Google AI',
  openrouter: 'OpenRouter',
  custom: '',
}

const defaultEndpoints: Record<string, string> = {
  openai: 'https://api.openai.com',
  anthropic: 'https://api.anthropic.com',
  google: 'https://generativelanguage.googleapis.com',
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
          className="w-full rounded-lg px-3 py-2.5 text-sm outline-none"
          style={{
            background: 'var(--cp-bg)',
            border: '1px solid var(--cp-border)',
            color: 'var(--cp-text)',
            height: 44,
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

export function StepConnection({ draft, onUpdate }: StepConnectionProps) {
  const { t } = useI18n()
  const providerType = draft.provider_type

  const name = draft.name || defaultNames[providerType ?? ''] || ''

  return (
    <div className="flex flex-col gap-4 max-w-lg">
      {/* Provider Name */}
      <InputField
        label={t('aiCenter.wizard.providerName', 'Provider Name')}
        value={name}
        onChange={(v) => onUpdate({ name: v })}
        placeholder={defaultNames[providerType ?? ''] || 'My Provider'}
        required={providerType === 'custom'}
      />

      {/* SN Router: just show status */}
      {providerType === 'sn_router' && (
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

      {/* API Key (not for sn_router) */}
      {providerType !== 'sn_router' && (
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
      {(providerType === 'openai' || providerType === 'anthropic' || providerType === 'google' || providerType === 'custom') && (
        <InputField
          label={t('aiCenter.wizard.endpoint', 'Endpoint')}
          value={draft.endpoint}
          onChange={(v) => onUpdate({ endpoint: v })}
          placeholder={defaultEndpoints[providerType] ?? 'https://'}
          required={providerType === 'custom'}
        />
      )}

      {/* Protocol Type (custom only) */}
      {providerType === 'custom' && (
        <div className="flex flex-col gap-1.5">
          <label className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.wizard.protocolType', 'Protocol Type')}
            <span style={{ color: 'var(--cp-danger)' }}> *</span>
          </label>
          <select
            value={draft.protocol_type ?? ''}
            onChange={(e) => onUpdate({ protocol_type: (e.target.value || null) as ProtocolType | null })}
            className="w-full rounded-lg px-3 py-2.5 text-sm outline-none appearance-none"
            style={{
              background: 'var(--cp-bg)',
              border: '1px solid var(--cp-border)',
              color: 'var(--cp-text)',
              height: 44,
            }}
          >
            <option value="">Select protocol...</option>
            <option value="openai_compatible">OpenAI Compatible</option>
            <option value="anthropic_compatible">Anthropic Compatible</option>
            <option value="google_compatible">Google Compatible</option>
          </select>
        </div>
      )}
    </div>
  )
}

export function isConnectionValid(draft: WizardDraft): boolean {
  if (!draft.provider_type) return false
  if (draft.provider_type === 'sn_router') return true
  if (!draft.api_key.trim()) return false
  if (draft.provider_type === 'custom') {
    if (!draft.endpoint.trim()) return false
    if (!draft.protocol_type) return false
  }
  return true
}
