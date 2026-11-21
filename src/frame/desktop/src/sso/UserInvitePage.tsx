import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { useParams } from 'react-router-dom'
import { Alert, Button, CircularProgress } from '@mui/material'
import { accessDenialOf } from 'buckyos'
import { acceptUserInvite, fetchUserInvite, type UserInviteResponse } from '../api/user_mgr'
import { OAuthButtons } from './OAuthButtons'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { endSessionInPlace } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { denialText } from './denial'
import { DidLoginForm } from './DidLoginForm'
import { IdentityCard } from './IdentityCard'
import { useCurrentIdentity } from './identity'
import { invitationReturnUrl } from './shareReturn'

export function UserInvitePage() {
  const { t } = useI18n()
  const { inviteId = '' } = useParams()
  const { identity, reload } = useCurrentIdentity()
  const [invitation, setInvitation] = useState<UserInviteResponse | null>(null)
  const [failed, setFailed] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [accepted, setAccepted] = useState(false)
  const targetDid = invitation?.invite.target_did || undefined
  const returnPage = invitationReturnUrl(window.location.href)
  const signInInterrupted = new URLSearchParams(window.location.search).has('did_error')
  const oauthSource = invitation?.oauth_source
  const verified = !!targetDid && (identity?.kind === 'anonymous'
    ? invitation?.verified_did === targetDid
    : identity?.auth_method === 'did_federation' && identity.did === targetDid)

  useDocumentTitle(t('sso.userInvite.title', 'Join this Zone'))

  useEffect(() => {
    let live = true
    void fetchUserInvite(inviteId).then(({ data }) => {
      if (!live) return
      setInvitation(data)
      setFailed(!data)
    })
    return () => { live = false }
  }, [inviteId])

  const switchIdentity = async () => {
    setBusy(true)
    setError(null)
    try {
      await endSessionInPlace()
      reload()
    } catch {
      setError(t('sso.userInvite.signOutFailed', 'Could not sign out. Try again.'))
    } finally {
      setBusy(false)
    }
  }

  const accept = async (event: FormEvent) => {
    event.preventDefault()
    if (!verified || busy) return
    setError(null)
    setBusy(true)
    const { data, error: failure } = await acceptUserInvite({ inviteId })
    setBusy(false)
    if (data?.ok) {
      setInvitation(data)
      setAccepted(true)
      return
    }
    const denial = accessDenialOf(failure)
    setError(denial ? denialText(t, denial) : failure instanceof Error
      ? failure.message : t('sso.userInvite.acceptFailed', 'The invitation could not be accepted. Try again.'))
  }

  let body: ReactNode
  if (failed) {
    body = <Alert severity="error">{t('sso.userInvite.loadFailed', 'This invitation could not be loaded. Check the link or try again later.')}</Alert>
  } else if (!invitation || !identity) {
    body = <CircularProgress size={28} />
  } else if (accepted || invitation.invite.state === 'accepted') {
    body = (
      <div className="space-y-4">
        <Alert severity="success" data-testid="user-invite-accepted">
          {t('sso.userInvite.accepted', 'This invitation has been accepted. Sign in again with your DID to use the account.')}
        </Alert>
        <Button href={`/login?redirect_url=${encodeURIComponent(new URL('/', window.location.origin).href)}`}>
          {t('sso.userInvite.signIn', 'Sign in')}
        </Button>
      </div>
    )
  } else if (invitation.expired || invitation.invite.state !== 'pending') {
    body = <Alert severity="warning" data-testid="user-invite-unavailable">{t('sso.userInvite.unavailable', 'This invitation is no longer available. Ask the person who invited you for a new link.')}</Alert>
  } else {
    body = (
      <div className="space-y-4">
        <p className="break-all text-sm" data-testid="user-invite-target">
          {targetDid ? t('sso.userInvite.forDid', 'Invitation for {{did}}', { did: targetDid })
            : t('sso.userInvite.verifyDid', 'Sign in with your DID before accepting this invitation.')}
        </p>
        <p className="text-sm">{t('sso.userInvite.purpose', '{{issuer}} invites you to sign in to {{zone}} as {{user}} ({{type}}).', { issuer: invitation.invite.created_by, zone: invitation.zone_host || window.location.host, user: invitation.invite.target_user_id || targetDid || '', type: invitation.invite.default_user_type })}</p>
        {invitation.invite.groups?.length ? <p>{invitation.invite.groups.join(', ')}</p> : null}
        {invitation.invite.expires_at ? <p>{t('sso.userInvite.expires', 'Expires {{time}}', {time: new Date(invitation.invite.expires_at * 1000).toLocaleString()})}</p> : null}
        {identity.kind !== 'anonymous' ? <IdentityCard identity={identity} compact /> : null}
        {signInInterrupted && (
          <Alert severity="warning">{t('sso.share.interrupted', 'Sign-in did not complete. You can retry this invitation or switch accounts.')}</Alert>
        )}
        {error ? <Alert severity="error" data-testid="user-invite-error">{error}</Alert> : null}
        {oauthSource ? (
          invitation.source_enabled === false ? <Alert severity="warning">{t('sso.userInvite.disabled', 'The login provider has been disabled.')}</Alert>
            : <OAuthButtons sources={[{id:oauthSource,display_name:'Google'}]} purpose="user_invite" inviteId={inviteId} redirectUrl={returnPage} />
        ) : !verified && identity.kind !== 'anonymous' ? (
          <div className="space-y-3" data-testid="user-invite-switch">
            <Alert severity="info">{t('sso.userInvite.requiresFederation', 'Accepting this invitation requires a DID sign-in as the invited identity. Sign out here, then verify that DID.')}</Alert>
            <Button disabled={busy} onClick={() => void switchIdentity()}>
              {t('sso.userInvite.switchIdentity', 'Sign out and verify DID')}
            </Button>
          </div>
        ) : !verified ? (
          <DidLoginForm redirectUrl={returnPage} requiredDid={targetDid} purpose="user_invite" inviteId={inviteId} />
        ) : (
          <form className="space-y-4" onSubmit={(event) => void accept(event)} data-testid="user-invite-form">
            <p>{t('sso.userInvite.binding', 'Confirm the binding to this Zone in your DID wallet. The Zone will check your published OwnerDocument when you accept.')}</p>
            <Button type="submit" variant="contained" disabled={busy} data-testid="user-invite-accept">
              {busy ? <CircularProgress size={18} color="inherit" /> : t('sso.userInvite.accept', 'Accept invitation')}
            </Button>
          </form>
        )}
      </div>
    )
  }

  return <AuthPageFrame testId="user-invite-page" title={t('sso.userInvite.title', 'Join this Zone')}>{body}</AuthPageFrame>
}
