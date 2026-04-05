import { Activity, CreditCard, DollarSign, Wallet } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAIStatus, useProviders, useUsageSummary } from '../../hooks/use-mock-store'
import { SummaryCard } from '../shared/SummaryCard'

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
}

export function UsageDashboard() {
  const { t } = useI18n()
  const status = useAIStatus()
  const providers = useProviders()
  const summary = useUsageSummary()

  const snProvider = providers.find((p) => p.config.provider_type === 'sn_router')
  const snCredit = snProvider?.account.balance_value

  const balanceProviders = providers.filter((p) => p.account.balance_supported && p.account.balance_value != null)

  const balanceSubtitle = balanceProviders
    .map((p) => {
      const unit = p.account.balance_unit === 'usd' ? '$' : ''
      const suffix = p.account.balance_unit === 'credit' ? ' Credit' : ''
      return `${p.config.name}: ${unit}${p.account.balance_value}${suffix}`
    })
    .join(' · ')

  return (
    <div className="flex flex-col gap-6">
      {/* Summary Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <SummaryCard
          icon={<Activity size={18} />}
          title={t('aiCenter.home.status', 'AI Status')}
          value={status.state === 'disabled'
            ? t('aiCenter.home.disabled', 'Disabled')
            : t('aiCenter.home.enabled', 'Enabled')}
          subtitle={`${status.provider_count} Providers · ${status.model_count} Models`}
        />
        <SummaryCard
          icon={<CreditCard size={18} />}
          title={t('aiCenter.home.credit', 'SN Credit')}
          value={snCredit != null ? `${snCredit} Credit` : '—'}
          subtitle={snProvider ? snProvider.config.name : undefined}
        />
        <SummaryCard
          icon={<DollarSign size={18} />}
          title={t('aiCenter.home.estimatedCost', 'Est. Cost')}
          value={`$${summary.total_estimated_cost.toFixed(2)}`}
          subtitle="Estimated"
        />
        <SummaryCard
          icon={<Wallet size={18} />}
          title={t('aiCenter.home.balanceOverview', 'Balance Overview')}
          value={`${balanceProviders.length} Providers`}
          subtitle={balanceSubtitle || undefined}
        />
      </div>

      {/* Usage Trend Placeholder (Phase 2) */}
      <div
        className="rounded-xl p-4"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <h3
          className="text-sm font-medium mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          {t('aiCenter.home.trendTitle', 'Usage Trend')}
        </h3>
        <div
          className="flex items-center justify-center rounded-lg"
          style={{
            background: 'var(--cp-bg)',
            minHeight: 220,
            color: 'var(--cp-muted)',
          }}
        >
          <span className="text-sm">{t('aiCenter.home.chartPlaceholder', 'Chart coming in Phase 2')}</span>
        </div>
      </div>

      {/* Usage Summary */}
      <div
        className="rounded-xl p-4"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <h3
          className="text-sm font-medium mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          {t('aiCenter.home.usageSummary', 'Usage Summary')}
        </h3>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.home.today', 'Today')}
            </div>
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {formatTokens(summary.today_tokens)} tokens
            </div>
          </div>
          <div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.home.thisMonth', 'This Month')}
            </div>
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {formatTokens(summary.this_month_tokens)} tokens
            </div>
          </div>
          <div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.home.total', 'Total')}
            </div>
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {formatTokens(summary.total_tokens)} tokens
            </div>
          </div>
          <div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.home.totalCost', 'Total Est. Cost')}
            </div>
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              ${summary.total_estimated_cost.toFixed(2)}
            </div>
          </div>
        </div>
      </div>

      {/* Category Breakdown Placeholder (Phase 2) */}
      <div
        className="rounded-xl p-4"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <h3
          className="text-sm font-medium mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          {t('aiCenter.home.categoryTitle', 'Category Breakdown')}
        </h3>
        <div
          className="flex items-center justify-center rounded-lg"
          style={{
            background: 'var(--cp-bg)',
            minHeight: 180,
            color: 'var(--cp-muted)',
          }}
        >
          <span className="text-sm">{t('aiCenter.home.chartPlaceholder', 'Chart coming in Phase 2')}</span>
        </div>
      </div>
    </div>
  )
}
