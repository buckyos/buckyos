/* ── Share entry `/sso/entry?service=&entry=`: how a visitor enters a shared resource (SSO-11) ──
 *
 * The resource service describes the entry, Control Panel intersects its methods with the Zone
 * switches. Only those methods are offered; methods the Zone closed show as unavailable. After a
 * sign-in the visitor comes back with `join=1` and enters the resource as that identity.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Alert, Button, CircularProgress, TextField } from '@mui/material'
import { ArrowRight, Fingerprint, LogIn, UserPlus, UserRound } from 'lucide-react'
import { accessDenialOf, buckyos, type AccessDenial, type CurrentIdentity, type EntryDescriptor, type EntryMethod, type EntryService } from 'buckyos'
import { describeEntry, joinWorkspaceEntry, parseEntryService, registerAccount } from '../api/sso'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { loginWithPassword } from '../auth/passwordLogin'
import { endSessionInPlace } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { currentZoneHost } from '../presentation/zone'
import { DenialNotice } from './DenialNotice'
import { DidLoginForm } from './DidLoginForm'
import { IdentityCard } from './IdentityCard'
import { identityDisplayName, isRestrictedIdentity, useCurrentIdentity } from './identity'

// The server's account names: one lowercase DNS label.
const USER_ID_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/
const RESERVED_USER_IDS = new Set(['root', 'system', 'admin', 'guest'])
const MIN_PASSWORD_LENGTH = 8

type Failure = { kind: 'denied'; denial: AccessDenial } | { kind: 'failed' }

function failureOf(error: unknown): Failure {
  const denial = accessDenialOf(error)
  return denial ? { kind: 'denied', denial } : { kind: 'failed' }
}

const PENDING_JOIN_KEY = 'buckyos.sso.pending-join'

function entryKey(): string {
  const params = new URLSearchParams(window.location.search)
  return `${params.get('service') ?? ''}|${params.get('entry') ?? ''}`
}

/**
 * This page with `join=1`: where a sign-in started here comes back to. The
 * sign-in leaves a marker in this tab, so only a return from it enters on its
 * own; a crafted `join=1` link still asks the visitor to continue.
 */
function joinUrl(): string {
  try {
    window.sessionStorage.setItem(PENDING_JOIN_KEY, entryKey())
  } catch {
    // No storage: the visitor confirms with the continue button instead.
  }
  const url = new URL(window.location.href)
  url.searchParams.set('join', '1')
  url.hash = ''
  return url.toString()
}

/** Whether a sign-in started on this entry left its marker in this tab. */
function hasPendingJoin(): boolean {
  try {
    return window.sessionStorage.getItem(PENDING_JOIN_KEY) === entryKey()
  } catch {
    return false
  }
}

function clearPendingJoin() {
  try {
    window.sessionStorage.removeItem(PENDING_JOIN_KEY)
  } catch {
    // Nothing was stored.
  }
}

function FailureNotice({ failure, identity }: { failure: Failure; identity: CurrentIdentity | null }) {
  const { t } = useI18n()
  if (failure.kind === 'denied') {
    return <DenialNotice denial={failure.denial} showAdjust={!!identity && identity.kind !== 'anonymous' && !isRestrictedIdentity(identity)} />
  }
  return <Alert severity="error">{t('sso.entry.failed', 'Something went wrong. Try again.')}</Alert>
}

function MethodButton({
  icon,
  label,
  unavailable,
  onClick,
  testId,
}: {
  icon: ReactNode
  label: string
  unavailable?: boolean
  onClick?: () => void
  testId: string
}) {
  const { t } = useI18n()
  return (
    <Button
      fullWidth
      variant={unavailable ? 'text' : 'outlined'}
      disabled={unavailable}
      startIcon={icon}
      onClick={onClick}
      sx={{ justifyContent: 'flex-start', py: 1.1 }}
      data-testid={testId}
      data-unavailable={unavailable ? 'true' : undefined}
    >
      <span className="flex-1 text-left">{label}</span>
      {unavailable ? (
        <span className="text-xs" data-testid={`${testId}-unavailable`}>
          {t('sso.entry.methodUnavailable', 'Not available right now')}
        </span>
      ) : null}
    </Button>
  )
}

