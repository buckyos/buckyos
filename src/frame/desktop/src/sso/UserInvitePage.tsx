import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { useParams } from 'react-router-dom'
import { Alert, Button, CircularProgress, TextField } from '@mui/material'
import { accessDenialOf } from 'buckyos'
import { acceptUserInvite, fetchUserInvite, type UserInviteResponse } from '../api/user_mgr'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { endSessionInPlace } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { denialText } from './denial'
import { DidLoginForm } from './DidLoginForm'
import { IdentityCard } from './IdentityCard'
import { useCurrentIdentity } from './identity'

export function UserInvitePage() {
  const { t } = useI18n()
  const { inviteId = '' } = useParams()
  const { identity, reload } = useCurrentIdentity()
  const [invitation, setInvitation] = useState<UserInviteResponse | null>(null)
  const [failed, setFailed] = useState(false)
  const [ownerDocument, setOwnerDocument] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [accepted, setAccepted] = useState(false)
  const targetDid = invitation?.invite.target_did || undefined
  const verified = identity && identity.kind !== 'anonymous'
    && identity.auth_method === 'did_federation'
    && (!targetDid || identity.did === targetDid)

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
      setOwnerDocument('')
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
    let ownerConfig: Record<string, unknown>
    try {
      const parsed: unknown = JSON.parse(ownerDocument)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error()
      ownerConfig = parsed as Record<string, unknown>
    } catch {
      setError(t('sso.userInvite.invalidDocument', 'Enter a valid OwnerDocument JSON object.'))
      return
    }
    if (ownerConfig.id !== identity.did) {
      setError(t('sso.userInvite.documentMismatch', 'The document DID must match the DID you used to sign in.'))
      return
    }
    setBusy(true)
    const { data, error: failure } = await acceptUserInvite({ inviteId, ownerConfig })
    setBusy(false)
    if (data?.ok) {
      setOwnerDocument('')
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
        {identity.kind !== 'anonymous' ? <IdentityCard identity={identity} compact /> : null}
        {error ? <Alert severity="error" data-testid="user-invite-error">{error}</Alert> : null}
        {!verified && identity.kind !== 'anonymous' ? (
          <div className="space-y-3" data-testid="user-invite-switch">
            <Alert severity="info">{t('sso.userInvite.requiresFederation', 'Accepting this invitation requires a DID sign-in as the invited identity. Sign out here, then verify that DID.')}</Alert>
            <Button disabled={busy} onClick={() => void switchIdentity()}>
              {t('sso.userInvite.switchIdentity', 'Sign out and verify DID')}
            </Button>
          </div>
        ) : !verified ? (
          <DidLoginForm redirectUrl={window.location.href} requiredDid={targetDid} />
        ) : (
          <form className="space-y-4" onSubmit={(event) => void accept(event)} data-testid="user-invite-form">
            <TextField
              fullWidth
              multiline
              minRows={6}
              label={t('sso.userInvite.ownerDocument', 'OwnerDocument JSON')}
              helperText={t('sso.userInvite.ownerDocumentHint', 'Provide the public document for your DID, with a binding to this Zone: {{zone}}.', { zone: invitation.zone_did || invitation.zone_host || window.location.host })}
              value={ownerDocument}
              disabled={busy}
              onChange={(event) => setOwnerDocument(event.target.value)}
            />
            <Button type="submit" variant="contained" disabled={busy || !ownerDocument.trim()} data-testid="user-invite-accept">
              {busy ? <CircularProgress size={18} color="inherit" /> : t('sso.userInvite.accept', 'Accept invitation')}
            </Button>
          </form>
        )}
      </div>
    )
  }

  return <AuthPageFrame testId="user-invite-page" title={t('sso.userInvite.title', 'Join this Zone')}>{body}</AuthPageFrame>
}
