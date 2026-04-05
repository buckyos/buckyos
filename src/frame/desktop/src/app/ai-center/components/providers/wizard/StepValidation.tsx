import { useEffect, useState } from 'react'
import { Check, X, AlertTriangle, Loader2 } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { useMockStore } from '../../../hooks/use-mock-store'
import type { ValidationResult, WizardDraft } from '../../../mock/types'

interface StepValidationProps {
  draft: WizardDraft
  onResult: (result: ValidationResult) => void
}

type CheckItem = {
  key: string
  label: string
  status: 'pending' | 'checking' | 'ok' | 'warning' | 'error'
  detail?: string
}

export function StepValidation({ draft, onResult }: StepValidationProps) {
  const { t } = useI18n()
  const store = useMockStore()
  const [checks, setChecks] = useState<CheckItem[]>([
    { key: 'endpoint', label: t('aiCenter.wizard.checkEndpoint', 'Checking endpoint connectivity...'), status: 'pending' },
    { key: 'auth', label: t('aiCenter.wizard.checkAuth', 'Verifying authentication...'), status: 'pending' },
    { key: 'models', label: t('aiCenter.wizard.checkModels', 'Discovering available models...'), status: 'pending' },
    { key: 'balance', label: t('aiCenter.wizard.checkBalance', 'Checking balance capability...'), status: 'pending' },
  ])
  const [models, setModels] = useState<string[]>([])

  useEffect(() => {
    const result = store.validateConnection(draft)
    let cancelled = false

    const steps = [
      {
        key: 'endpoint',
        delay: 400,
        update: (): CheckItem => ({
          key: 'endpoint',
          label: result.endpoint_reachable
            ? t('aiCenter.wizard.endpointOk', 'Endpoint reachable')
            : t('aiCenter.wizard.endpointFail', 'Endpoint not reachable'),
          status: result.endpoint_reachable ? 'ok' as const : 'error' as const,
        }),
      },
      {
        key: 'auth',
        delay: 800,
        update: (): CheckItem => ({
          key: 'auth',
          label: result.auth_valid
            ? t('aiCenter.wizard.authOk', 'Authentication valid')
            : t('aiCenter.wizard.authFail', 'Authentication failed'),
          status: result.auth_valid ? 'ok' as const : 'error' as const,
        }),
      },
      {
        key: 'models',
        delay: 1200,
        update: (): CheckItem => ({
          key: 'models',
          label: t('aiCenter.wizard.modelsFound', '{{count}} models discovered', { count: result.models_discovered.length }),
          status: result.models_discovered.length > 0 ? 'ok' as const : 'warning' as const,
        }),
      },
      {
        key: 'balance',
        delay: 1500,
        update: (): CheckItem => ({
          key: 'balance',
          label: result.balance_available
            ? t('aiCenter.wizard.balanceOk', 'Balance query available')
            : t('aiCenter.wizard.balanceUnavailable', 'Balance query not available'),
          status: result.balance_available ? 'ok' as const : 'warning' as const,
        }),
      },
    ]

    steps.forEach((step, i) => {
      setTimeout(() => {
        if (cancelled) return
        setChecks((prev) => {
          const next = [...prev]
          next[i] = step.update()
          // Mark next item as checking
          if (i + 1 < next.length && next[i + 1].status === 'pending') {
            next[i + 1] = { ...next[i + 1], status: 'checking' }
          }
          return next
        })
        if (step.key === 'models') {
          setModels(result.models_discovered)
        }
        if (i === steps.length - 1) {
          onResult(result)
        }
      }, step.delay)
    })

    // Start first item as checking
    setChecks((prev) => {
      const next = [...prev]
      next[0] = { ...next[0], status: 'checking' }
      return next
    })

    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const statusIcon = (status: CheckItem['status']) => {
    switch (status) {
      case 'pending': return <div className="w-4 h-4 rounded-full" style={{ border: '2px solid var(--cp-border)' }} />
      case 'checking': return <Loader2 size={16} className="animate-spin" style={{ color: 'var(--cp-accent)' }} />
      case 'ok': return <Check size={16} style={{ color: 'var(--cp-success)' }} />
      case 'warning': return <AlertTriangle size={16} style={{ color: 'var(--cp-warning)' }} />
      case 'error': return <X size={16} style={{ color: 'var(--cp-danger)' }} />
    }
  }

  return (
    <div className="flex flex-col gap-4 max-w-lg">
      {checks.map((check) => (
        <div key={check.key} className="flex items-center gap-3">
          {statusIcon(check.status)}
          <span
            className="text-sm"
            style={{ color: check.status === 'pending' ? 'var(--cp-muted)' : 'var(--cp-text)' }}
          >
            {check.label}
          </span>
        </div>
      ))}

      {models.length > 0 && (
        <div
          className="rounded-lg p-3 mt-2 max-h-48 overflow-y-auto"
          style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
        >
          <div className="text-xs font-medium mb-2" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.providers.models', 'Models')}
          </div>
          {models.slice(0, 10).map((m) => (
            <div key={m} className="text-xs py-0.5 font-mono" style={{ color: 'var(--cp-text)' }}>
              {m}
            </div>
          ))}
          {models.length > 10 && (
            <div className="text-xs pt-1" style={{ color: 'var(--cp-muted)' }}>
              +{models.length - 10} more
            </div>
          )}
        </div>
      )}
    </div>
  )
}
