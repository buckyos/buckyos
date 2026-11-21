import type { PublicOAuthSource } from 'buckyos'
import { useState } from 'react'
import { Alert, Button } from '@mui/material'
import { beginOAuthLogin, type LoginPurpose } from '../api/sso'
import { useI18n } from '../i18n/provider'

export function OAuthButtons(
  { sources, purpose, redirectUrl, inviteId }: {
    sources: PublicOAuthSource[]
    purpose: LoginPurpose
    redirectUrl: string
    inviteId?: string
  },
) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const start = async (id: string) => {
    setBusy(true)
    setFailed(false)
    const { data } = await beginOAuthLogin(id, redirectUrl, purpose, inviteId)
    if (data?.status === 'redirect') window.location.assign(data.url)
    else {
      setFailed(true)
      setBusy(false)
    }
  }
  if (!sources.length) return null
  return (
    <div className='space-y-2' aria-label={t('sso.sources', 'Login from')}>
      {sources.map((source) => (
        <Button
          key={source.id}
          fullWidth
          type='button'
          variant='outlined'
          disabled={busy}
          onClick={() => void start(source.id)}
        >
          {t('sso.source.button', 'Continue with {{source}}', { source: source.display_name })}
        </Button>
      ))}
      {failed && (
        <Alert severity='error'>{t('sso.source.failed', 'Sign-in could not start. Please try again.')}</Alert>
      )}
    </div>
  )
}
