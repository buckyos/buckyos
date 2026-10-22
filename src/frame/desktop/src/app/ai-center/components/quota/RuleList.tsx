import { useI18n } from '../../../../i18n/provider'
import type { QuotaRuleView } from '../../datamodel/quota'
import { KindBadge, ResetLabel, ResourceLabel, SourceBadge, StatusChip, Tokens, UsageBar } from './QuotaWidgets'
import { useObjectLabel } from './quota-labels'

export function RuleRow({ view, onOpen }: { view: QuotaRuleView; onOpen: (view: QuotaRuleView) => void }) {
  const { t } = useI18n()
  const objectLabel = useObjectLabel()
  const { rule, state } = view
  const cap = state.limit_tokens ?? state.granted_tokens ?? 0
  return (
    <button
      type="button"
      data-testid="quota-rule"
      data-rule-id={rule.rule_id}
      data-kind={rule.kind}
      data-source={rule.source}
      onClick={() => onOpen(view)}
      className="flex w-full flex-col gap-2 rounded-xl p-4 text-left transition hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', opacity: rule.enabled ? 1 : 0.7 }}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{objectLabel(rule.object)}</span>
        <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>·</span>
        <span className="text-sm" style={{ color: 'var(--cp-text)' }}><ResourceLabel resource={rule.resource} /></span>
        <span className="ml-auto flex items-center gap-1.5">
          <KindBadge kind={rule.kind} period={rule.period} />
          <SourceBadge source={rule.source} />
          <StatusChip status={state.status} />
        </span>
      </div>
      <UsageBar state={state} />
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
        <span>
          {rule.kind === 'total' ? t('aiCenter.quota.usedOfGranted', 'Used') : t('aiCenter.quota.usedOfLimit', 'Used')}{' '}
          <Tokens value={state.settled_tokens} /> / <Tokens value={cap} />
        </span>
        {state.reserved_tokens > 0 && (
          <span>{t('aiCenter.quota.reserved', 'Reserved')} <Tokens value={state.reserved_tokens} /></span>
        )}
        <span>{t('aiCenter.quota.available', 'Available')} <Tokens value={state.available_tokens} strong /></span>
        {state.debt_tokens > 0 && (
          <span style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.quota.debt', 'Over by')} <Tokens value={state.debt_tokens} /></span>
        )}
        <span className="md:ml-auto"><ResetLabel state={state} /></span>
      </div>
      {rule.pending_period && (
        <div className="text-xs" style={{ color: rule.pending_period.conflict ? 'var(--cp-danger)' : 'var(--cp-warning)' }}>
          {rule.pending_period.conflict
            ? t('aiCenter.quota.pendingConflict', 'The period change could not take effect: another rule already uses that period')
            : t('aiCenter.quota.pendingPeriod', 'Switches to a new period when the current window ends')}
        </div>
      )}
    </button>
  )
}

export function RuleList({ rules, onOpen, empty }: { rules: QuotaRuleView[]; onOpen: (view: QuotaRuleView) => void; empty: string }) {
  if (rules.length === 0) {
    return (
      <div className="rounded-xl p-6 text-center text-sm" style={{ background: 'var(--cp-surface)', border: '1px dashed var(--cp-border)', color: 'var(--cp-muted)' }}>
        {empty}
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2">
      {rules.map((view) => <RuleRow key={view.rule.rule_id} view={view} onOpen={onOpen} />)}
    </div>
  )
}
