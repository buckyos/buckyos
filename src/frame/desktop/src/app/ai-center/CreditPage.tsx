import { useEffect, useMemo, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { fetchUserGroups } from '../../api/user_mgr'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { usePolicyApi } from './hooks/use-policy-api'
import { isNegativeCredit, isPositiveCredit, type CreditAccount } from './datamodel/policy'
import { AdjustDialog, LedgerList, LedgerPanel, RechargeDialog } from './components/credit/CreditDialogs'
import { CreditAmount, Field } from './components/policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, formatLocal, panelStyle } from './components/policy/policy-labels'

type Action = { kind: 'recharge' | 'adjust' | 'ledger'; account: CreditAccount }

/** AI Credit balances: an administrator sees and recharges every user's; a user sees their own and its ledger. */
export function CreditPage() {
  const { t, locale } = useI18n()
  const api = usePolicyApi()
  const isAdmin = useViewerIsAdmin()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [accounts, setAccounts] = useState<CreditAccount[] | null>(null)
  const [own, setOwn] = useState<CreditAccount | null>(null)
  const [groups, setGroups] = useState<{ id: string; name: string }[]>([])
  const [group, setGroup] = useState('')
  const [query, setQuery] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [generation, setGeneration] = useState(0)
  const [action, setAction] = useState<Action | null>(null)
  const [ledgerFor, setLedgerFor] = useState<CreditAccount | null>(null)

  useEffect(() => {
    if (!isAdmin) return
    void fetchUserGroups().then(({ data }) => setGroups((data?.groups ?? []).map((item) => ({ id: item.group_id, name: item.name })))).catch(() => undefined)
  }, [isAdmin])
  useEffect(() => {
    if (isAdmin === null) return
    let live = true
    const load = isAdmin ? api.listAccounts(group || undefined).then((list) => { if (live) setAccounts(list) }) : api.getAccount().then((account) => { if (live) setOwn(account) })
    load.then(() => { if (live) setError(null) }).catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, isAdmin, group, generation])

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return (accounts ?? []).filter((account) => !needle || account.user_id.toLowerCase().includes(needle))
  }, [accounts, query])
  const changed = (account: CreditAccount) => {
    setAccounts((list) => list?.map((item) => item.user_id === account.user_id ? account : item) ?? list)
    setLedgerFor((current) => current && current.user_id === account.user_id ? account : current)
    setGeneration((value) => value + 1)
  }
  const negative = (accounts ?? []).filter((account) => isNegativeCredit(account.balance)).length

  return (
    <div className="flex flex-col gap-5" data-testid="credit-page">
      <header className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h1 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.credit.title', 'AI Credit')}</h1>
          <p className="mt-1 max-w-3xl text-sm" style={{ color: 'var(--cp-muted)' }}>
            {isAdmin
              ? t('aiCenter.credit.subtitle', 'AI Credit is the Zone’s unit for AI use. Calls to priced models deduct it after they run, so a large call can take a balance below zero; then priced models are refused until a recharge. It is not the Provider account balance or SN Credit.')
              : t('aiCenter.credit.subtitleUser', 'Your AI Credit pays for priced models; free models do not use it. An administrator recharges it. It is not a Provider account balance.')}
          </p>
        </div>
        <button type="button" onClick={() => setGeneration((value) => value + 1)} aria-label={t('aiCenter.policy.refresh', 'Refresh')} title={t('aiCenter.policy.refresh', 'Refresh')} className="min-h-10 self-start rounded-lg px-3" style={buttonStyle.secondary}><RefreshCw size={16} /></button>
      </header>
      {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}

      {isAdmin === false && own && (
        <>
          <section className="flex flex-col gap-1 rounded-xl p-4" style={panelStyle} data-testid="credit-own">
            <Field label={t('aiCenter.credit.myBalance', 'My AI Credit balance')}><span className="text-2xl"><CreditAmount value={own.balance} strong testId="credit-own-balance" /></span></Field>
            {!isPositiveCredit(own.balance)
              ? <p className="text-xs" style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.credit.emptyHint', 'Priced models are refused until an administrator recharges your AI Credit. Free models still work.')}</p>
              : null}
          </section>
          <section className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle}>
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.credit.ledger', 'Ledger')}</h2>
            <LedgerList api={api} generation={generation} />
          </section>
        </>
      )}

      {isAdmin && (
        <>
          {negative > 0 && (
            <div className="rounded-xl px-4 py-3 text-sm" data-testid="credit-negative" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
              {t('aiCenter.credit.negativeNotice', '{{count}} users are below zero; their priced calls are refused until a recharge.', { count: negative })}
            </div>
          )}
          <div className="flex flex-col gap-2 md:flex-row md:items-center">
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t('aiCenter.credit.search', 'Search user')} aria-label={t('aiCenter.credit.search', 'Search user')} className="min-h-10 flex-1 rounded-lg px-3 text-sm" style={fieldStyle} />
            <select data-testid="credit-group" aria-label={t('aiCenter.credit.group', 'User group')} value={group} onChange={(event) => setGroup(event.target.value)} className="min-h-10 rounded-lg px-3 text-sm" style={fieldStyle}>
              <option value="">{t('aiCenter.credit.allUsers', 'All users')}</option>
              {groups.map((item) => <option key={item.id} value={item.id}>{item.name || item.id}</option>)}
            </select>
          </div>
          {!accounts && !error && <div role="status" className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('common.loading', 'Loading...')}</div>}
          {accounts && (
            isMobile ? (
              <div className="flex flex-col gap-2">
                {shown.map((account) => (
                  <div key={account.user_id} data-testid="credit-account" data-user-id={account.user_id} className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle}>
                    <button type="button" onClick={() => setLedgerFor(account)} className="flex items-center justify-between gap-2 text-left">
                      <span className="min-w-0 break-all text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{account.user_id}</span>
                      <span className="text-base"><CreditAmount value={account.balance} strong testId="credit-balance" /></span>
                    </button>
                    <div className="flex gap-2">
                      <button type="button" data-testid="credit-recharge-open" onClick={() => setAction({ kind: 'recharge', account })} className="min-h-10 flex-1 rounded-lg text-sm font-medium" style={buttonStyle.primary}>{t('aiCenter.credit.recharge', 'Recharge')}</button>
                      <button type="button" data-testid="credit-adjust-open" onClick={() => setAction({ kind: 'adjust', account })} className="min-h-10 flex-1 rounded-lg text-sm" style={buttonStyle.secondary}>{t('aiCenter.credit.adjust', 'Adjust')}</button>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="overflow-hidden rounded-xl" style={panelStyle}>
                <table className="w-full text-left text-sm" style={{ color: 'var(--cp-text)' }}>
                  <thead className="text-xs" style={{ background: 'var(--cp-bg)', color: 'var(--cp-muted)' }}>
                    <tr>
                      <th className="px-4 py-2 font-medium">{t('aiCenter.credit.user', 'User')}</th>
                      <th className="px-4 py-2 text-right font-medium">{t('aiCenter.credit.balance', 'AI Credit balance')}</th>
                      <th className="px-4 py-2 font-medium">{t('aiCenter.credit.updated', 'Last change')}</th>
                      <th className="px-4 py-2 font-medium"><span className="sr-only">{t('aiCenter.credit.actions', 'Actions')}</span></th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map((account) => (
                      <tr key={account.user_id} data-testid="credit-account" data-user-id={account.user_id} style={{ borderTop: '1px solid var(--cp-border)' }}>
                        <td className="px-4 py-1"><button type="button" onClick={() => setLedgerFor(account)} className="min-h-10 break-all text-left underline-offset-2 hover:underline" style={{ color: 'var(--cp-accent)' }}>{account.user_id}</button></td>
                        <td className="px-4 py-2 text-right"><CreditAmount value={account.balance} strong testId="credit-balance" /></td>
                        <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{account.updated_at_ms ? formatLocal(account.updated_at_ms, locale) : t('aiCenter.credit.never', 'Never')}</td>
                        <td className="px-4 py-2">
                          <div className="flex justify-end gap-2">
                            <button type="button" data-testid="credit-adjust-open" onClick={() => setAction({ kind: 'adjust', account })} className="min-h-9 rounded-lg px-3 text-xs" style={buttonStyle.secondary}>{t('aiCenter.credit.adjust', 'Adjust')}</button>
                            <button type="button" data-testid="credit-recharge-open" onClick={() => setAction({ kind: 'recharge', account })} className="min-h-9 rounded-lg px-3 text-xs font-medium" style={buttonStyle.primary}>{t('aiCenter.credit.recharge', 'Recharge')}</button>
                          </div>
                        </td>
                      </tr>
                    ))}
                    {shown.length === 0 && <tr><td colSpan={4} className="px-4 py-8 text-center text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.credit.noAccounts', 'No user matches.')}</td></tr>}
                  </tbody>
                </table>
              </div>
            )
          )}
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.credit.gatewayHint', 'A payment gateway recharges through the same API with the order id as recharge id; it needs the obj://aicc/credit/recharge grant in the RBAC policy.')}</p>
        </>
      )}

      {ledgerFor && (
        <LedgerPanel
          api={api}
          account={ledgerFor}
          generation={generation}
          onClose={() => setLedgerFor(null)}
          onRecharge={() => setAction({ kind: 'recharge', account: ledgerFor })}
          onAdjust={() => setAction({ kind: 'adjust', account: ledgerFor })}
        />
      )}
      {action?.kind === 'recharge' && <RechargeDialog api={api} account={action.account} onClose={() => setAction(null)} onDone={changed} />}
      {action?.kind === 'adjust' && <AdjustDialog api={api} account={action.account} onClose={() => setAction(null)} onDone={changed} />}
    </div>
  )
}
