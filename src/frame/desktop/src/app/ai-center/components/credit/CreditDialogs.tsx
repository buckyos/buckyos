import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { PolicyApi } from '../../../../api/aicc_policy'
import {
  addCredit,
  newEntryId,
  parsePositiveCredit,
  parseSignedCredit,
  type CreditAccount,
  type CreditLedgerItem,
  type CreditLedgerListResponse,
} from '../../datamodel/policy'
import { CreditAmount, ExactModelLabel, Field, Modal, SidePanel, StatusDot } from '../policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, formatLocal } from '../policy/policy-labels'

interface EntryDialogProps {
  api: PolicyApi
  account: CreditAccount
  onClose: () => void
  onDone: (account: CreditAccount) => void
}

/** Recharge (positive) or adjustment (signed, with a note). The id is generated once per dialog, so a retry
 * after a lost response applies once (§3.4). */
function EntryDialog({ api, account, onClose, onDone, kind }: EntryDialogProps & { kind: 'recharge' | 'adjust' }) {
  const { t } = useI18n()
  const [entryId] = useState(() => newEntryId(kind === 'recharge' ? 'rc' : 'adj'))
  const [text, setText] = useState('')
  const [note, setNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<{ replayed: boolean; account: CreditAccount } | null>(null)
  const amount = kind === 'recharge' ? parsePositiveCredit(text) : parseSignedCredit(text)
  const after = amount ? addCredit(account.balance, amount) : null
  const canSubmit = !!amount && (kind === 'recharge' || note.trim().length > 0)
  const submit = async () => {
    if (!amount) return
    setBusy(true)
    setError(null)
    try {
      const response = kind === 'recharge'
        ? await api.recharge({ user_id: account.user_id, recharge_id: entryId, amount, ...(note.trim() ? { note: note.trim() } : {}) })
        : await api.adjust({ user_id: account.user_id, adjust_id: entryId, amount, note: note.trim() })
      setResult({ replayed: response.replayed, account: response.account })
      onDone(response.account)
    } catch (reason) {
      // The same id is kept: retrying after a lost response applies once.
      setError(errorText(reason))
    } finally {
      setBusy(false)
    }
  }
  const title = kind === 'recharge'
    ? t('aiCenter.credit.rechargeTitle', 'Recharge AI Credit for {{user}}', { user: account.user_id })
    : t('aiCenter.credit.adjustTitle', 'Adjust AI Credit of {{user}}', { user: account.user_id })
  return (
    <Modal title={title} onClose={onClose} busy={busy} testId={kind === 'recharge' ? 'credit-recharge' : 'credit-adjust'}>
      <div className="mb-4 grid grid-cols-2 gap-3">
        <Field label={t('aiCenter.credit.balance', 'AI Credit balance')}><CreditAmount value={result?.account.balance ?? account.balance} strong testId="credit-dialog-balance" /></Field>
        <Field label={kind === 'recharge' ? t('aiCenter.credit.rechargeId', 'Recharge ID') : t('aiCenter.credit.adjustId', 'Adjustment ID')}><span className="break-all text-xs" data-testid="credit-entry-id" style={{ color: 'var(--cp-muted)' }}>{entryId}</span></Field>
      </div>
      {result ? (
        <p className="mb-2 text-sm" data-testid="credit-entry-done" style={{ color: 'var(--cp-success)' }}>
          {result.replayed
            ? t('aiCenter.credit.replayed', 'This had already been applied; nothing was added twice.')
            : kind === 'recharge' ? t('aiCenter.credit.recharged', 'Recharged. The balance is updated.') : t('aiCenter.credit.adjusted', 'Adjusted. The balance is updated.')}
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          <label className="flex flex-col gap-1 text-sm">
            <span style={{ color: 'var(--cp-text)' }}>{kind === 'recharge' ? t('aiCenter.credit.amount', 'AI Credit to add') : t('aiCenter.credit.adjustAmount', 'Change (negative to deduct)')}</span>
            <input data-testid="credit-amount" value={text} inputMode="decimal" onChange={(event) => setText(event.target.value)} placeholder={kind === 'recharge' ? '100' : '-2.5'} className="min-h-10 rounded-lg px-3" style={{ ...fieldStyle, borderColor: text.trim() && !amount ? 'var(--cp-danger)' : 'var(--cp-border)' }} />
            {text.trim() && !amount && <span className="text-xs" style={{ color: 'var(--cp-danger)' }}>{kind === 'recharge' ? t('aiCenter.credit.amountInvalid', 'Enter a positive amount with up to six decimals.') : t('aiCenter.credit.adjustInvalid', 'Enter a non-zero amount with up to six decimals.')}</span>}
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span style={{ color: 'var(--cp-text)' }}>{kind === 'recharge' ? t('aiCenter.credit.note', 'Note (optional)') : t('aiCenter.credit.noteRequired', 'Reason (required)')}</span>
            <input data-testid="credit-note" value={note} onChange={(event) => setNote(event.target.value)} maxLength={500} className="min-h-10 rounded-lg px-3" style={fieldStyle} />
          </label>
          {after && (
            <p className="text-xs" data-testid="credit-after" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.credit.balanceAfter', 'Balance after')} <CreditAmount value={after} />
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
          <button type="button" data-testid="credit-submit" disabled={busy || !canSubmit} onClick={() => void submit()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>
            {busy
              ? t('aiCenter.credit.applying', 'Applying…')
              : error
                ? t('aiCenter.credit.retry', 'Retry with the same ID')
                : kind === 'recharge' ? t('aiCenter.credit.recharge', 'Recharge') : t('aiCenter.credit.adjust', 'Adjust')}
          </button>
        )}
      </div>
    </Modal>
  )
}

export function RechargeDialog(props: EntryDialogProps) {
  return <EntryDialog {...props} kind="recharge" />
}

export function AdjustDialog(props: EntryDialogProps) {
  return <EntryDialog {...props} kind="adjust" />
}

function LedgerRow({ item }: { item: CreditLedgerItem }) {
  const { t, locale } = useI18n()
  if (item.type === 'entry') {
    return (
      <div data-testid="ledger-item" data-type={item.kind} className="flex flex-col gap-0.5 py-2 text-xs" style={{ borderTop: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{item.kind === 'recharge' ? t('aiCenter.credit.kind.recharge', 'Recharge') : t('aiCenter.credit.kind.adjust', 'Adjustment')}</span>
          <span className="ml-auto font-medium"><CreditAmount value={item.amount.startsWith('-') ? item.amount : `+${item.amount}`} /></span>
        </div>
        <div className="flex flex-wrap gap-x-3" style={{ color: 'var(--cp-muted)' }}>
          <span>{formatLocal(item.created_at_ms, locale)}</span>
          <span>{item.operator}</span>
          <span>{t('aiCenter.credit.balanceAfter', 'Balance after')} <CreditAmount value={item.balance_after} /></span>
          {item.note && <span className="break-all">{item.note}</span>}
        </div>
      </div>
    )
  }
  const statusLabel = {
    charged: t('aiCenter.credit.status.charged', 'Charged'),
    free: t('aiCenter.credit.status.free', 'Free'),
    unpriced: t('aiCenter.credit.status.unpriced', 'Not priced, nothing charged'),
    unknown_usage: t('aiCenter.credit.status.unknown', 'Outcome unknown, nothing charged yet'),
  }[item.status]
  return (
    <div data-testid="ledger-item" data-type="charge" data-status={item.status} className="flex flex-col gap-0.5 py-2 text-xs" style={{ borderTop: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 font-medium"><ExactModelLabel exactModel={item.exact_model} /></span>
        <span className="ml-auto font-medium"><CreditAmount value={item.amount === '0' ? '0' : `-${item.amount}`} /></span>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1" style={{ color: 'var(--cp-muted)' }}>
        <span>{formatLocal(item.settled_at_ms, locale)}</span>
        {item.app_id && <span>{item.app_id}</span>}
        <span>{t('aiCenter.credit.tokens', '{{count}} tokens', { count: item.tokens.toLocaleString(locale) })}</span>
        <StatusDot tone={item.status === 'charged' ? 'ok' : item.status === 'free' ? 'muted' : 'warning'} label={statusLabel} />
      </div>
    </div>
  )
}

/** Recharges, adjustments and charges of one user, newest first. */
export function LedgerList({ api, userId, generation = 0 }: { api: PolicyApi; userId?: string; generation?: number }) {
  const { t } = useI18n()
  const [page, setPage] = useState<{ key: string; value: CreditLedgerListResponse } | null>(null)
  const [more, setMore] = useState<CreditLedgerItem[]>([])
  const [cursor, setCursor] = useState<string | undefined>(undefined)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const key = `${userId ?? ''}\u0000${generation}`
  useEffect(() => {
    let live = true
    api.ledger(userId, undefined, 30)
      .then((value) => { if (live) { setPage({ key, value }); setMore([]); setCursor(value.next_cursor); setError(null) } })
      .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, key, userId])
  const loadMore = async () => {
    if (!cursor) return
    setLoading(true)
    try {
      const next = await api.ledger(userId, cursor, 30)
      setMore((items) => [...items, ...next.items])
      setCursor(next.next_cursor)
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setLoading(false)
    }
  }
  const current = page?.key === key ? page.value : null
  const items = [...(current?.items ?? []), ...more]
  return (
    <div className="flex flex-col" data-testid="credit-ledger">
      {error && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      {!current && !error && <p role="status" className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('common.loading', 'Loading...')}</p>}
      {current && items.length === 0 && <p className="py-4 text-center text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.credit.ledgerEmpty', 'No recharge or charge yet.')}</p>}
      {items.map((item) => <LedgerRow key={item.type === 'entry' ? item.entry_id : item.attempt_id} item={item} />)}
      {cursor && (
        <button type="button" disabled={loading} onClick={() => void loadMore()} className="mt-2 min-h-10 rounded-lg text-sm disabled:opacity-50" style={buttonStyle.secondary}>
          {loading ? t('common.loading', 'Loading...') : t('common.loadMore', 'Load more')}
        </button>
      )}
    </div>
  )
}

export function LedgerPanel({ api, account, onClose, onRecharge, onAdjust, generation }: { api: PolicyApi; account: CreditAccount; onClose: () => void; onRecharge?: () => void; onAdjust?: () => void; generation: number }) {
  const { t } = useI18n()
  return (
    <SidePanel title={t('aiCenter.credit.ledgerTitle', 'AI Credit of {{user}}', { user: account.user_id })} onClose={onClose} testId="credit-ledger-panel">
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-end gap-4">
          <Field label={t('aiCenter.credit.balance', 'AI Credit balance')}><span className="text-2xl"><CreditAmount value={account.balance} strong /></span></Field>
          <div className="ml-auto flex gap-2">
            {onAdjust && <button type="button" onClick={onAdjust} className="min-h-10 rounded-lg px-3 text-sm" style={buttonStyle.secondary}>{t('aiCenter.credit.adjust', 'Adjust')}</button>}
            {onRecharge && <button type="button" onClick={onRecharge} className="min-h-10 rounded-lg px-3 text-sm font-medium" style={buttonStyle.primary}>{t('aiCenter.credit.recharge', 'Recharge')}</button>}
          </div>
        </div>
        <LedgerList api={api} userId={account.user_id} generation={generation} />
      </div>
    </SidePanel>
  )
}
