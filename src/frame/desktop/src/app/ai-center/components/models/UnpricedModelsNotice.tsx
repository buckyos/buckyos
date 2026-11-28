import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { useViewerIsAdmin } from '../../../settings/hooks/use-viewer-is-admin'
import { usePolicyApi } from '../../hooks/use-policy-api'
import { buttonStyle } from '../policy/policy-labels'

/** Models of the inventory without any AI Credit price (§3.3): users who pay cannot use them; administrators only. */
export function UnpricedModelsNotice({ onOpenPrices }: { onOpenPrices?: () => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin() === true
  const api = usePolicyApi()
  const [count, setCount] = useState(0)
  useEffect(() => {
    if (!isAdmin) return
    let live = true
    api.getPrices().then((prices) => { if (live) setCount(prices.unpriced_models?.length ?? 0) }).catch(() => undefined)
    return () => { live = false }
  }, [api, isAdmin])
  if (!isAdmin || count === 0) return null
  return (
    <div data-testid="models-unpriced" className="flex flex-col gap-2 rounded-xl px-4 py-3 text-sm md:flex-row md:items-center" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      <span>{t('aiCenter.prices.unpricedNotice', '{{count}} models in the model list have no price: users who pay (coefficient not 0) cannot use them.', { count })}</span>
      {onOpenPrices && <button type="button" onClick={onOpenPrices} className="min-h-9 shrink-0 whitespace-nowrap rounded-lg px-3 text-sm md:ml-auto" style={buttonStyle.secondary}>{t('aiCenter.prices.open', 'Open the price list')}</button>}
    </div>
  )
}
