import { Alert, Button, CircularProgress, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { useEffect, useState } from 'react'
import { fetchOAuthSourceAccounts, type OAuthSource, type OAuthSourceAccount } from '../../../../api/sso'
import { useI18n } from '../../../../i18n/provider'

type Accounts = { kind: 'loading' } | { kind: 'failed' } | { kind: 'ready'; accounts: OAuthSourceAccount[] }

/**
 * Confirms deleting a login source, naming the local accounts that sign in with it. Give it a new
 * `key` for each opening.
 */
export function DeleteOAuthSourceDialog({
  source,
  onClose,
  onDelete,
}: {
  source: OAuthSource | null
  onClose: () => void
  /** Resolves to an error text, or null once deleted. */
  onDelete: (source: OAuthSource) => Promise<string | null>
}) {
  const { t } = useI18n()
  const [accounts, setAccounts] = useState<Accounts>({ kind: 'loading' })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [seq, setSeq] = useState(0)

  useEffect(() => {
    if (!source) return
    let live = true
    void fetchOAuthSourceAccounts(source.id).then(({ data }) => {
      if (!live) return
      setAccounts(data ? { kind: 'ready', accounts: data.accounts } : { kind: 'failed' })
    })
    return () => { live = false }
  }, [source, seq])

  const remove = async () => {
    if (!source) return
    setBusy(true)
    setError(null)
    const failure = await onDelete(source)
    setBusy(false)
    if (failure) setError(failure)
  }

  const name = source?.display_name ?? ''
  const affected = accounts.kind === 'ready' ? accounts.accounts : []

  return (
    <Dialog open={!!source} onClose={busy ? undefined : onClose} fullWidth maxWidth="xs" data-testid="sso-provider-delete-dialog">
      <DialogTitle>{t('sso.provider.deleteTitle', 'Delete “{{name}}”?', { name })}</DialogTitle>
      <DialogContent className="space-y-3">
        {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
        {accounts.kind === 'loading' ? (
          <div className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}>
            <CircularProgress size={14} />
            {t('sso.provider.checkingAccounts', 'Checking which accounts sign in with it…')}
          </div>
        ) : accounts.kind === 'failed' ? (
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-sm" style={{ color: 'var(--cp-danger)' }} role="alert">
              {t('sso.provider.accountsFailed', 'Could not check which accounts sign in with it.')}
            </p>
            <Button size="small" variant="outlined" onClick={() => { setAccounts({ kind: 'loading' }); setSeq((value) => value + 1) }}>
              {t('sso.common.retry', 'Retry')}
            </Button>
          </div>
        ) : affected.length ? (
          <>
            <p className="text-sm leading-6" style={{ color: 'var(--cp-text)' }}>
              {t('sso.provider.deleteAffects', 'After it is deleted, the users below can no longer sign in with {{name}}. Delete it?', { name })}
            </p>
            <ul className="max-h-56 space-y-1 overflow-y-auto rounded-lg border p-2 text-sm" style={{ borderColor: 'var(--cp-border)' }} data-testid="sso-provider-delete-accounts">
              {affected.map((account) => (
                <li key={account.user_id} className="px-1 py-0.5">
                  <span className="block font-medium" style={{ color: 'var(--cp-text)' }}>
                    {account.show_name}
                    {account.show_name !== account.user_id && <span className="font-normal" style={{ color: 'var(--cp-muted)' }}> · {account.user_id}</span>}
                  </span>
                  <span className="block break-all" dir="ltr" style={{ color: 'var(--cp-muted)' }}>{account.email}</span>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p className="text-sm leading-6" style={{ color: 'var(--cp-text)' }}>
            {t('sso.provider.deleteNoAccounts', 'No local account signs in with {{name}}. Delete it?', { name })}
          </p>
        )}
        <p className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
          {t('sso.provider.deleteNote', 'Visitors who signed in with it lose access to what was shared with them. Adding the same source again later restores these sign-ins. To stop it for a while, switch it off instead.')}
        </p>
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={onClose}>{t('settings.common.cancel', 'Cancel')}</Button>
        <Button
          color="error"
          variant="contained"
          disabled={busy || accounts.kind !== 'ready'}
          onClick={() => void remove()}
          data-testid="sso-provider-delete-confirm"
        >
          {busy ? t('sso.provider.deleting', 'Deleting…') : t('sso.provider.delete', 'Delete')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
