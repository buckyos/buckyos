import { buckyos } from 'buckyos'
import { useI18n } from '../../i18n/provider'
import { isMockRuntime } from '../../runtime'
import type { AppContentLoaderProps } from '../types'
import { desktopCatalogIdForLogicalApp } from '../backend-apps'

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

export function SystestAppPanel({ app }: AppContentLoaderProps) {
  const { t } = useI18n()
  const src = buildAppWebUrl(app)

  if (!src) {
    return (
      <div className="shell-subtle-panel p-4">
        <p>{t('common.unsupportedPanel', app.id)}</p>
      </div>
    )
  }

  const isSystest = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id) === 'systest'

  return (
    <iframe
      className="block h-full w-full border-0 bg-white"
      data-testid={isSystest ? 'systest-frame' : 'web-app-frame'}
      src={src}
      title={t(app.labelKey)}
    />
  )
}