function RegisterForm({ service, entryId, onCancel }: { service: EntryService; entryId: string; onCancel: () => void }) {
  const { t } = useI18n()
  const [userId, setUserId] = useState('')
  const [showName, setShowName] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [fieldError, setFieldError] = useState<{ field: 'userId' | 'password' | 'confirm'; text: string } | null>(null)
  const [failure, setFailure] = useState<Failure | null>(null)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    // Hashed with the exact account name the server stores.
    const id = userId.trim().toLowerCase()
    if (!USER_ID_PATTERN.test(id) || RESERVED_USER_IDS.has(id)) {
      setFieldError({ field: 'userId', text: t('sso.register.userIdInvalid', 'Use 1-63 lowercase letters, digits or hyphens (root, system, admin and guest are reserved).') })
      return
    }
    if (password.length < MIN_PASSWORD_LENGTH) {
      setFieldError({ field: 'password', text: t('sso.firstPassword.tooShort', 'Use at least {{count}} characters.', { count: MIN_PASSWORD_LENGTH }) })
      return
    }
    if (password !== confirm) {
      setFieldError({ field: 'confirm', text: t('sso.firstPassword.mismatch', 'The two passwords do not match.') })
      return
    }
    setFieldError(null)
    setFailure(null)
    setBusy(true)
    const { data, error } = await registerAccount({
      service,
      entryId,
      userId: id,
      showName: showName.trim() || undefined,
      passwordHash: buckyos.hashPassword(id, password),
    })
    if (data?.status === 'created') {
      try {
        window.location.assign(await loginWithPassword(data.user_id, password, joinUrl()))
      } catch (loginError) {
        setFailure(failureOf(loginError))
        setBusy(false)
      }
      return
    }
    setBusy(false)
    if (data?.status === 'name_taken') setFieldError({ field: 'userId', text: t('sso.register.nameTaken', 'This user name is taken. Choose another one.') })
    else if (data?.status === 'denied') setFailure({ kind: 'denied', denial: data.denial })
    else setFailure(failureOf(error))
  }

  return (
    <form className="space-y-3" onSubmit={(event) => void submit(event)} noValidate data-testid="sso-register-form">
      <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('sso.register.hint', 'Create an account on {{zone}}. It is only for this Zone and does not change any identity you have elsewhere.', { zone: currentZoneHost() })}
      </p>
      <TextField
        fullWidth
        size="small"
        label={t('sso.register.userId', 'User name')}
        autoComplete="username"
        value={userId}
        disabled={busy}
        error={fieldError?.field === 'userId'}
        helperText={fieldError?.field === 'userId' ? fieldError.text : undefined}
        onChange={(event) => setUserId(event.target.value)}
      />
      <TextField
        fullWidth
        size="small"
        label={t('sso.register.showName', 'Display name (optional)')}
        autoComplete="nickname"
        value={showName}
        disabled={busy}
        onChange={(event) => setShowName(event.target.value)}
      />
      <TextField
        fullWidth
        size="small"
        type="password"
        autoComplete="new-password"
        label={t('sso.register.password', 'Password')}
        value={password}
        disabled={busy}
        error={fieldError?.field === 'password'}
        helperText={fieldError?.field === 'password' ? fieldError.text : undefined}
        onChange={(event) => setPassword(event.target.value)}
      />
      <TextField
        fullWidth
        size="small"
        type="password"
        autoComplete="new-password"
        label={t('sso.register.confirm', 'Confirm password')}
        value={confirm}
        disabled={busy}
        error={fieldError?.field === 'confirm'}
        helperText={fieldError?.field === 'confirm' ? fieldError.text : undefined}
        onChange={(event) => setConfirm(event.target.value)}
      />
      {failure ? <FailureNotice failure={failure} identity={null} /> : null}
      <div className="flex gap-2">
        <Button type="submit" variant="contained" disabled={busy} data-testid="sso-register-submit">
          {busy ? <CircularProgress size={18} color="inherit" /> : t('sso.register.submit', 'Create account and continue')}
        </Button>
        <Button variant="text" disabled={busy} onClick={onCancel}>
          {t('sso.common.cancel', 'Cancel')}
        </Button>
      </div>
    </form>
  )
}

