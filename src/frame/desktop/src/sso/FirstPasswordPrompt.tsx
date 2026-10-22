import { useState, type FormEvent } from 'react'
import { Alert, Button, TextField } from '@mui/material'
import { accessDenialOf, type CurrentIdentity } from 'buckyos'
import { KeyRound } from 'lucide-react'
import { setFirstPassword } from '../api/sso'
import { useI18n } from '../i18n/provider'
import { denialText } from './denial'
import { needsFirstPassword } from './identity'

const MIN_PASSWORD_LENGTH = 8

/**
 * The non-blocking "set a password" entry of an exclusive invite account that has none yet (P-1).
 * Renders nothing for anyone else. After it is set the invitation links stop signing the account
 * in; this device stays signed in.
 */
export function FirstPasswordPrompt({
  identity,
  onPasswordSet,
}: {
  identity: CurrentIdentity | null | undefined
  onPasswordSet?: () => void
}) {
  const { t } = useI18n()
  const [phase, setPhase] = useState<'idle' | 'editing' | 'saving' | 'done'>('idle')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [error, setError] = useState<string | null>(null)

  if (phase === 'done' && identity?.kind === 'account') {
    return (
      <Alert severity="success" data-testid="sso-first-password-done">
        {t('sso.firstPassword.done', 'Password set. From now on, sign in as {{userId}} with this password.', { userId: identity.user_id })}
      </Alert>
    )
  }
  if (!needsFirstPassword(identity) || identity?.kind !== 'account') return null
  const userId = identity.user_id

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (password.length < MIN_PASSWORD_LENGTH) {
      setError(t('sso.firstPassword.tooShort', 'Use at least {{count}} characters.', { count: MIN_PASSWORD_LENGTH }))
      return
    }
    if (password !== confirm) {
      setError(t('sso.firstPassword.mismatch', 'The two passwords do not match.'))
      return
    }
    setError(null)
    setPhase('saving')
    const { data, error: failure } = await setFirstPassword(userId, password)
    if (data?.status === 'ok') {
      setPassword('')
      setConfirm('')
      setPhase('done')
      onPasswordSet?.()
      return
    }
    const denial = data?.status === 'denied' ? data.denial : accessDenialOf(failure)
    setError(denial ? denialText(t, denial) : t('sso.firstPassword.failed', 'The password could not be set. Try again.'))
    setPhase('editing')
  }

  return (
    <div
      className="rounded-2xl p-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-accent-soft) 12%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))',
      }}
      data-testid="sso-first-password"
    >
      <div className="flex flex-wrap items-center gap-3">
        <KeyRound size={18} style={{ color: 'var(--cp-accent)' }} />
        <p className="min-w-0 flex-1 text-sm" style={{ color: 'var(--cp-text)' }}>
          {t('sso.firstPassword.hint', 'Your account has no password yet. Set one to keep signing in after your invitation link expires.')}
        </p>
        {phase === 'idle' ? (
          <Button size="small" variant="contained" onClick={() => setPhase('editing')} data-testid="sso-first-password-open">
            {t('sso.firstPassword.open', 'Set password')}
          </Button>
        ) : null}
      </div>
      {phase === 'editing' || phase === 'saving' ? (
        <form className="mt-3 space-y-3" onSubmit={(event) => void submit(event)} noValidate>
          <TextField
            fullWidth
            size="small"
            type="password"
            autoComplete="new-password"
            label={t('sso.firstPassword.password', 'New password')}
            value={password}
            disabled={phase === 'saving'}
            onChange={(event) => setPassword(event.target.value)}
          />
          <TextField
            fullWidth
            size="small"
            type="password"
            autoComplete="new-password"
            label={t('sso.firstPassword.confirm', 'Confirm password')}
            value={confirm}
            disabled={phase === 'saving'}
            onChange={(event) => setConfirm(event.target.value)}
          />
          {error ? <Alert severity="error">{error}</Alert> : null}
          <div className="flex gap-2">
            <Button type="submit" size="small" variant="contained" disabled={phase === 'saving'} data-testid="sso-first-password-save">
              {phase === 'saving' ? t('sso.firstPassword.saving', 'Saving…') : t('sso.firstPassword.save', 'Save password')}
            </Button>
            <Button size="small" variant="text" disabled={phase === 'saving'} onClick={() => { setPhase('idle'); setError(null) }}>
              {t('sso.common.cancel', 'Cancel')}
            </Button>
          </div>
        </form>
      ) : null}
    </div>
  )
}
