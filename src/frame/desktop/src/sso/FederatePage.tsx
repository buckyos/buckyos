/* ── Canonical-side consent `/sso/federate?…` of a DID login (SSO-09, contract §9) ──
 *
 * Another Zone asked to sign this account's DID in. The signed-in owner sees the DID, the target
 * Zone and the public profile it will receive, then agrees or cancels; either way the browser
 * goes back to the target Zone with the answer.
 */

import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Alert, Button, Checkbox, CircularProgress, FormControlLabel } from '@mui/material'
import { ArrowRight, ShieldCheck } from 'lucide-react'
import { accessDenialOf, type AccessDenial } from 'buckyos'
import {
  FEDERATION_REQUEST_KEYS,
  approveFederation,
  denyFederation,
  describeFederation,
  type FederationConsent,
  type FederationDecision,
  type FederationRequest,
} from '../api/sso'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { redirectToDesktopLogin } from '../auth/loginRedirect'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { DenialNotice } from './DenialNotice'
import { IdentityCard } from './IdentityCard'
import { useCurrentIdentity } from './identity'

type View =
  | { kind: 'loading' }
  | { kind: 'invalid' }
  | { kind: 'failed' }
  | { kind: 'denied'; denial: AccessDenial }
  | { kind: 'consent'; consent: FederationConsent }
  | { kind: 'leaving' }

const RESTRICTED_DENIAL: AccessDenial = { reason: 'session_kind_not_allowed', adjust: { party: 'product' } }

function requestFrom(params: URLSearchParams): FederationRequest | null {
  const entries = FEDERATION_REQUEST_KEYS.map((key) => [key, params.get(key)?.trim() ?? ''] as const)
  if (entries.some(([, value]) => !value)) return null
  return Object.fromEntries(entries) as unknown as FederationRequest
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 py-1.5 text-sm sm:flex-row sm:justify-between sm:gap-4">
      <span style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="break-all font-medium sm:text-right" style={{ color: 'var(--cp-text)' }}>{children}</span>
    </div>
  )
}

export function FederatePage() {
  const { t } = useI18n()
  const [searchParams] = useSearchParams()
  const request = useMemo(() => requestFrom(searchParams), [searchParams])
  const { identity } = useCurrentIdentity()
  const [view, setView] = useState<View>({ kind: 'loading' })
  const [remember, setRemember] = useState(true)

  useDocumentTitle(t('sso.federate.documentTitle', 'Confirm DID sign-in'))

  useEffect(() => {
    if (!identity || !request) return
    if (identity.kind === 'anonymous') {
      redirectToDesktopLogin()
      return
    }
    if (identity.restricted) return
    let live = true
    void describeFederation(request).then(({ data, error }) => {
      if (!live) return
      if (data?.status === 'consent') setView({ kind: 'consent', consent: data })
      else if (data?.status === 'denied') setView({ kind: 'denied', denial: data.denial })
      else {
        const denial = accessDenialOf(error)
        setView(denial ? { kind: 'denied', denial } : { kind: 'failed' })
      }
    })
    return () => { live = false }
  }, [identity, request])

  const decide = async (decision: Promise<{ data: FederationDecision | null; error: unknown }>) => {
    setView({ kind: 'leaving' })
    const { data, error } = await decision
    if (data?.status === 'redirect') {
      window.location.assign(data.url)
      return
    }
    const denial = data?.status === 'denied' ? data.denial : accessDenialOf(error)
    setView(denial ? { kind: 'denied', denial } : { kind: 'failed' })
  }

  let body: ReactNode
  if (identity && identity.kind !== 'anonymous' && identity.restricted) {
    // Only Admin and User accounts sign in elsewhere with their DID (product boundary); no account call is made.
    body = <DenialNotice denial={RESTRICTED_DENIAL} testId="sso-federate-denied" />
  } else if (!request) {
    body = <Alert severity="error" data-testid="sso-federate-invalid">{t('sso.federate.invalid', 'This sign-in request is incomplete. Start again from the Zone you want to sign in to.')}</Alert>
  } else if (view.kind === 'denied') {
    body = <DenialNotice denial={view.denial} showAdjust={identity?.kind === 'account' && !identity.restricted} testId="sso-federate-denied" />
  } else if (view.kind === 'failed') {
    body = <Alert severity="error" data-testid="sso-federate-failed">{t('sso.federate.failed', 'The request cannot be checked right now. Try again later.')}</Alert>
  } else if (view.kind === 'consent' && identity && identity.kind !== 'anonymous') {
    const { consent } = view
    body = (
      <div className="space-y-4" data-testid="sso-federate-consent" data-already-granted={consent.already_granted ? 'true' : 'false'}>
        <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
          {t('sso.federate.intro', '{{host}} asks to sign you in with your DID. It receives only what is listed below.', { host: consent.client_host })}
        </p>
        <IdentityCard identity={identity} compact />
        <div className="rounded-2xl px-4 py-2" style={{ border: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }}>
          <Row label={t('sso.federate.did', 'DID')}><span className="font-mono text-xs" data-testid="sso-federate-did">{request.did}</span></Row>
          <Row label={t('sso.federate.target', 'Signing in to')}><span data-testid="sso-federate-target">{consent.client_host}</span></Row>
          <Row label={t('sso.federate.targetZone', 'Target Zone DID')}><span className="font-mono text-xs">{consent.client_zone}</span></Row>
          <Row label={t('sso.federate.displayName', 'Display name')}><span data-testid="sso-federate-name">{consent.profile.show_name}</span></Row>
          <Row label={t('sso.federate.avatar', 'Avatar')}>
            {consent.profile.avatar
              ? <img src={consent.profile.avatar} alt="" className="inline-block h-6 w-6 rounded-full object-cover" />
              : t('sso.federate.noAvatar', 'None')}
          </Row>
        </div>
        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {t('sso.federate.private', 'Your contacts, files and other private data are not shared. Signing in there does not change which Zone your identity belongs to.')}
        </p>
        {consent.already_granted ? (
          <Alert severity="info" data-testid="sso-federate-granted">
            {t('sso.federate.alreadyGranted', 'You agreed to this Zone before.')}
          </Alert>
        ) : (
          <FormControlLabel
            control={<Checkbox size="small" checked={remember} onChange={(event) => setRemember(event.target.checked)} />}
            label={<span className="text-sm">{t('sso.federate.remember', 'Remember this choice for {{host}}', { host: consent.client_host })}</span>}
            data-testid="sso-federate-remember"
          />
        )}
        <div className="flex flex-col gap-2 sm:flex-row">
          <Button
            fullWidth
            variant="contained"
            startIcon={consent.already_granted ? <ArrowRight size={16} /> : <ShieldCheck size={16} />}
            onClick={() => void decide(approveFederation(request, consent.already_granted || remember))}
            data-testid="sso-federate-approve"
          >
            {consent.already_granted ? t('sso.federate.continue', 'Continue') : t('sso.federate.approve', 'Agree and continue')}
          </Button>
          <Button fullWidth variant="outlined" onClick={() => void decide(denyFederation(request))} data-testid="sso-federate-deny">
            {t('sso.common.cancel', 'Cancel')}
          </Button>
        </div>
      </div>
    )
  } else {
    body = <div className="flex justify-center py-6"><CircularProgress size={28} /></div>
  }

  return (
    <AuthPageFrame testId="sso-federate-page" title={t('sso.federate.title', 'Sign in to another Zone')}>
      {body}
    </AuthPageFrame>
  )
}
