import type { PublicOAuthSource } from 'buckyos'
import { useEffect, useState } from 'react'
import { Alert, Button, CircularProgress } from '@mui/material'
import { useParams } from 'react-router-dom'
import {
  acceptShare,
  describeShare,
  fetchSsoConfig,
  type ResourceShareDescription,
  type ShareIdentities,
  shareIdentities,
} from '../api/sso'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { endSessionInPlace } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { RegisterForm } from './EntryPage'
import { DidLoginForm } from './DidLoginForm'
import { invitationReturnUrl, rememberSharePage } from './shareReturn'
import { OAuthButtons } from './OAuthButtons'
import { useCurrentIdentity } from './identity'

type Source = 'session' | 'native' | 'oauth'
interface ShareView {
  share: ResourceShareDescription
  identities: ShareIdentities
  providers: PublicOAuthSource[]
  didEnabled: boolean
}
export default function SharePage() {
  const { inviteId = '' } = useParams()
  const { t } = useI18n()
  const { identity, reload } = useCurrentIdentity()
  const signedIn = !!identity && identity.kind !== 'anonymous'
  const [data, setData] = useState<ShareView | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [register, setRegister] = useState(false)
  useEffect(() => {
    if (!identity) return
    let live = true
    setData(null)
    setError('')
    void Promise.all([describeShare(inviteId), shareIdentities(inviteId, signedIn), fetchSsoConfig()]).then(
      ([share, ids, config]) => {
        if (!live) return
        if (!share.data || !ids.data || !config.data) {
          setError(
            String(
              share.error || ids.error || config.error || t('sso.share.unavailable', 'Invitation unavailable'),
            ),
          )
          return
        }
        setData({
          share: share.data,
          identities: ids.data,
          providers: config.data.oauth_sources ?? [],
          didEnabled: config.data.did_login_enabled,
        })
      },
    )
    return () => {
      live = false
    }
  }, [inviteId, identity, signedIn, t])
  const accept = async (source: Source) => {
    setBusy(true)
    setError('')
    const { data: result, error } = await acceptShare(inviteId, source, signedIn)
    if (result?.callback_url && data) {
      rememberSharePage(data.share.target_url, inviteId)
      window.location.assign(result.callback_url)
    } else {
      setError(String(error || t('sso.share.retry', 'Delivery failed. Try again.')))
      setBusy(false)
    }
  }
  const recipient = data?.share.audience.kind === 'directed' ? data.share.audience.did : undefined
  const required = data?.identities.required_source
  const providers = data?.share.methods.oauth
    ? data.providers.filter((p) => !required || required === p.id)
    : []
  const back = invitationReturnUrl(window.location.href)
  return (
    <AuthPageFrame title={t('sso.share.title', 'Resource invitation')} testId='sso-share-page'>
      {new URLSearchParams(window.location.search).has('did_error') && (
        <Alert severity='warning'>
          {t(
            'sso.share.interrupted',
            'Sign-in did not complete. You can retry this invitation or switch accounts.',
          )}
        </Alert>
      )}
      {error && <Alert severity='error' role='alert'>{error}</Alert>}
      {!data && !error && <CircularProgress size={24} />}
      {data && (
        <div className='space-y-4'>
          <h2 className='text-lg font-semibold'>{data.share.title}</h2>
          <p>
            {t('sso.share.from', '{{sharer}} invites you to {{host}}.', {
              sharer: data.share.sharer,
              host: new URL(data.share.target_url).host,
            })}
          </p>
          <p>{data.share.operations.join(', ')}</p>
          {data.share.unavailable_reason && <Alert severity='warning'>{data.share.unavailable_reason}</Alert>}
          {recipient && <p>{t('sso.share.recipient', 'Recipient: {{did}}', { did: recipient })}</p>}
          {!!data.identities.mismatches?.length && (
            <Alert severity='warning'>
              {t(
                'sso.share.mismatch',
                'This invitation is for a different identity. Switch accounts or cancel.',
              )}
            </Alert>
          )}
          {(['session', 'native', 'oauth'] as const).map((source) =>
            !data.identities.mismatches?.includes(source) && data.identities[source] && (
              <Button
                fullWidth
                variant='contained'
                disabled={busy}
                key={source}
                onClick={() => void accept(source)}
              >
                {t('sso.share.accept', 'Accept as {{did}}', { did: data.identities[source] })}
              </Button>
            )
          )}
          {signedIn && (
            <Button onClick={() => void endSessionInPlace().then(reload)}>
              {t('sso.share.switch', 'Switch account')}
            </Button>
          )}
          {!data.share.unavailable_reason && data.share.methods.guest && data.share.audience.kind === 'open' &&
            data.share.target.kind === 'system' && data.share.target.service_id === 'aiworkspace' &&
            (
              <Button
                href={'/sso/entry?' +
                  new URLSearchParams({ service: 'aiworkspace', entry: data.share.resource_id })}
              >
                {t('sso.entry.method.guest', 'Continue as guest')}
              </Button>
            )}
          {!signedIn && data.share.methods.password && (
            <Button href={'/login?redirect_url=' + encodeURIComponent(back)}>
              {t('sso.share.local', 'Use a local account')}
            </Button>
          )}
          {!signedIn && !data.share.unavailable_reason && data.share.methods.self_register && data.share.audience.kind === 'open' &&
            data.share.target.kind === 'system' &&
            (data.share.target.service_id === 'aiworkspace' || data.share.target.service_id === 'msg-center') &&
            (register
              ? (
                <RegisterForm
                  service={data.share.target.service_id}
                  entryId={data.share.invite_id}
                  redirectUrl={back}
                  onCancel={() => setRegister(false)}
                />
              )
              : (
                <Button onClick={() => setRegister(true)}>
                  {t('sso.entry.method.selfRegister', 'Create an account here')}
                </Button>
              ))}
          {!required && data.share.methods.did_login && data.didEnabled && (
            <DidLoginForm redirectUrl={back} requiredDid={recipient} purpose='share' inviteId={inviteId} />
          )}
          {required && providers.length === 0 && (
            <Alert severity='warning'>
              {t('sso.share.sourceDisabled', 'The login provider for this invitation is disabled.')}
            </Alert>
          )}
          <OAuthButtons sources={providers} purpose='share' redirectUrl={back} inviteId={inviteId} />
          <Button href='/'>{t('sso.common.cancel', 'Cancel')}</Button>
        </div>
      )}
    </AuthPageFrame>
  )
}
