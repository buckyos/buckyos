import { buckyos } from 'buckyos'
import { useI18n } from '../../i18n/provider'
import { isMockRuntime } from '../../runtime'
import type { AppContentLoaderProps } from '../types'

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

function buildSystestUrl() {
  const zoneHost = resolveZoneHost()
  const port = window.location.port ? `:${window.location.port}` : ''
  return `${window.location.protocol}//systest.${zoneHost}${port}/`
}

export function SystestAppPanel({ app }: AppContentLoaderProps) {
  const { t } = useI18n()

  return (
    <iframe
      className="block h-full w-full border-0 bg-white"
      data-testid="systest-frame"
      src={buildSystestUrl()}
      title={t(app.labelKey, app.id)}
    />
  )
}
