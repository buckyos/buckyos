import { useEffect, useMemo, useState } from 'react'
import { Gauge } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { createQuotaApi } from '../../../../api/aicc_quota'
import { useAICCStore } from '../../hooks/use-aicc-store'
import type { QuotaQueryResponse } from '../../datamodel/quota'
import { buttonStyle } from '../quota/quota-labels'

/** Token quota near or at its limit (§9.2: 80 % warning); upstream Provider quota is shown elsewhere. */
export function QuotaHomeCard({ onOpen }: { onOpen: () => void }) {
  const { t } = useI18n()
  const store = useAICCStore()
  const api = useMemo(() => createQuotaApi(store), [store])
  const [quota, setQuota] = useState<QuotaQueryResponse | null>(null)
  useEffect(() => {
    let live = true
    api.query().then((result) => { if (live) setQuota(result) }).catch(() => undefined)
    return () => { live = false }
  }, [api])
  if (!quota || (quota.near_limit_count === 0 && quota.exhausted_count === 0)) return null
  const exhausted = quota.exhausted_count > 0
  return (
    <section
      data-testid="quota-home-card"
      className="flex flex-col gap-2 rounded-xl p-4 md:flex-row md:items-center"
      style={{ background: 'var(--cp-surface)', border: `1px solid ${exhausted ? 'var(--cp-danger)' : 'var(--cp-warning)'}` }}
    >
      <div className="flex items-center gap-2">
        <Gauge size={18} style={{ color: exhausted ? 'var(--cp-danger)' : 'var(--cp-warning)' }} />
        <div className="flex flex-col">
          <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.homeTitle', 'Token quota')}</span>
          <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.quota.warningSummary', '{{near}} near the limit, {{exhausted}} exhausted', { near: String(quota.near_limit_count), exhausted: String(quota.exhausted_count) })}
          </span>
        </div>
      </div>
      <button type="button" onClick={onOpen} className="min-h-10 rounded-lg px-4 text-sm md:ml-auto" style={buttonStyle.secondary}>
        {t('aiCenter.quota.open', 'Open quota')}
      </button>
    </section>
  )
}
