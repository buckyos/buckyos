import { useState } from 'react'
import { ArrowLeft } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { useMockStore } from '../../../hooks/use-mock-store'
import type { ProviderType, ValidationResult, WizardDraft } from '../../../mock/types'
import { Stepper } from '../../shared/Stepper'
import { StepChooseType } from './StepChooseType'
import { StepConnection, isConnectionValid } from './StepConnection'
import { StepValidation } from './StepValidation'
import { StepReview } from './StepReview'

const INITIAL_DRAFT: WizardDraft = {
  provider_type: null,
  name: '',
  endpoint: '',
  protocol_type: null,
  api_key: '',
  auto_sync_models: true,
}

interface WizardShellProps {
  onBack: () => void
  onCreated: () => void
}

export function WizardShell({ onBack, onCreated }: WizardShellProps) {
  const { t } = useI18n()
  const store = useMockStore()

  const [step, setStep] = useState(0)
  const [draft, setDraft] = useState<WizardDraft>(INITIAL_DRAFT)
  const [validation, setValidation] = useState<ValidationResult | null>(null)
  const [creating, setCreating] = useState(false)

  const steps = [
    t('aiCenter.wizard.step.chooseType', 'Choose Type'),
    t('aiCenter.wizard.step.connection', 'Connection'),
    t('aiCenter.wizard.step.validation', 'Validation'),
    t('aiCenter.wizard.step.review', 'Review'),
  ]

  const updateDraft = (partial: Partial<WizardDraft>) => {
    setDraft((prev) => ({ ...prev, ...partial }))
  }

  const canNext = () => {
    switch (step) {
      case 0: return draft.provider_type !== null
      case 1: return isConnectionValid(draft)
      case 2: return validation !== null && !validation.errors.some((e) =>
        !e.includes('balance') // allow balance errors
      ) && validation.endpoint_reachable && validation.auth_valid
      case 3: return true
      default: return false
    }
  }

  const handleNext = async () => {
    if (step === 3) {
      setCreating(true)
      await new Promise((r) => setTimeout(r, 300))
      store.addProvider(draft)
      onCreated()
      return
    }
    if (step === 1) {
      // Reset validation when moving to step 2
      setValidation(null)
    }
    setStep((s) => s + 1)
  }

  const handlePrev = () => {
    if (step === 0) {
      onBack()
    } else {
      if (step === 2) setValidation(null)
      setStep((s) => s - 1)
    }
  }

  const handleTypeSelect = (type: ProviderType) => {
    updateDraft({
      provider_type: type,
      name: '',
      endpoint: '',
      protocol_type: null,
      api_key: '',
    })
  }

  return (
    <div className="flex flex-col h-full -mx-4 md:-mx-8 -my-4 md:-my-6">
      {/* Header */}
      <div
        className="flex items-center gap-3 px-4 md:px-6 py-3 shrink-0"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        <button
          type="button"
          onClick={handlePrev}
          className="p-1 rounded-md hover:opacity-70"
          style={{ color: 'var(--cp-muted)' }}
        >
          <ArrowLeft size={18} />
        </button>
        <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
          {t('aiCenter.wizard.title', 'Add Provider')}
        </span>
      </div>

      {/* Stepper */}
      <div className="px-4 md:px-6 py-3 shrink-0" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <Stepper steps={steps} current={step} />
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 md:px-6 py-6">
        {step === 0 && (
          <StepChooseType selected={draft.provider_type} onSelect={handleTypeSelect} />
        )}
        {step === 1 && (
          <StepConnection draft={draft} onUpdate={updateDraft} />
        )}
        {step === 2 && (
          <StepValidation draft={draft} onResult={setValidation} />
        )}
        {step === 3 && (
          <StepReview
            draft={draft}
            validation={validation}
            onToggleAutoSync={(v) => updateDraft({ auto_sync_models: v })}
          />
        )}
      </div>

      {/* Footer */}
      <div
        className="flex justify-between items-center px-4 md:px-6 py-3 shrink-0"
        style={{ borderTop: '1px solid var(--cp-border)' }}
      >
        <button
          type="button"
          onClick={handlePrev}
          className="px-4 py-2 rounded-lg text-sm"
          style={{ color: 'var(--cp-muted)' }}
        >
          {step === 0 ? t('aiCenter.wizard.back', 'Back') : t('aiCenter.wizard.prev', 'Previous')}
        </button>

        {step === 2 && validation && !validation.auth_valid ? (
          <button
            type="button"
            onClick={() => { setValidation(null); setStep(1) }}
            className="px-4 py-2 rounded-lg text-sm font-medium"
            style={{ background: 'var(--cp-warning)', color: '#fff' }}
          >
            {t('aiCenter.wizard.goBackToFix', 'Go Back to Fix')}
          </button>
        ) : (
          <button
            type="button"
            onClick={handleNext}
            disabled={!canNext() || creating}
            className="px-5 py-2 rounded-lg text-sm font-medium transition-opacity disabled:opacity-40"
            style={{ background: 'var(--cp-accent)', color: '#fff' }}
          >
            {step === 3
              ? creating
                ? t('aiCenter.wizard.creating', 'Creating...')
                : t('aiCenter.wizard.create', 'Create Provider')
              : t('aiCenter.wizard.next', 'Next')}
          </button>
        )}
      </div>
    </div>
  )
}
