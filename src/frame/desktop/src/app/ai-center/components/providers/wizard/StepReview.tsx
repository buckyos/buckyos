import { Check } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { LongField } from '../../shared/LongField'
import type { ValidationResult, WizardDraft } from '../../../../../api/aicc_mgr'

interface StepReviewProps {
  draft: WizardDraft
  validation: ValidationResult | null
  onToggleAutoSync: (value: boolean) => void
}

const defaultNames: Record<string, string> = {
  sn_router: 'SN Router',
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  google: 'Google AI',
  openrouter: 'OpenRouter',
  custom: '',
}

export function StepReview({ draft, validation, onToggleAutoSync }: StepReviewProps) {
  const { t } = useI18n()
  const providerName = draft.name || defaultNames[draft.provider_type ?? ''] || draft.provider_instance_name || '-'

  const rows = [
    { label: t('aiCenter.providers.type', 'Type'), value: draft.provider_type ?? '-' },
    { label: t('aiCenter.wizard.providerName', 'Provider Name'), value: providerName },
    { label: t('aiCenter.providers.endpoint', 'Endpoint'), value: draft.endpoint || t('aiCenter.providers.default', 'Default') },
    { label: t('aiCenter.providers.auth', 'Authentication'), value: draft.api_key ? 'API Key' : '-' },
    {
      label: 'Connection',
      value: (
        <span className="inline-flex items-center gap-1">
          <Check size={14} style={{ color: 'var(--cp-success)' }} />
          {t('aiCenter.providers.connected', 'Connected')}
        </span>
      ),
    },
    {
      label: t('aiCenter.providers.models', 'Models'),
      value: `${validation?.models_discovered.length ?? 0}`,
    },
  ]

  return (
    <div className="max-w-lg">
      <div
        className="rounded-xl p-4"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex flex-col gap-3">
          {rows.map((row) => (
            <div key={row.label} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.35fr)] gap-4 text-sm">
              <span className="min-w-0 truncate" style={{ color: 'var(--cp-muted)' }}>{row.label}</span>
              <span className="min-w-0 justify-self-end text-right font-medium" style={{ color: 'var(--cp-text)' }}>
                {typeof row.value === 'string' ? <LongField value={row.value} expandable /> : row.value}
              </span>
            </div>
          ))}

          <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-4 items-center text-sm pt-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
            <span className="min-w-0 truncate" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.wizard.autoSync', 'Auto-sync model list')}
            </span>
            <button
              type="button"
              onClick={() => onToggleAutoSync(!draft.auto_sync_models)}
              className="relative h-6 w-11 rounded-full transition-colors"
              style={{
                background: draft.auto_sync_models ? 'var(--cp-accent)' : 'var(--cp-border)',
              }}
            >
              <span
                className="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform"
                style={{
                  transform: draft.auto_sync_models ? 'translateX(20px)' : 'translateX(0)',
                }}
              />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
