/* ── App Service Home Page ── */

import { useState } from 'react'
import {
  Plus,
  Play,
  AlertTriangle,
  Download,
  Square,
  ChevronRight,
  Loader2,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import type { AppServiceItem } from '../mock/types'
import type { AppServiceNav } from '../components/layout/navigation'
import { AppIcon } from '../../../components/DesktopVisuals'

function statusIcon(status: AppServiceItem['status']) {
  switch (status) {
    case 'running':
      return <Play size={12} />
    case 'starting':
      return <Loader2 size={12} className="animate-spin" />
    case 'stopped':
      return <Square size={12} />
    case 'error':
      return <AlertTriangle size={12} />
    case 'installing':
      return <Download size={12} />
    default:
      return <Square size={12} />
  }
}

function statusColor(status: AppServiceItem['status']) {
  switch (status) {
    case 'running':
      return 'var(--cp-success)'
    case 'starting':
      return 'var(--cp-accent)'
    case 'stopped':
      return 'var(--cp-muted)'
    case 'error':
      return 'var(--cp-danger)'
    case 'installing':
      return 'var(--cp-warning)'
    default:
      return 'var(--cp-muted)'
  }
}

function statusLabel(status: AppServiceItem['status'], t: (k: string, f: string) => string) {
  switch (status) {
    case 'running':
      return t('appService.status.running', 'Running')
    case 'starting':
      return t('appService.status.starting', 'Starting')
    case 'stopped':
      return t('appService.status.stopped', 'Stopped')
    case 'error':
      return t('appService.status.error', 'Error')
    case 'installing':
      return t('appService.status.installing', 'Installing')
    default:
      return status
  }
}

/* ── App Card (App Store style, width controlled by grid parent) ── */

function statusBadgeStyle(status: AppServiceItem['status']): React.CSSProperties {
  const color = statusColor(status)
  return {
    background: `color-mix(in srgb, ${color} 12%, transparent)`,
    color,
    border: `1px solid color-mix(in srgb, ${color} 25%, transparent)`,
  }
}

function AppCard({
  service,
  onOpen,
}: {
  service: AppServiceItem
  onOpen: () => void
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="w-full rounded-2xl text-left transition-all hover:shadow-lg hover:-translate-y-0.5"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
        overflow: 'hidden',
      }}
    >
      {/* Top section with icon + info */}
      <div className="p-4 pb-3">
        <div className="flex items-start gap-3.5">
          {/* Large rounded icon */}
          <div
            className="flex h-16 w-16 shrink-0 items-center justify-center rounded-[18px] shadow-sm"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface-2))',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <AppIcon iconKey={service.iconKey} className="!size-8" />
          </div>
          {/* Name + description + version */}
          <div className="flex-1 min-w-0 pt-0.5">
            <div
              className="text-[15px] font-semibold truncate leading-tight"
              style={{ color: 'var(--cp-text)' }}
            >
              {service.name}
            </div>
            <div
              className="text-xs mt-1 line-clamp-2 leading-relaxed"
              style={{ color: 'var(--cp-muted)' }}
            >
              {service.description}
            </div>
            <div className="text-[11px] mt-1.5" style={{ color: 'var(--cp-muted)' }}>
              v{service.version}
            </div>
          </div>
          {/* Status badge */}
          <div
            className="shrink-0 flex items-center gap-1.5 rounded-full px-2.5 py-1"
            style={statusBadgeStyle(service.status)}
          >
            <span className="flex items-center">{statusIcon(service.status)}</span>
            <span className="text-[11px] font-semibold whitespace-nowrap">
              {statusLabel(service.status, (_key, fallback) => fallback)}
            </span>
          </div>
        </div>
      </div>

      {/* Install progress bar */}
      {service.status === 'installing' && service.installProgress != null && (
        <div className="px-4 pb-3">
          <div
            className="h-1.5 w-full rounded-full overflow-hidden"
            style={{ background: 'var(--cp-border)' }}
          >
            <div
              className="h-full rounded-full transition-all"
              style={{
                width: `${service.installProgress}%`,
                background: 'var(--cp-warning)',
              }}
            />
          </div>
          <div className="text-[11px] mt-1 text-right" style={{ color: 'var(--cp-muted)' }}>
            {service.installProgress}%
          </div>
        </div>
      )}

      {/* Bottom action hint */}
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{
          borderTop: '1px solid var(--cp-border)',
          background: 'color-mix(in srgb, var(--cp-surface-2) 50%, var(--cp-surface))',
        }}
      >
        <span className="text-[11px] font-medium" style={{ color: 'var(--cp-muted)' }}>
          {service.docker ? service.docker.imageName : 'Native Service'}
        </span>
        <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
      </div>
    </button>
  )
}

