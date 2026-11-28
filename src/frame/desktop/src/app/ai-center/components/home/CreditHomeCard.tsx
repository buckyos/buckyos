import { Coins } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { isNegativeCredit, isPositiveCredit } from '../../datamodel/policy'
import { usePolicyStatus } from '../../hooks/use-policy-api'
import { CreditAmount } from '../policy/PolicyWidgets'
import { buttonStyle } from '../policy/policy-labels'

/** The viewer's AI Credit balance and their limits near or at the limit (80 % warning, `policy.status`).
 * Provider account quota and balances are separate and shown with the Providers and the usage cards. */
export function CreditHomeCard({ onOpenCredit, onOpenPolicies }: { onOpenCredit: () => void; onOpenPolicies: () => void }) {
  const { t } = useI18n()
  const status = usePolicyStatus()
  if (!status) return null
  const warnings = status.near_limit_count + status.exhausted_count
  const empty = !isPositiveCredit(status.balance)
  const tone = status.exhausted_count > 0 || isNegativeCredit(status.balance) ? 'var(--cp-danger)' : warnings > 0 ? 'var(--cp-warning)' : 'var(--cp-border)'
  return (
    <section
      data-testid="credit-home-card"
      data-warnings={warnings}
      className="flex flex-col gap-3 rounded-xl p-4 md:flex-row md:items-center"
      style={{ background: 'var(--cp-surface)', border: `1px solid ${tone}` }}
    >
      <div className="flex min-w-0 items-center gap-3">
        <Coins size={18} className="shrink-0" style={{ color: tone === 'var(--cp-border)' ? 'var(--cp-muted)' : tone }} />
        <div className="flex min-w-0 flex-col">
          <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
            {t('aiCenter.home.myCredit', 'My AI Credit')}: <CreditAmount value={status.balance} strong testId="credit-home-balance" />
          </span>
          <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {warnings > 0
              ? t('aiCenter.home.limitWarnings', '{{near}} limits near the limit, {{exhausted}} used up', { near: status.near_limit_count, exhausted: status.exhausted_count })
              : empty
                ? t('aiCenter.home.creditEmpty', 'Priced models need AI Credit; free models do not.')
                : t('aiCenter.home.limitsOk', 'No limit is near its end.')}
          </span>
        </div>
      </div>
      <div className="flex gap-2 md:ml-auto">
        {warnings > 0 && <button type="button" onClick={onOpenPolicies} className="min-h-10 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('aiCenter.home.openPolicies', 'View limits')}</button>}
        <button type="button" onClick={onOpenCredit} className="min-h-10 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('aiCenter.home.openCredit', 'AI Credit')}</button>
      </div>
    </section>
  )
}
