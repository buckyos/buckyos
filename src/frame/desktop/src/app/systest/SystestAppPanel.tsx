import { useEffect, useRef, useState } from 'react'
import { AppFrameHost } from 'buckyos/app-frame'
import { buckyos } from 'buckyos'
import { useI18n } from '../../i18n/provider'
import { useAppText } from '../../presentation/appText'
import { isMockRuntime } from '../../runtime'
import type { AppContentLoaderProps } from '../types'
import { desktopCatalogIdForLogicalApp } from '../backend-apps'
import { useAppLaunch } from '../web-frame/launch'
import { AppLaunchStatus } from '../web-frame/AppLaunchStatus'
import { useZoneAccent, zoneAccent } from '../../desktop/appearance/accent'

function resolveZoneHost() {
  let sdkZoneHost: string | null = null
  if (!isMockRuntime()) {
    try {
      sdkZoneHost = buckyos.getZoneHostName()?.trim() ?? null
    } catch {
      sdkZoneHost = null
    }
  }
  if (sdkZoneHost) {
    return sdkZoneHost
  }

  const currentHost = window.location.hostname.toLowerCase()
  if (currentHost.startsWith('sys.')) {
    return currentHost.slice(4)
  }
  if (currentHost === '127.0.0.1' || currentHost === '::1') {
    return 'localhost'
  }
  return currentHost
}

function resolveAppWebHost(app: AppContentLoaderProps['app']) {
  const configuredHost = app.webHosts?.[0]?.trim().toLowerCase()
  if (configuredHost) {
    return configuredHost
  }
  const catalogId = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id)
  return catalogId === 'systest' ? 'systest' : null
}

function buildAppWebUrl(app: AppContentLoaderProps['app']) {
  const appHost = resolveAppWebHost(app)
  if (!appHost || !/^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/.test(appHost)) {
    return null
  }
  const zoneHost = resolveZoneHost()
  const port = window.location.port ? `:${window.location.port}` : ''
  return `${window.location.protocol}//${appHost}.${zoneHost}${port}/`
}

export function SystestAppPanel({ app, windowId, themeMode }: AppContentLoaderProps) {
  const { locale, t } = useI18n()
  const appText = useAppText()
  const isSystest = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id) === 'systest'
  const [nonce] = useState(() => crypto.randomUUID())
  const [initialLocale] = useState(locale)
  const accent = useZoneAccent()
  const iframe = useRef<HTMLIFrameElement>(null)
  const host = useRef<AppFrameHost | null>(null)
  const base = buildAppWebUrl(app)
  const src = isSystest && base ? `${base}?locale=${initialLocale}#bfp=${nonce}` : base
  const launch = useAppLaunch(app.appInstanceId ?? app.id, src ?? undefined)

  useEffect(() => {
    if (!base || !isSystest) return
    const frame = new AppFrameHost({
      target: () => iframe.current?.contentWindow ?? null,
      origin: new URL(base).origin,
      init: { nonce, windowId: windowId ?? app.id, locale: initialLocale, theme: { mode: 'light', accent: zoneAccent() },
        formFactor: matchMedia('(max-width: 900px)').matches ? 'mobile' : 'desktop', shellCapabilities: [] },
    })
    host.current = frame
    return () => { frame.dispose(); host.current = null }
  }, [app.id, base, initialLocale, isSystest, nonce, windowId])
  useEffect(() => {
    host.current?.update({ locale, theme: { mode: themeMode, accent } })
    host.current?.send('frame.localeChanged', { locale })
  }, [base, isSystest, locale, themeMode, accent])

  if (!src) {
    return (
      <div className="shell-subtle-panel p-4">
        <p>{t('common.unsupportedPanel', app.id)}</p>
      </div>
    )
  }

  if (launch.outcome?.kind !== 'ready') {
    return <AppLaunchStatus outcome={launch.outcome} title={appText(app).title} onRetry={launch.retry} />
  }

  return (
    <iframe
      ref={iframe}
      className="block h-full w-full border-0 bg-white"
      data-testid={isSystest ? 'systest-frame' : 'web-app-frame'}
      src={launch.outcome.url}
      title={appText(app).title}
    />
  )
}
