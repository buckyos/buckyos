/* ── App Service Detail Page ── */

import { useState } from 'react'
import { useMediaQuery } from '@mui/material'
import {
  ArrowLeft,
  Play,
  Square,
  CheckCircle2,
  AlertTriangle,
  Server,
  HardDrive,
  Container,
  ChevronDown,
  ChevronUp,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import { AppIcon } from '../../../components/DesktopVisuals'
import type { AppServiceNav } from '../components/layout/navigation'

function statusColor(status: string) {
  switch (status) {
    case 'running':
      return 'var(--cp-success)'
    case 'starting':
      return 'var(--cp-accent)'
    case 'stopped':
      return 'var(--cp-muted)'
    case 'error':
    case 'not_running':
    case 'missing':
      return 'var(--cp-danger)'
    case 'installing':
    case 'pulling':
      return 'var(--cp-warning)'
    case 'present':
      return 'var(--cp-success)'
    case 'not_created':
      return 'var(--cp-muted)'
    default:
      return 'var(--cp-muted)'
  }
}

function StatusDot({ color }: { color: string }) {
  return (
    <span
      className="inline-block h-2 w-2 rounded-full"
      style={{ background: color }}
    />
  )
}

/* ── Detail Page ── */

interface DetailPageProps {
  serviceId: string
  onNavigate: (nav: AppServiceNav) => void
}

export function DetailPage({ serviceId, onNavigate }: DetailPageProps) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [, setTick] = useState(0)
  const [specOpen, setSpecOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)

  const service = store.getById(serviceId)

  if (!service) {
    return (
      <div className="text-center py-12 text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('appService.detail.notFound', 'Service not found')}
      </div>
    )
  }

  const handleBack = () => onNavigate({ page: 'home' })

  const handleStart = () => {
    store.startService(serviceId)
    setTick((n) => n + 1)
    // Poll for state change
    const timer = setInterval(() => setTick((n) => n + 1), 500)
    setTimeout(() => clearInterval(timer), 3000)
  }

  const handleStop = () => {
    store.stopService(serviceId)
    setTick((n) => n + 1)
  }

  const canStart = service.status === 'stopped'
  const canStop = service.status === 'running' || service.status === 'starting'

  return (
    <div className="space-y-5">
      {/* Back button – hidden on mobile where title bar provides back */}
      {!isMobile && (
        <button
          type="button"
          onClick={handleBack}
          className="flex items-center gap-1.5 text-sm font-medium transition-colors"
          style={{ color: 'var(--cp-muted)' }}
        >
          <ArrowLeft size={16} />
          {t('appService.detail.back', 'Back')}
        </button>
      )}

      {/* Header */}
      <div
        className="rounded-2xl p-5"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div className="flex items-center gap-4">
          <div
            className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface-2))',
              color: 'var(--cp-text)',
            }}
          >
            <AppIcon iconKey={service.iconKey} className="!size-6" />
          </div>
          <div className="flex-1 min-w-0">
            <h1 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
              {service.name}
            </h1>
            <div className="flex items-center gap-2 mt-0.5">
              <StatusDot color={statusColor(service.status)} />
              <span
                className="text-sm font-medium capitalize"
                style={{ color: statusColor(service.status) }}
              >
                {service.status}
              </span>
              <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                v{service.version}
              </span>
            </div>
          </div>

          {/* Control buttons */}
          <div className="flex items-center gap-2 shrink-0">
            <button
              type="button"
              disabled={!canStart}
              onClick={handleStart}
              className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-40"
              style={{
                background: canStart ? 'var(--cp-success)' : 'var(--cp-surface-2)',
                color: canStart ? 'white' : 'var(--cp-muted)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <Play size={12} />
              Start
            </button>
            <button
              type="button"
              disabled={!canStop}
              onClick={handleStop}
              className="flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-40"
              style={{
                background: canStop ? 'var(--cp-danger)' : 'var(--cp-surface-2)',
                color: canStop ? 'white' : 'var(--cp-muted)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <Square size={12} />
              Stop
            </button>
          </div>
        </div>
        {service.description && (
          <p className="text-sm mt-3" style={{ color: 'var(--cp-muted)' }}>
            {service.description}
          </p>
        )}
      </div>

      {/* Status Overview */}
      <section>
        <h2
          className="text-xs font-semibold uppercase tracking-wide mb-2"
          style={{ color: 'var(--cp-muted)' }}
        >
          {t('appService.detail.statusOverview', 'Status Overview')}
        </h2>
        <div
          className="rounded-2xl p-4 space-y-3"
          style={{
            background: 'var(--cp-surface)',
            border: '1px solid var(--cp-border)',
          }}
        >
          {/* App status */}
          <div className="flex items-center justify-between">
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              App Status
            </span>
            <div className="flex items-center gap-1.5">
              <StatusDot color={statusColor(service.status)} />
              <span className="text-sm font-medium capitalize" style={{ color: statusColor(service.status) }}>
                {service.status}
              </span>
            </div>
          </div>

          {/* Docker dependency chain */}
          {service.docker && (
            <>
              <div
                className="border-t"
                style={{ borderColor: 'var(--cp-border)' }}
              />
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Server size={14} style={{ color: 'var(--cp-muted)' }} />
                  <span className="text-sm" style={{ color: 'var(--cp-text)' }}>Docker Engine</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <StatusDot color={statusColor(service.docker.engine)} />
                  <span className="text-sm font-medium capitalize" style={{ color: statusColor(service.docker.engine) }}>
                    {service.docker.engine === 'running' ? 'Running' : 'Not Running'}
                  </span>
                </div>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <HardDrive size={14} style={{ color: 'var(--cp-muted)' }} />
                  <span className="text-sm" style={{ color: 'var(--cp-text)' }}>Image</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <StatusDot color={statusColor(service.docker.image)} />
                  <span className="text-sm font-medium capitalize" style={{ color: statusColor(service.docker.image) }}>
                    {service.docker.image === 'present'
                      ? 'Present'
                      : service.docker.image === 'pulling'
                        ? 'Pulling...'
                        : 'Missing'}
                  </span>
                </div>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Container size={14} style={{ color: 'var(--cp-muted)' }} />
                  <span className="text-sm" style={{ color: 'var(--cp-text)' }}>Container</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <StatusDot color={statusColor(service.docker.container)} />
                  <span className="text-sm font-medium capitalize" style={{ color: statusColor(service.docker.container) }}>
                    {service.docker.container === 'not_created'
                      ? 'Not Created'
                      : service.docker.container}
                  </span>
                </div>
              </div>
            </>
          )}
        </div>
      </section>

      {/* Configuration – Spec */}
      {Object.keys(service.spec).length > 0 && (
        <section>
          <button
            type="button"
            onClick={() => setSpecOpen(!specOpen)}
            className="flex w-full items-center justify-between text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            <span>{t('appService.detail.spec', 'Spec')}</span>
            {specOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
          {specOpen && (
            <div
              className="rounded-2xl p-4 space-y-2"
              style={{
                background: 'var(--cp-surface)',
                border: '1px solid var(--cp-border)',
              }}
            >
              {Object.entries(service.spec).map(([key, value]) => (
                <div key={key} className="flex items-center justify-between">
                  <span className="text-sm font-mono" style={{ color: 'var(--cp-muted)' }}>
                    {key}
                  </span>
                  <span className="text-sm font-mono" style={{ color: 'var(--cp-text)' }}>
                    {value}
                  </span>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {/* Configuration – Settings */}
      {Object.keys(service.settings).length > 0 && (
        <section>
          <button
            type="button"
            onClick={() => setSettingsOpen(!settingsOpen)}
            className="flex w-full items-center justify-between text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            <span>{t('appService.detail.settings', 'Settings')}</span>
            {settingsOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
          {settingsOpen && (
            <div
              className="rounded-2xl p-4 space-y-2"
              style={{
                background: 'var(--cp-surface)',
                border: '1px solid var(--cp-border)',
              }}
            >
              {Object.entries(service.settings).map(([key, value]) => (
                <div key={key} className="flex items-center justify-between">
                  <span className="text-sm font-mono" style={{ color: 'var(--cp-muted)' }}>
                    {key}
                  </span>
                  <span className="text-sm font-mono" style={{ color: 'var(--cp-text)' }}>
                    {value}
                  </span>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {/* Runtime Info */}
      {service.docker && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('appService.detail.runtimeInfo', 'Runtime Info')}
          </h2>
          <div
            className="rounded-2xl p-4 space-y-2"
            style={{
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <div className="flex items-center justify-between">
              <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>Version</span>
              <span className="text-sm font-mono" style={{ color: 'var(--cp-text)' }}>
                {service.version}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>Image</span>
              <span className="text-sm font-mono truncate ml-4" style={{ color: 'var(--cp-text)' }}>
                {service.docker.imageName}
              </span>
            </div>
          </div>
        </section>
      )}

      {/* Diagnostics */}
      {service.diagnostics.length > 0 && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-danger)' }}
          >
            {t('appService.detail.diagnostics', 'Diagnostics')}
          </h2>
          <div
            className="rounded-2xl p-4 space-y-2"
            style={{
              background: 'color-mix(in srgb, var(--cp-danger) 5%, var(--cp-surface))',
              border: '1px solid color-mix(in srgb, var(--cp-danger) 20%, var(--cp-border))',
            }}
          >
            {service.diagnostics.map((msg, i) => (
              <div key={i} className="flex items-start gap-2">
                <AlertTriangle
                  size={14}
                  className="mt-0.5 shrink-0"
                  style={{ color: 'var(--cp-danger)' }}
                />
                <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
                  {msg}
                </span>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* No issues */}
      {service.diagnostics.length === 0 && service.status === 'running' && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('appService.detail.diagnostics', 'Diagnostics')}
          </h2>
          <div
            className="rounded-2xl p-4 flex items-center gap-2"
            style={{
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <CheckCircle2 size={14} style={{ color: 'var(--cp-success)' }} />
            <span className="text-sm" style={{ color: 'var(--cp-success)' }}>
              {t('appService.detail.noIssues', 'No issues detected')}
            </span>
          </div>
        </section>
      )}
    </div>
  )
}
