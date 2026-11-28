import { useEffect, useState } from 'react'
import { Check } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { usePolicyApi } from '../../../hooks/use-policy-api'
import { buttonStyle, errorText } from '../../policy/policy-labels'
import { applyProviderAccess } from './providerAccess'

/** After adding a Provider: how many of its models have an AI Credit price, and a way to make it free. */
export function ProviderCreatedPanel({ provider, models, free, kept = 0, keepError, onOpenPrices, onDone }: {
  provider: string
  models: string[]
  free: boolean
  /** Usecases whose old model was kept in the Zone default, and why keeping failed. */
  kept?: number
  keepError?: string
  onOpenPrices?: () => void
  onDone: () => void
}) {
  const { t } = useI18n()
  const api = usePolicyApi()
  const [unpriced, setUnpriced] = useState<number | null>(null)
  const [isFree, setIsFree] = useState(free)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    api.getPrices()
      .then((prices) => { if (live) setUnpriced(models.filter((model) => prices.unpriced_models?.includes(model)).length) })
      .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, models])
  const makeFree = async () => {
    setBusy(true)
    setError(null)
    try {
      await applyProviderAccess(api, provider, { openTo: [], free: true })
      setIsFree(true)
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="flex max-w-lg flex-col gap-4 rounded-xl p-5" data-testid="provider-created" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <h3 className="flex items-center gap-2 text-base font-semibold" style={{ color: 'var(--cp-text)' }}><Check size={18} style={{ color: 'var(--cp-success)' }} />{t('aiCenter.policy.wizardCreated', '{{provider}} is added', { provider })}</h3>
      {unpriced !== null && (
        <p className="text-sm" data-testid="provider-price-coverage" style={{ color: 'var(--cp-text)' }}>
          {models.length === 0
            ? t('aiCenter.policy.wizardNoModelsYet', 'Its model list is not loaded yet; check the price list once it is.')
            : t('aiCenter.policy.wizardCoverage', '{{priced}} of its {{total}} models have an AI Credit price, {{unpriced}} do not.', { priced: models.length - unpriced, total: models.length, unpriced })}
        </p>
      )}
      <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {isFree
          ? t('aiCenter.policy.wizardIsFree', 'It is free for everyone who can use it.')
          : t('aiCenter.policy.wizardUnpricedHint', 'Users who pay (a coefficient other than 0) cannot use models without a price. Price them, or make the Provider free.')}
      </p>
      {kept > 0 && <p className="text-xs" data-testid="provider-kept-models" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.wizard.coverage.kept', 'Usecases that keep their previous model in the Zone default: {{count}}', { count: kept })}</p>}
      {keepError && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.wizard.coverage.keepFailed', 'Could not keep every previous model: {{error}}. Set them on the Usecases page.', { error: keepError })}</p>}
      {error && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:flex-wrap sm:justify-end">
        {!isFree && <button type="button" data-testid="provider-make-free" disabled={busy} onClick={() => void makeFree()} className="min-h-11 rounded-lg px-4 text-sm disabled:opacity-50" style={buttonStyle.secondary}>{t('aiCenter.policy.wizardMakeFree', 'Make it free')}</button>}
        {onOpenPrices && <button type="button" onClick={onOpenPrices} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('aiCenter.prices.open', 'Open the price list')}</button>}
        <button type="button" data-testid="provider-created-done" onClick={onDone} className="min-h-11 rounded-lg px-4 text-sm font-medium" style={buttonStyle.primary}>{t('aiCenter.policy.wizardDone', 'Done')}</button>
      </div>
    </div>
  )
}