type Loaded = { kind: 'loading' } | { kind: 'failed' } | { kind: 'ready'; descriptor: EntryDescriptor }

export function EntryPage() {
  const { t } = useI18n()
  const [searchParams] = useSearchParams()
  const service = parseEntryService(searchParams.get('service'))
  const entryId = searchParams.get('entry')?.trim() ?? ''
  const [join] = useState(() => searchParams.get('join') === '1' && hasPendingJoin())
  const { identity, reload } = useCurrentIdentity()
  const [loaded, setLoaded] = useState<Loaded>({ kind: 'loading' })
  const [loadSeq, setLoadSeq] = useState(0)
  const [panel, setPanel] = useState<'methods' | 'register' | 'did' | 'switch'>('methods')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<Failure | null>(null)
  const joinAttempted = useRef(false)

  useDocumentTitle(t('sso.entry.documentTitle', 'Shared link'))

  useEffect(() => {
    if (!service || !entryId) return
    let live = true
    void describeEntry(service, entryId).then(({ data }) => {
      if (live) setLoaded(data ? { kind: 'ready', descriptor: data } : { kind: 'failed' })
    })
    return () => { live = false }
  }, [service, entryId, loadSeq])

  const descriptor = loaded.kind === 'ready' ? loaded.descriptor : null
  const resource = descriptor?.state === 'active' ? descriptor.resource : undefined
  const signedIn = !!identity && identity.kind !== 'anonymous'

  /** Enter the resource as the signed-in identity (AI Workspace records the visitor on the entry first). */
  const enter = useCallback(async () => {
    if (!resource || !service) return
    if (service === 'aiworkspace') {
      const { data, error } = await joinWorkspaceEntry(entryId)
      if (!data) {
        setFailure(failureOf(error))
        setBusy(false)
        return
      }
    }
    window.location.assign(resource.path)
  }, [entryId, resource, service])

  useEffect(() => {
    if (!join || !signedIn || !resource || joinAttempted.current) return
    joinAttempted.current = true
    clearPendingJoin()
    void enter()
  }, [enter, join, resource, signedIn])

  const switchIdentity = async () => {
    setBusy(true)
    try {
      await endSessionInPlace()
    } catch {
      setBusy(false)
      setFailure({ kind: 'failed' })
      return
    }
    setBusy(false)
    setPanel('methods')
    reload()
  }

  const methodLabels: Record<EntryMethod, string> = useMemo(() => ({
    guest: t('sso.entry.method.guest', 'Continue as a guest'),
    password: t('sso.entry.method.password', 'Sign in with an account of this Zone'),
    self_register: t('sso.entry.method.selfRegister', 'Create an account here'),
    did_login: t('sso.entry.method.didLogin', 'Use an identity I already have (DID)'),
  }), [t])

  const kindLabel = (kind: string) => {
    if (kind === 'workspace') return t('sso.entry.kind.workspace', 'AI Workspace')
    if (kind === 'group' || kind === 'group_session') return t('sso.entry.kind.group', 'Message Hub conversation')
    return kind
  }

  let body: ReactNode
  if (!service || !entryId || descriptor?.state === 'unavailable') {
    body = (
      <Alert severity="info" data-testid="sso-entry-unavailable">
        {t('sso.entry.unavailable', 'This link is not available. Contact the person who shared it.')}
      </Alert>
    )
  } else if (loaded.kind === 'failed') {
    body = (
      <Alert
        severity="warning"
        action={<Button size="small" onClick={() => { setLoaded({ kind: 'loading' }); setLoadSeq((seq) => seq + 1) }}>{t('sso.common.retry', 'Retry')}</Button>}
        data-testid="sso-entry-load-failed"
      >
        {t('sso.entry.loadFailed', 'This link cannot be read right now. Try again later.')}
      </Alert>
    )
  } else if (!resource || !descriptor || !identity) {
    body = <div className="flex justify-center py-6"><CircularProgress size={28} /></div>
  } else if (signedIn && panel !== 'switch') {
    const name = identityDisplayName(identity)
    // Coming back from a sign-in (`join=1`) enters on its own; the button shows it is under way.
    const joining = busy || (join && !failure)
    body = (
      <div className="space-y-3" data-testid="sso-entry-signed-in">
        <IdentityCard identity={identity} />
        {failure ? <FailureNotice failure={failure} identity={identity} /> : null}
        <Button
          fullWidth
          variant="contained"
          endIcon={joining ? undefined : <ArrowRight size={16} />}
          disabled={joining}
          onClick={() => { setBusy(true); setFailure(null); void enter() }}
          data-testid="sso-entry-continue"
        >
          {joining ? <CircularProgress size={18} color="inherit" /> : t('sso.entry.continueAs', 'Continue as {{name}}', { name })}
        </Button>
        <Button fullWidth variant="text" disabled={joining} onClick={() => setPanel('switch')} data-testid="sso-entry-switch">
          {t('sso.entry.switch', 'Use another identity')}
        </Button>
      </div>
    )
  } else if (signedIn) {
    body = (
      <div className="space-y-3" data-testid="sso-entry-switch-confirm">
        <Alert severity="info">
          {t('sso.entry.switchConfirm', 'You will be signed out as {{name}} on this Zone, then you can choose another way in.', { name: identityDisplayName(identity) })}
        </Alert>
        {failure ? <FailureNotice failure={failure} identity={identity} /> : null}
        <div className="flex gap-2">
          <Button variant="contained" disabled={busy} onClick={() => void switchIdentity()} data-testid="sso-entry-switch-confirm-button">
            {t('sso.entry.switchConfirmButton', 'Sign out and switch')}
          </Button>
          <Button variant="text" disabled={busy} onClick={() => setPanel('methods')} data-testid="sso-entry-switch-cancel">
            {t('sso.common.cancel', 'Cancel')}
          </Button>
        </div>
      </div>
    )
  } else if (panel === 'register') {
    body = <RegisterForm service={service} entryId={entryId} onCancel={() => setPanel('methods')} />
  } else if (panel === 'did') {
    body = <DidLoginForm redirectUrl={joinUrl()} onCancel={() => setPanel('methods')} />
  } else {
    const { methods } = descriptor
    const closed = new Set(descriptor.zone_disabled ?? [])
    const offered = (['guest', 'password', 'self_register', 'did_login'] as const).filter((method) => methods[method] || closed.has(method))
    const open = (method: EntryMethod) => {
      if (method === 'guest') window.location.assign(resource.path)
      else if (method === 'password') window.location.assign(`/login?redirect_url=${encodeURIComponent(joinUrl())}`)
      else if (method === 'self_register') setPanel('register')
      else setPanel('did')
    }
    const icons: Record<EntryMethod, ReactNode> = {
      guest: <UserRound size={16} />,
      password: <LogIn size={16} />,
      self_register: <UserPlus size={16} />,
      did_login: <Fingerprint size={16} />,
    }
    body = offered.length === 0 ? (
      <Alert severity="info" data-testid="sso-entry-no-methods">
        {t('sso.entry.noMethods', 'This link offers no way in right now. Contact the person who shared it.')}
      </Alert>
    ) : (
      <div className="space-y-2" data-testid="sso-entry-methods">
        {offered.map((method) => (
          <MethodButton
            key={method}
            icon={icons[method]}
            label={methodLabels[method]}
            unavailable={!methods[method]}
            onClick={() => open(method)}
            testId={`sso-entry-method-${method}`}
          />
        ))}
      </div>
    )
  }

  return (
    <AuthPageFrame
      testId="sso-entry-page"
      title={resource?.title ?? t('sso.entry.title', 'Shared link')}
      header={resource ? (
        <div className="mt-1 text-center text-sm" style={{ color: 'var(--cp-muted)' }} data-testid="sso-entry-resource">
          <div>{kindLabel(resource.kind)}{resource.owner_name ? ` · ${t('sso.entry.sharedBy', 'shared by {{name}}', { name: resource.owner_name })}` : ''}</div>
          <div className="text-xs">{t('sso.entry.onZone', 'on {{zone}}', { zone: currentZoneHost() })}</div>
        </div>
      ) : undefined}
    >
      {body}
    </AuthPageFrame>
  )
}
