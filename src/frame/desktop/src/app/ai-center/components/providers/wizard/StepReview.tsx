import { Check } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ValidationResult, WizardDraft } from '../../../mock/types'

interface StepReviewProps {
  draft: WizardDraft
  validation: ValidationResult | null
  onToggleAutoSync: (value: boolean) => void
}

export function StepReview({ draft, validation, onToggleAutoSync }: StepReviewProps) {
  const { t } = useI18n()

  const rows = [
    { label: t('aiCenter.providers.type', 'Type'), value: draft.provider_type ?? '—' },
    { label: t('aiCenter.wizard.providerName', 'Provider Name'), value: draft.name || '—' },
    { label: t('aiCenter.providers.endpoint', 'Endpoint'), value: draft.endpoint || t('aiCenter.providers.default', 'Default') },
    { label: t('aiCenter.providers.auth', 'Authentication'), value: draft.api_key ? 'API Key' : '—' },
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
            <div key={row.label} className="flex justify-between text-sm">
              <span style={{ color: 'var(--cp-muted)' }}>{row.label}</span>
              <span className="font-medium" style={{ color: 'var(--cp-text)' }}>{row.value}</span>
            </div>
          ))}

          <div className="flex justify-between items-center text-sm pt-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
            <span style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.wizard.autoSync', 'Auto-sync model list')}
            </span>
            <button
              type="button"
              onClick={() => onToggleAutoSync(!draft.auto_sync_models)}
              className="relative w-10 h-5 rounded-full transition-colors"
              style={{
                background: draft.auto_sync_models ? 'var(--cp-accent)' : 'var(--cp-border)',
              }}
            >
              <span
                className="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform"
                style={{
                  transform: draft.auto_sync_models ? 'translateX(22px)' : 'translateX(2px)',
                }}
              />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
