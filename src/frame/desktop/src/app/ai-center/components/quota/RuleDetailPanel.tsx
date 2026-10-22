import { useCallback, useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { newRechargeId, type QuotaApi } from '../../../../api/aicc_quota'
import { formatTokensExact, type QuotaRuleDetail, type QuotaRuleView } from '../../datamodel/quota'
import { ConfirmDialog } from '../shared/ConfirmDialog'
import {
  Field,
  KindBadge,
  ResetLabel,
  ResourceLabel,
  SidePanel,
  SourceBadge,
  StatusChip,
  Tokens,
  TokenInput,
  UsageBar,
} from './QuotaWidgets'
import { buttonStyle, formatLocal, useObjectLabel } from './quota-labels'

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function RechargeDialog({ api, detail, onClose, onDone }: { api: QuotaApi; detail: QuotaRuleDetail; onClose: () => void; onDone: () => void }) {
  const { t } = useI18n()
  const account = detail.account!
  const [rechargeId] = useState(newRechargeId)
  const [amount, setAmount] = useState<number | null>(null)
  const [note, setNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<{ replayed: boolean } | null>(null)
  const afterGranted = account.granted_tokens + (amount ?? 0)
  const afterAvailable = Math.max(0, afterGranted - account.settled_tokens - account.reserved_tokens)
  const submit = async () => {
    if (!amount) return
    setBusy(true)
    setError(null)
    try {
      const response = await api.recharge({ account_id: account.account_id, recharge_id: rechargeId, amount_tokens: amount, ...(note.trim() ? { note: note.trim() } : {}) })
      setResult({ replayed: response.replayed })
      onDone()
    } catch (reason) {
      // The same recharge id is kept: a retry after a lost response credits once.
      setError(errorText(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center" data-testid="quota-recharge">
      <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={busy ? undefined : onClose} />
      <div role="dialog" aria-label={t('aiCenter.quota.rechargeTitle', 'Recharge total quota')} className="relative max-h-[calc(100dvh-1rem)] w-full overflow-y-auto rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-md md:rounded-xl" style={{ background: 'var(--cp-surface)' }}>
        <h3 className="mb-3 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.rechargeTitle', 'Recharge total quota')}</h3>
        <div className="mb-4 grid grid-cols-2 gap-3">
          <Field label={t('aiCenter.quota.granted', 'Granted')}><Tokens value={account.granted_tokens} /></Field>
          <Field label={t('aiCenter.quota.available', 'Available')}><Tokens value={account.available_tokens} /></Field>
          <Field label={t('aiCenter.quota.settled', 'Settled')}><Tokens value={account.settled_tokens} /></Field>
          <Field label={t('aiCenter.quota.reserved', 'Reserved')}><Tokens value={account.reserved_tokens} /></Field>
        </div>
        {result ? (
          <p className="mb-4 text-sm" data-testid="quota-recharge-done" style={{ color: 'var(--cp-success)' }}>
            {result.replayed ? t('aiCenter.quota.rechargeReplayed', 'This recharge had already been credited; nothing was added twice.') : t('aiCenter.quota.rechargeDone', 'Recharged. The balance is updated.')}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            <TokenInput label={t('aiCenter.quota.rechargeAmount', 'Tokens to add')} value={amount} onChange={setAmount} allowZero={false} testId="quota-recharge-amount" />
            <label className="flex flex-col gap-1 text-sm">
              <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.note', 'Note (optional)')}</span>
              <input value={note} onChange={(event) => setNote(event.target.value)} maxLength={500} className="min-h-10 rounded-lg px-3" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }} />
            </label>
            {amount !== null && (
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {t('aiCenter.quota.rechargePreview', 'Granted becomes {{granted}}, available about {{available}}. Debt is paid first; other limits may still block.', { granted: formatTokensExact(afterGranted), available: formatTokensExact(afterAvailable) })}
              </p>
            )}
          </div>
        )}
        {error && <p className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
        <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onClose} disabled={busy} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>
            {result ? t('common.close', 'Close') : t('common.cancel', 'Cancel')}
          </button>
          {!result && (
            <button type="button" data-testid="quota-recharge-submit" disabled={busy || !amount} onClick={() => void submit()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>
              {busy ? t('aiCenter.quota.recharging', 'Recharging…') : error ? t('aiCenter.quota.rechargeRetry', 'Retry the same recharge') : t('aiCenter.quota.recharge', 'Recharge')}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

interface DetailProps {
  api: QuotaApi
  view: QuotaRuleView
  isAdmin: boolean
  onClose: () => void
  onEdit: (view: QuotaRuleView) => void
  onChanged: () => void
}

export function RuleDetailPanel({ api, view, isAdmin, onClose, onEdit, onChanged }: DetailProps) {
  const { t } = useI18n()
  const objectLabel = useObjectLabel()
  const [detail, setDetail] = useState<QuotaRuleDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [recharging, setRecharging] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)

  const load = useCallback(() => {
    api.getRule(view.rule.rule_id).then(setDetail).catch((reason: unknown) => setError(errorText(reason)))
  }, [api, view.rule.rule_id])
  useEffect(load, [load])

  const current = detail?.rule ?? view
  const { rule, state } = current
  const remove = async () => {
    setDeleting(true)
    setDeleteError(null)
    try {
      await api.deleteRule(rule.rule_id, rule.version)
      onChanged()
      onClose()
    } catch (reason) {
      setDeleteError(errorText(reason))
    } finally {
      setDeleting(false)
    }
  }

  return (
    <SidePanel title={t('aiCenter.quota.detailTitle', 'Token quota')} onClose={onClose} testId="quota-detail">
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{objectLabel(rule.object)}</span>
            <KindBadge kind={rule.kind} period={rule.period} />
            <SourceBadge source={rule.source} />
            <StatusChip status={state.status} />
          </div>
          <span className="text-sm" style={{ color: 'var(--cp-text)' }}><ResourceLabel resource={rule.resource} /></span>
          {rule.object.type === 'app' && (
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.sharedHint', 'Shared by every user of this App in the Zone; per-user details are not shown.')}</span>
          )}
        </div>
        <section className="flex flex-col gap-3 rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <UsageBar state={state} />
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            {rule.kind === 'total'
              ? <Field label={t('aiCenter.quota.granted', 'Granted')}><Tokens value={state.granted_tokens ?? 0} /></Field>
              : <Field label={t('aiCenter.quota.limit', 'Token limit per period')}><Tokens value={state.limit_tokens ?? 0} /></Field>}
            <Field label={t('aiCenter.quota.settled', 'Settled')}><Tokens value={state.settled_tokens} /></Field>
            <Field label={t('aiCenter.quota.reserved', 'Reserved')}><Tokens value={state.reserved_tokens} /></Field>
            <Field label={t('aiCenter.quota.pending', 'Awaiting reconciliation')}><Tokens value={state.pending_tokens} /></Field>
            <Field label={t('aiCenter.quota.available', 'Available')}><span data-testid="quota-available"><Tokens value={state.available_tokens} strong /></span></Field>
            {state.debt_tokens > 0 && <Field label={t('aiCenter.quota.debt', 'Over by')}><Tokens value={state.debt_tokens} /></Field>}
          </div>
          {rule.kind === 'periodic' && state.window_start_ms !== undefined && state.window_end_ms !== undefined && (
            <Field label={t('aiCenter.quota.window', 'Current window')}>{formatLocal(state.window_start_ms)} – {formatLocal(state.window_end_ms)}</Field>
          )}
          {rule.kind === 'total' && state.account_start_ms !== undefined && (
            <Field label={t('aiCenter.quota.accountStart', 'Counting since')}>{formatLocal(state.account_start_ms)}</Field>
          )}
          <span className="text-xs"><ResetLabel state={state} /></span>
          {state.status === 'exhausted' && rule.kind === 'total' && (
            <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.quota.totalExhausted', 'Used up: an administrator has to recharge it.')}</p>
          )}
        </section>

        {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}

        {detail && detail.constraints.length > 0 && (
          <section className="flex flex-col gap-2" data-testid="quota-constraints">
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.constraints', 'All limits on the same calls')}</h3>
            {detail.constraints.map((item) => {
              const blocking = detail.blocking_rule_ids.includes(item.rule.rule_id)
              return (
                <div key={item.rule.rule_id} data-blocking={blocking} className="flex flex-wrap items-center gap-2 rounded-lg px-3 py-2 text-xs" style={{ background: 'var(--cp-surface)', border: `1px solid ${blocking ? 'var(--cp-danger)' : 'var(--cp-border)'}`, color: 'var(--cp-text)' }}>
                  <span className="truncate">{objectLabel(item.rule.object)} · <ResourceLabel resource={item.rule.resource} /></span>
                  <KindBadge kind={item.rule.kind} period={item.rule.period} />
                  <span className="ml-auto">{t('aiCenter.quota.available', 'Available')} <Tokens value={item.state.available_tokens} /></span>
                  {blocking && <span style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.quota.blocking', 'Blocking now')}</span>}
                </div>
              )
            })}
          </section>
        )}

        {detail?.grants && detail.grants.length > 0 && (
          <section className="flex flex-col gap-1">
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.grants', 'Grants and recharges')}</h3>
            {detail.grants.map((grant) => (
              <div key={grant.grant_id} className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-text)' }}>
                <span>{grant.kind === 'initial' ? t('aiCenter.quota.initialGrant', 'Initial grant') : t('aiCenter.quota.recharge', 'Recharge')}</span>
                <span>+<Tokens value={grant.amount_tokens} /></span>
                <span style={{ color: 'var(--cp-muted)' }}>{grant.operator} · {formatLocal(grant.created_at_ms)}</span>
                {grant.note && <span className="truncate" style={{ color: 'var(--cp-muted)' }}>{grant.note}</span>}
              </div>
            ))}
          </section>
        )}

        {detail?.audit && detail.audit.length > 0 && (
          <section className="flex flex-col gap-1">
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.history', 'Changes')}</h3>
            {detail.audit.slice(0, 20).map((item) => (
              <div key={item.audit_id} className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {formatLocal(item.created_at_ms)} · {item.operator} · {item.action}
              </div>
            ))}
          </section>
        )}

        <div className="flex flex-col gap-2 sm:flex-row sm:flex-wrap">
          {current.editable && (
            <button type="button" data-testid="quota-edit" onClick={() => onEdit(current)} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('aiCenter.quota.edit', 'Edit')}</button>
          )}
          {isAdmin && rule.kind === 'total' && detail?.account && (
            <button type="button" data-testid="quota-recharge-open" onClick={() => setRecharging(true)} className="min-h-11 rounded-lg px-4 text-sm font-medium" style={buttonStyle.primary}>{t('aiCenter.quota.recharge', 'Recharge')}</button>
          )}
          {current.editable && (
            <button type="button" data-testid="quota-delete" onClick={() => setConfirmDelete(true)} className="min-h-11 rounded-lg px-4 text-sm sm:ml-auto" style={buttonStyle.danger}>{t('aiCenter.quota.delete', 'Delete')}</button>
          )}
        </div>
        {!current.editable && (
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {rule.source === 'admin' ? t('aiCenter.quota.readOnlyAdmin', 'Set by an administrator; it applies to you and cannot be changed here.') : t('aiCenter.quota.readOnlyUser', "A user's own limit; only that user changes it.")}
          </p>
        )}
      </div>
      {recharging && detail && (
        <RechargeDialog api={api} detail={detail} onClose={() => setRecharging(false)} onDone={() => { load(); onChanged() }} />
      )}
      <ConfirmDialog
        open={confirmDelete}
        title={t('aiCenter.quota.deleteTitle', 'Delete this limit?')}
        message={rule.kind === 'total'
          ? t('aiCenter.quota.deleteTotal', 'The rule stops blocking. The account keeps its ledger; a new rule on the same scope restores it without a new grant.')
          : t('aiCenter.quota.deletePeriodic', 'Only this limit is removed; usage history and other limits stay.')}
        confirmLabel={t('aiCenter.quota.delete', 'Delete')}
        confirming={deleting}
        error={deleteError}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmDelete(false)}
      />
    </SidePanel>
  )
}

