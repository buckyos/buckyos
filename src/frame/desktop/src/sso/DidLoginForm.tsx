import { useState, type FormEvent } from 'react'
import { Alert, Button, CircularProgress, TextField } from '@mui/material'
import { accessDenialOf, type AccessDenial } from 'buckyos'
import { beginDidLogin } from '../api/sso'
import { useI18n } from '../i18n/provider'
import { DenialNotice } from './DenialNotice'

type Failure = { kind: 'denied'; denial: AccessDenial } | { kind: 'canonical_unavailable' } | { kind: 'invalid' } | { kind: 'failed' }

/**
 * DID login: the visitor names their DID, this Zone sends the browser to that DID's canonical Zone
 * for consent and comes back to `redirectUrl` signed in.
 */
export function DidLoginForm({ redirectUrl, requiredDid, onCancel }: { redirectUrl: string; requiredDid?: string; onCancel?: () => void }) {
  const { t } = useI18n()
  const [did, setDid] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<Failure | null>(null)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const value = (requiredDid ?? did).trim()
    if (!value.startsWith('did:') || value.length < 8) {
      setFailure({ kind: 'invalid' })
      return
    }
    setBusy(true)
    setFailure(null)
    const { data, error } = await beginDidLogin(value, redirectUrl)
    if (data?.status === 'redirect') {
      window.location.assign(data.url)
      return
    }
    setBusy(false)
    if (data?.status === 'denied') setFailure({ kind: 'denied', denial: data.denial })
    else if (data?.status === 'canonical_unavailable') setFailure({ kind: 'canonical_unavailable' })
    else {
      const denial = accessDenialOf(error)
      setFailure(denial ? { kind: 'denied', denial } : { kind: 'failed' })
    }
  }

  return (
    <form className="space-y-3" onSubmit={(event) => void submit(event)} noValidate data-testid="sso-did-form">
      <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('sso.did.hint', 'Sign in with an identity you already have. Its home Zone confirms it is you; no password is created here.')}
      </p>
      <TextField
        fullWidth
        size="small"
        label={t('sso.did.label', 'Your DID')}
        placeholder="did:bns:alice"
        value={requiredDid ?? did}
        disabled={busy || !!requiredDid}
        autoComplete="username"
        onChange={(event) => setDid(event.target.value)}
        slotProps={{ htmlInput: { 'data-testid': 'sso-did-input' } }}
      />
      {failure?.kind === 'denied' ? <DenialNotice denial={failure.denial} /> : null}
      {failure?.kind === 'canonical_unavailable' ? (
        <Alert severity="warning" data-testid="sso-did-unavailable">
          {t('sso.did.canonicalUnavailable', 'The home Zone of this DID cannot be reached right now. Try again later.')}
        </Alert>
      ) : null}
      {failure?.kind === 'invalid' ? (
        <Alert severity="error">{t('sso.did.invalid', 'Enter a DID such as did:bns:alice.')}</Alert>
      ) : null}
      {failure?.kind === 'failed' ? (
        <Alert severity="error">{t('sso.did.failed', 'DID sign-in could not start. Try again.')}</Alert>
      ) : null}
      <div className="flex gap-2">
        <Button type="submit" variant="contained" disabled={busy} data-testid="sso-did-submit">
          {busy ? <CircularProgress size={18} color="inherit" /> : t('sso.did.continue', 'Continue')}
        </Button>
        {onCancel ? (
          <Button variant="text" disabled={busy} onClick={onCancel}>
            {t('sso.common.cancel', 'Cancel')}
          </Button>
        ) : null}
      </div>
    </form>
  )
}
