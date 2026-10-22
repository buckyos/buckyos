/* ── `/` for a restricted identity (Limited account or External DID): no full Desktop ──
 *
 * Shows who is signed in, the first-password entry when it applies, the Apps a Limited account may
 * open and a way to sign out. Shared resources are reached through the links the visitor received.
 */

import { useEffect, useState } from 'react'
import { Alert, Button, CircularProgress } from '@mui/material'
import { AppWindow, LogOut } from 'lucide-react'
import type { CurrentIdentity } from 'buckyos'
import { requestAppLaunch } from '../api/app_launch'
import { fetchAppList, type AppSummary } from '../api/app_mgr'
import { backendAppText, DESKTOP_BUILTIN_APP_IDS } from '../app/backend-apps'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { signOutToLogin } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { DEFAULT_PRESENTATION_LOCALE, pickLocalized } from '../presentation/model'
import { currentZoneHost } from '../presentation/zone'
import { isMockRuntime } from '../runtime'
import { FirstPasswordPrompt } from './FirstPasswordPrompt'
import { IdentityCard } from './IdentityCard'
import { getBootIdentity } from './identity'

/** Web Apps the account may open: what `apps.list` returns for it, without Desktop built-ins and Agent Apps. */
function openableApps(apps: AppSummary[]): AppSummary[] {
  return apps.filter((app) =>
    app.runtime_type !== 'agent'
    && !DESKTOP_BUILTIN_APP_IDS.has(app.app_id)
    && app.web_hosts.some((host) => host.trim()))
}

function appUrl(app: AppSummary): string {
  const host = app.web_hosts.map((item) => item.trim()).find(Boolean) ?? ''
  const zone = currentZoneHost()
  return `${window.location.protocol}//${host.endsWith(zone) ? host : `${host}.${zone}`}/`
}

function appTitle(app: AppSummary, locale: string): string {
  const title = backendAppText(app, false).title
  return (title && (pickLocalized(title, locale) ?? pickLocalized(title, DEFAULT_PRESENTATION_LOCALE))) || app.app_id
}

/** The Limited account's Apps; omitted when the list cannot be read for it. */
function RestrictedApps() {
  const { t, locale } = useI18n()
  const [apps, setApps] = useState<AppSummary[] | null>(null)
  const [opening, setOpening] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    if (isMockRuntime()) return
    let live = true
    void fetchAppList().then(({ data }) => {
      if (live && data) setApps(openableApps(data.apps))
    })
    return () => { live = false }
  }, [])

  if (!apps || apps.length === 0) return null

  const open = async (app: AppSummary) => {
    setOpening(app.app_instance_id)
    setFailed(false)
    const result = await requestAppLaunch(app.app_instance_id, appUrl(app))
    if (result?.status === 'ready') {
      window.location.assign(result.callback_url)
      return
    }
    setOpening(null)
    setFailed(true)
  }

  return (
    <div className="space-y-2" data-testid="restricted-home-apps">
      <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('sso.restricted.apps', 'Apps you can open')}</h2>
      {failed ? <Alert severity="error">{t('sso.restricted.appFailed', 'The App could not be opened. Try again later.')}</Alert> : null}
      {apps.map((app) => (
        <Button
          key={app.app_instance_id}
          fullWidth
          variant="outlined"
          startIcon={opening === app.app_instance_id ? <CircularProgress size={14} /> : <AppWindow size={16} />}
          disabled={opening !== null}
          onClick={() => void open(app)}
          sx={{ justifyContent: 'flex-start' }}
        >
          {appTitle(app, locale)}
        </Button>
      ))}
    </div>
  )
}

export function RestrictedHome({ identity = getBootIdentity() }: { identity?: CurrentIdentity }) {
  const { t } = useI18n()
  const [signingOut, setSigningOut] = useState(false)
  const [signOutFailed, setSignOutFailed] = useState(false)

  useDocumentTitle(t('sso.restricted.documentTitle', 'Home'))

  const signOut = async () => {
    setSigningOut(true)
    setSignOutFailed(false)
    try {
      await signOutToLogin()
    } catch {
      setSigningOut(false)
      setSignOutFailed(true)
    }
  }

  return (
    <AuthPageFrame testId="restricted-home" maxWidth="sm" title={t('sso.restricted.title', 'Welcome')}>
      <div className="space-y-4">
        <IdentityCard identity={identity} />
        <FirstPasswordPrompt identity={identity} />
        <p className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }} data-testid="restricted-home-hint">
          {identity.kind === 'external'
            ? t('sso.restricted.hintExternal', 'You signed in with an identity from another Zone. Open the links you were sent to reach what was shared with you here.')
            : t('sso.restricted.hintLimited', 'This account can use what was shared with it. Open the links you were sent to reach your conversations and documents.')}
        </p>
        {identity.kind === 'account' ? <RestrictedApps /> : null}
        {signOutFailed ? <Alert severity="error">{t('shell.logoutFailed', 'Log out failed. Please try again.')}</Alert> : null}
        <Button fullWidth variant="outlined" startIcon={<LogOut size={16} />} disabled={signingOut} onClick={() => void signOut()} data-testid="restricted-home-sign-out">
          {signingOut ? t('shell.loggingOut', 'Logging out...') : t('shell.logout', 'Log out')}
        </Button>
      </div>
    </AuthPageFrame>
  )
}
