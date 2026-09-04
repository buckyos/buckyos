import { useEffect, useState, type FocusEvent } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { ArrowLeft } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { useAICCStore, useProviders } from '../../../hooks/use-aicc-store'
import type { ProviderSetupCatalog, ProviderType, ValidationResult, WizardDraft } from '../../../../../api/aicc_mgr'
import { Stepper } from '../../shared/Stepper'
import { StepChooseType } from './StepChooseType'
import { StepConnection } from './StepConnection'
import { StepValidation } from './StepValidation'
import { StepReview } from './StepReview'
import { isConnectionValid, wizardDraftSchema } from './connectionValidation'

const INITIAL_DRAFT: WizardDraft = {
  provider_profile_id: null,
  display_name: '',
  base_url: '',
  protocol_family_id: null,
  auth_mode: 'api_key',
  api_key: '',
  auto_sync_models: true,
}

interface WizardShellProps {
  onBack: () => void
  onCreated: () => void
}

export function WizardShell({ onBack, onCreated }: WizardShellProps) {
  const { t } = useI18n()
  const store = useAICCStore()
  const providers = useProviders()
  const hasManagedSnProvider = providers.some((provider) => provider.config.provider_profile_id === 'sn')

  const [step, setStep] = useState(0)
  const form = useForm<WizardDraft>({
    resolver: zodResolver(wizardDraftSchema),
    defaultValues: INITIAL_DRAFT,
    mode: 'onChange',
  })
  const draft = useWatch({ control: form.control }) as WizardDraft
  const [catalog, setCatalog] = useState<ProviderSetupCatalog | null>(null)
  const [catalogLoading, setCatalogLoading] = useState(true)
  const [catalogError, setCatalogError] = useState<string | null>(null)
  const [validation, setValidation] = useState<ValidationResult | null>(null)
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [keyboardInset, setKeyboardInset] = useState(0)
  const selectedProfile = catalog?.providers.find((item) => item.provider_profile_id === draft.provider_profile_id)

  const steps = [
    t('aiCenter.wizard.step.chooseType', 'Choose Type'),
    t('aiCenter.wizard.step.connection', 'Connection'),
    t('aiCenter.wizard.step.validation', 'Validation'),
    t('aiCenter.wizard.step.review', 'Review'),
  ]

  const updateDraft = (partial: Partial<WizardDraft>) => {
    setCreateError(null)
    for (const [key, value] of Object.entries(partial)) {
      form.setValue(key as keyof WizardDraft, value as never, { shouldDirty: true, shouldValidate: true })
    }
  }

  const canNext = () => {
    switch (step) {
      case 0: return draft.provider_profile_id !== null
      case 1: return isConnectionValid(draft) && providerConnectionFieldsValid(draft, selectedProfile)
      case 2: return validation !== null && validationCanProceed(validation)
      case 3: return true
      default: return false
    }
  }

  const handleNext = async () => {
    if (step === 3) {
      setCreating(true)
      setCreateError(null)
      try {
        await new Promise((r) => setTimeout(r, 300))
        await store.addProvider(draft)
        onCreated()
      } catch (error) {
        const message = error instanceof Error ? error.message : ''
        if (message.includes('provider_validation_required') || message.includes('provider_validation_mismatch')) {
          setCreateError(t('aiCenter.wizard.validationExpired', 'Provider validation expired. Please validate again.'))
          setValidation(null)
          setStep(2)
        } else {
          setCreateError(message || t('aiCenter.wizard.createFailed', 'Could not create provider.'))
        }
      } finally {
        setCreating(false)
      }
      return
    }
    if (step === 1) {
      // Reset validation when moving to step 2
      setValidation(null)
      setCreateError(null)
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
    if (type === 'sn' && hasManagedSnProvider) {
      onCreated()
      return
    }
    const profile = catalog?.providers.find((item) => item.provider_profile_id === type)
    updateDraft({
      provider_profile_id: type,
      display_name: profile?.display_name ?? '',
      base_url: profile?.base_url ?? '',
      protocol_family_id: null,
      protocol_adapter_id: profile?.protocol_adapter_id,
      region: profile?.connection_fields.region?.default_value,
      workspace: profile?.connection_fields.workspace?.default_value,
      account: profile?.connection_fields.account?.default_value,
      auth_mode: type === 'sn' ? 'dynamic_login' : 'api_key',
      api_key: '',
    })
  }

  const loadCatalog = async () => {
    setCatalogLoading(true)
    setCatalogError(null)
    try {
      setCatalog(await store.fetchProviderSetupCatalog())
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : 'provider_catalog_failed')
    } finally {
      setCatalogLoading(false)
    }
  }

  const keepFocusedFieldVisible = (event: FocusEvent<HTMLDivElement>) => {
    const target = event.target
    if (!(target instanceof HTMLElement)) return
    if (!target.matches('input, textarea, select')) return
    const scrollFocusedTarget = () => {
      const viewport = window.visualViewport
      const rect = target.getBoundingClientRect()
      const visibleBottom = viewport
        ? viewport.offsetTop + viewport.height - 112
        : window.innerHeight - 112
      if (rect.bottom > visibleBottom) {
        window.scrollBy({ top: rect.bottom - visibleBottom + 24, behavior: 'smooth' })
      }
      target.scrollIntoView({ block: 'center', behavior: 'smooth' })
    }
    window.setTimeout(scrollFocusedTarget, 120)
    window.setTimeout(scrollFocusedTarget, 360)
    window.setTimeout(scrollFocusedTarget, 780)
  }

  useEffect(() => {
    let cancelled = false
    void store.fetchProviderSetupCatalog()
      .then((result) => {
        if (!cancelled) setCatalog(result)
      })
      .catch((error) => {
        if (!cancelled) setCatalogError(error instanceof Error ? error.message : 'provider_catalog_failed')
      })
      .finally(() => {
        if (!cancelled) setCatalogLoading(false)
      })
    return () => { cancelled = true }
  }, [store])

  useEffect(() => {
    const viewport = window.visualViewport
    if (!viewport) return
    const updateKeyboardInset = () => {
      const inset = Math.max(0, window.innerHeight - viewport.height - viewport.offsetTop)
      setKeyboardInset(inset)
    }
    updateKeyboardInset()
    viewport.addEventListener('resize', updateKeyboardInset)
    viewport.addEventListener('scroll', updateKeyboardInset)
    return () => {
      viewport.removeEventListener('resize', updateKeyboardInset)
      viewport.removeEventListener('scroll', updateKeyboardInset)
    }
  }, [])

  return (
    <div className="flex h-full min-h-0 flex-col -mx-4 md:-mx-8 -my-4 md:-my-6">
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
      <div
        className="min-h-0 flex-1 overflow-y-auto px-4 py-6 pb-52 [scroll-padding-bottom:14rem] md:px-6 md:pb-6"
        style={keyboardInset > 0 ? {
          paddingBottom: `calc(14rem + ${keyboardInset}px)`,
          scrollPaddingBottom: `calc(14rem + ${keyboardInset}px)`,
        } : undefined}
        onFocusCapture={keepFocusedFieldVisible}
      >
        {step === 0 && (
          <StepChooseType
            selected={draft.provider_profile_id}
            onSelect={handleTypeSelect}
            hasManagedSnProvider={hasManagedSnProvider}
            catalog={catalog}
            loading={catalogLoading}
            error={catalogError}
            onRetry={() => void loadCatalog()}
          />
        )}
        {step === 1 && (
          <StepConnection draft={draft} catalog={catalog} onUpdate={updateDraft} />
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
        {createError && (
          <div
            className="mt-4 max-w-lg rounded-lg px-3 py-2 text-sm"
            style={{
              background: 'color-mix(in oklch, var(--cp-danger), transparent 90%)',
              border: '1px solid color-mix(in oklch, var(--cp-danger), transparent 60%)',
              color: 'var(--cp-danger)',
            }}
          >
            {createError}
          </div>
        )}
      </div>

      {/* Footer */}
      <div
        className="sticky bottom-0 z-10 flex shrink-0 items-center justify-between px-4 py-3 md:px-6"
        style={{
          borderTop: '1px solid var(--cp-border)',
          background: 'var(--cp-surface)',
          bottom: keyboardInset > 0 ? `${keyboardInset}px` : 'env(keyboard-inset-height, 0px)',
          paddingBottom: 'calc(0.75rem + env(safe-area-inset-bottom))',
        }}
      >
        <button
          type="button"
          onClick={handlePrev}
          className="min-h-11 rounded-lg px-4 py-2 text-sm"
          style={{ color: 'var(--cp-muted)' }}
        >
          {step === 0 ? t('aiCenter.wizard.back', 'Back') : t('aiCenter.wizard.prev', 'Previous')}
        </button>

        {step === 2 && validation && !validation.auth_valid ? (
          <button
            type="button"
            onClick={() => { setValidation(null); setStep(1) }}
            className="min-h-11 rounded-lg px-4 py-2 text-sm font-medium"
            style={{ background: 'var(--cp-warning)', color: '#fff' }}
          >
            {t('aiCenter.wizard.goBackToFix', 'Go Back to Fix')}
          </button>
        ) : (
          <button
            type="button"
            onClick={handleNext}
            disabled={!canNext() || creating}
            className="min-h-11 rounded-lg px-5 py-2 text-sm font-medium transition-opacity disabled:opacity-40"
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

function providerConnectionFieldsValid(
  draft: WizardDraft,
  profile: ProviderSetupCatalog['providers'][number] | undefined,
): boolean {
  if (!profile) return draft.provider_profile_id === 'custom'
  return (['region', 'workspace', 'account'] as const).every((name) =>
    profile.connection_fields[name]?.mode !== 'required' || Boolean(draft[name]?.trim()),
  )
}

function validationCanProceed(validation: ValidationResult): boolean {
  const blockingDetails = validation.error_details?.filter((error) => error.kind !== 'balance') ?? []
  const hasBlockingError = blockingDetails.length > 0
    || (!validation.error_details?.length && validation.errors.some((error) => !error.toLowerCase().includes('balance')))
  return !hasBlockingError && validation.base_url_reachable && validation.auth_valid
}
