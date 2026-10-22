import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { useViewerIsAdmin } from '../../../settings/hooks/use-viewer-is-admin'
import { useQuotaApi } from '../../hooks/use-quota-api'

/** Models the inventory gained that no group can see yet (§3.4); administrators only. */
export function UnexposedModelsNotice() {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin() === true
  const api = useQuotaApi()
  const [count, setCount] = useState(0)
  useEffect(() => {
    if (!isAdmin) return
    let live = true
    api.getVisibility().then((state) => { if (live) setCount(state.unexposed_models.length) }).catch(() => undefined)
    return () => { live = false }
  }, [api, isAdmin])
  if (!isAdmin || count === 0) return null
  return (
    <div data-testid="models-unexposed" className="rounded-xl px-4 py-3 text-sm" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      {t('aiCenter.visibility.unexposedModels', '{{count}} models are not open to any group yet. Open them under AI Center › Visibility.', { count: String(count) })}
    </div>
  )
}