/* ── Service Row (for system/kernel layer) ── */

function ServiceRow({
  service,
  onOpen,
}: {
  service: AppServiceItem
  onOpen: () => void
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors hover:brightness-[1.02]"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <span className="text-sm font-medium flex-1 truncate" style={{ color: 'var(--cp-text)' }}>
        {service.name}
      </span>
      <div className="flex items-center gap-1.5 shrink-0">
        <span style={{ color: statusColor(service.status) }}>{statusIcon(service.status)}</span>
        <span
          className="text-xs font-medium"
          style={{ color: statusColor(service.status) }}
        >
          {statusLabel(service.status, (_key, fallback) => fallback)}
        </span>
      </div>
      <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
    </button>
  )
}

/* ── Section Header ── */

function SectionHeader({
  title,
  count,
}: {
  title: string
  count: number
}) {
  return (
    <h2
      className="text-xs font-semibold uppercase tracking-wide mb-2"
      style={{ color: 'var(--cp-muted)' }}
    >
      {title} ({count})
    </h2>
  )
}

/* ── Home Page ── */

interface HomePageProps {
  onNavigate: (nav: AppServiceNav) => void
}

export function HomePage({ onNavigate }: HomePageProps) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const [, setTick] = useState(0)

  const appServices = store.getByLayer('app')
  const systemServices = store.getByLayer('system')
  const kernelServices = store.getByLayer('kernel')

  const handleOpen = (id: string) => {
    onNavigate({ page: 'detail', serviceId: id })
  }

  const handleInstall = () => {
    onNavigate({ page: 'install', installStep: 1 })
  }

  // Auto-refresh for installing/starting states
  const hasActiveStates = [...appServices, ...systemServices, ...kernelServices].some(
    (s) => s.status === 'installing' || s.status === 'starting',
  )
  if (hasActiveStates) {
    setTimeout(() => setTick((n) => n + 1), 2000)
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.title', 'App Service')}
          </h1>
          <p className="text-sm mt-0.5" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.subtitle', 'System service control panel')}
          </p>
        </div>
        <button
          type="button"
          onClick={handleInstall}
          className="flex items-center gap-2 rounded-xl px-4 py-2.5 text-sm font-medium transition-colors"
          style={{
            background: 'var(--cp-accent)',
            color: 'white',
          }}
        >
          <Plus size={16} />
          {t('appService.install', 'Install')}
        </button>
      </div>

      {/* App Layer */}
      {appServices.length > 0 && (
        <section>
          <SectionHeader
            title={t('appService.layer.apps', 'Running Apps')}
            count={appServices.length}
          />
          <div
            className="grid gap-4"
            style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(375px, 100%), 1fr))' }}
          >
            {appServices.map((svc) => (
              <AppCard key={svc.id} service={svc} onOpen={() => handleOpen(svc.id)} />
            ))}
          </div>
        </section>
      )}

      {/* System Services Layer */}
      {systemServices.length > 0 && (
        <section>
          <SectionHeader
            title={t('appService.layer.system', 'System Services')}
            count={systemServices.length}
          />
          <div className="space-y-1.5">
            {systemServices.map((svc) => (
              <ServiceRow key={svc.id} service={svc} onOpen={() => handleOpen(svc.id)} />
            ))}
          </div>
        </section>
      )}

      {/* Kernel Layer */}
      {kernelServices.length > 0 && (
        <section>
          <SectionHeader
            title={t('appService.layer.kernel', 'Kernel')}
            count={kernelServices.length}
          />
          <div className="space-y-1.5">
            {kernelServices.map((svc) => (
              <ServiceRow key={svc.id} service={svc} onOpen={() => handleOpen(svc.id)} />
            ))}
          </div>
        </section>
      )}
    </div>
  )
}
