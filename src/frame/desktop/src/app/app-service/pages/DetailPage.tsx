import { useEffect, useState } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { useForm } from 'react-hook-form'
import {
  AlertTriangle,
  ArrowLeft,
  Box,
  CheckCircle2,
  CircleStop,
  Container,
  FileCode2,
  HardDrive,
  Loader2,
  Pencil,
  Play,
  Save,
  ScrollText,
  Server,
  Settings2,
  Square,
  X,
} from 'lucide-react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../../i18n/provider'
import { AppIcon } from '../../../components/DesktopVisuals'
import type { AppServiceNav } from '../components/layout/navigation'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import type { AppServiceItem } from '../mock/types'
import { settingsInputSchema, type SettingsInput } from '../schemas'

function stateColor(state: string) {
  switch (state) {
    case 'running':
    case 'present':
      return 'var(--cp-success)'
    case 'starting':
      return 'var(--cp-accent)'
    case 'installing':
    case 'pulling':
      return 'var(--cp-warning)'
    case 'error':
    case 'activation_failed':
    case 'not_running':
    case 'missing':
      return 'var(--cp-danger)'
    case 'stopped':
    case 'not_created':
    default:
      return 'var(--cp-muted)'
  }
}

function StateMark({ state }: { state: string }) {
  const color = stateColor(state)
  if (state === 'starting' || state === 'pulling' || state === 'installing') {
    return <Loader2 size={14} className="animate-spin" style={{ color }} aria-hidden="true" />
  }
  if (['error', 'activation_failed', 'not_running', 'missing'].includes(state)) {
    return <AlertTriangle size={14} style={{ color }} aria-hidden="true" />
  }
  if (state === 'running' || state === 'present') {
    return <CheckCircle2 size={14} style={{ color }} aria-hidden="true" />
  }
  return <CircleStop size={14} style={{ color }} aria-hidden="true" />
}

function stateLabel(state: string, t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.stateLabel.${state}`, state.replaceAll('_', ' '))
}

function StateValue({ state }: { state: string }) {
  const { t } = useI18n()
  return (
    <span className="inline-flex items-center gap-1.5 text-xs font-semibold" style={{ color: stateColor(state) }}>
      <StateMark state={state} />
      {stateLabel(state, t)}
    </span>
  )
}

function SectionTitle({ icon: Icon, children }: { icon: typeof Box; children: React.ReactNode }) {
  return (
    <div className="mb-3 flex items-center gap-2">
      <Icon size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
      <h2 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
        {children}
      </h2>
    </div>
  )
}

function DefinitionRows({ values }: { values: Record<string, string> }) {
  return (
    <dl className="divide-y" style={{ borderColor: 'var(--cp-border)' }}>
      {Object.entries(values).map(([key, value]) => (
        <div key={key} className="grid gap-1 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[minmax(120px,0.55fr)_minmax(0,1fr)] sm:gap-4">
          <dt className="text-xs" style={{ color: 'var(--cp-muted)' }}>{key}</dt>
          <dd className="min-w-0 break-words text-xs font-medium sm:text-right" style={{ color: 'var(--cp-text)' }}>
            {value}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function DependencyRow({
  icon: Icon,
  label,
  state,
  last,
}: {
  icon: typeof Server
  label: string
  state: string
  last?: boolean
}) {
  return (
    <div className="relative grid grid-cols-[32px_minmax(0,1fr)_auto] items-center gap-3 py-2.5">
      {!last && (
        <span
          className="absolute left-[15px] top-8 h-5 w-px"
          style={{ background: 'var(--cp-border)' }}
          aria-hidden="true"
        />
      )}
      <span
        className="flex size-8 items-center justify-center rounded-full"
        style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)', color: 'var(--cp-muted)' }}
      >
        <Icon size={14} aria-hidden="true" />
      </span>
      <span className="text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{label}</span>
      <StateValue state={state} />
    </div>
  )
}

function SettingsEditor({ service }: { service: AppServiceItem }) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const [editing, setEditing] = useState(false)
  const [saved, setSaved] = useState(false)
  const form = useForm<SettingsInput>({
    resolver: zodResolver(settingsInputSchema),
    defaultValues: service.settings,
    mode: 'onBlur',
  })

  useEffect(() => {
    form.reset(service.settings)
  }, [form, service.settings])

  const save = form.handleSubmit((values) => {
    store.updateSettings(service.id, values)
    setEditing(false)
    setSaved(true)
  })

  const cancel = () => {
    form.reset(service.settings)
    setEditing(false)
  }

  return (
    <section>
      <div className="mb-3 flex items-center justify-between gap-4">
        <div className="flex items-center gap-2">
          <Settings2 size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
          <h2 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.detail.settings', 'Settings')}
          </h2>
          {saved && !editing && (
            <span className="text-[11px] font-semibold" style={{ color: 'var(--cp-success)' }}>
              {t('appService.detail.settingsSaved', 'Saved')}
            </span>
          )}
        </div>
        {!editing && Object.keys(service.settings).length > 0 && (
          <button
            type="button"
            onClick={() => { setEditing(true); setSaved(false) }}
            className="inline-flex min-h-11 items-center gap-1.5 rounded-lg px-3 text-xs font-semibold"
            style={{ color: 'var(--cp-accent)', background: 'var(--cp-surface-2)' }}
          >
            <Pencil size={13} aria-hidden="true" />
            {t('appService.detail.editSettings', 'Edit settings')}
          </button>
        )}
      </div>

      <div className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
        {Object.keys(service.settings).length === 0 ? (
          <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.detail.noSettings', 'This application does not expose editable settings.')}
          </p>
        ) : editing ? (
          <form className="space-y-4" onSubmit={save} noValidate>
            {Object.keys(service.settings).map((key) => {
              const error = form.formState.errors[key]
              return (
                <label key={key} className="block">
                  <span className="mb-1.5 block text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{key}</span>
                  <input
                    {...form.register(key)}
                    className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                    aria-invalid={Boolean(error)}
                    aria-describedby={error ? `settings-error-${key}` : undefined}
                    style={{
                      color: 'var(--cp-text)',
                      background: 'var(--cp-bg)',
                      border: `1px solid ${error ? 'var(--cp-danger)' : 'var(--cp-border)'}`,
                    }}
                  />
                  {error && (
                    <span id={`settings-error-${key}`} className="mt-1 block text-xs" style={{ color: 'var(--cp-danger)' }}>
                      {t('appService.detail.settingRequired', 'Enter a value before saving.')}
                    </span>
                  )}
                </label>
              )
            })}
            <div className="flex flex-wrap justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={cancel}
                className="inline-flex min-h-11 items-center gap-1.5 rounded-xl px-3 text-xs font-semibold"
                style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
              >
                <X size={13} aria-hidden="true" />
                {t('common.cancel', 'Cancel')}
              </button>
              <button
                type="submit"
                className="inline-flex min-h-11 items-center gap-1.5 rounded-xl px-3 text-xs font-semibold"
                style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
              >
                <Save size={13} aria-hidden="true" />
                {t('appService.detail.saveSettings', 'Save settings')}
              </button>
            </div>
          </form>
        ) : (
          <DefinitionRows values={service.settings} />
        )}
      </div>
    </section>
  )
}

interface DetailPageProps {
  serviceId: string
  onNavigate: (nav: AppServiceNav) => void
}

export function DetailPage({ serviceId, onNavigate }: DetailPageProps) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [logsOpen, setLogsOpen] = useState(false)
  const service = store.getById(serviceId)

  if (!service) {
    return (
      <div className="mx-auto max-w-lg py-16">
        <AlertTriangle size={24} style={{ color: 'var(--cp-danger)' }} />
        <h1 className="mt-4 font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.detail.notFound', 'Application not found')}
        </h1>
        <button
          type="button"
          onClick={() => onNavigate({ page: 'home' })}
          className="mt-5 inline-flex min-h-11 items-center gap-2 rounded-xl px-4 text-sm font-semibold"
          style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}
        >
          <ArrowLeft size={15} aria-hidden="true" />
          {t('appService.detail.back', 'Back to applications')}
        </button>
      </div>
    )
  }

  const canStart = service.status === 'stopped' || service.status === 'activation_failed'
  const canStop = service.status === 'running' || service.status === 'starting'

  return (
    <div className="space-y-7">
      {!isMobile && (
        <button
          type="button"
          onClick={() => onNavigate({ page: 'home' })}
          className="inline-flex min-h-11 items-center gap-2 rounded-lg pr-3 text-sm font-semibold"
          style={{ color: 'var(--cp-muted)' }}
        >
          <ArrowLeft size={16} aria-hidden="true" />
          {t('appService.detail.back', 'Back to applications')}
        </button>
      )}

      <header
        className="grid gap-5 rounded-[22px] p-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-center"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-panel-shadow)' }}
      >
        <div className="flex min-w-0 items-start gap-4">
          <div
            className="flex size-14 shrink-0 items-center justify-center rounded-[16px]"
            style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
          >
            <AppIcon iconKey={service.iconKey} className="!size-7" />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <h1 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{service.name}</h1>
              <StateValue state={service.status} />
            </div>
            <p className="mt-1 max-w-2xl text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{service.description}</p>
            <div className="mt-2 text-[11px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>
              v{service.version} · {service.serviceInfo.node ?? t('appService.home.localNode', 'Local node')}
            </div>
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => setLogsOpen((open) => !open)}
            className="inline-flex min-h-11 items-center gap-2 rounded-xl px-3.5 text-xs font-semibold"
            style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          >
            <ScrollText size={14} aria-hidden="true" />
            {logsOpen ? t('appService.detail.hideLogs', 'Hide log') : t('appService.detail.openLogs', 'Open log')}
          </button>
          <button
            type="button"
            disabled={!canStart}
            onClick={() => store.startService(service.id)}
            className="inline-flex min-h-11 items-center gap-2 rounded-xl px-3.5 text-xs font-semibold disabled:opacity-40"
            style={{ background: canStart ? 'var(--cp-success)' : 'var(--cp-surface-2)', color: canStart ? 'var(--cp-bg-strong)' : 'var(--cp-muted)' }}
          >
            <Play size={14} aria-hidden="true" />
            {t('appService.action.start', 'Start')}
          </button>
          <button
            type="button"
            disabled={!canStop}
            onClick={() => store.stopService(service.id)}
            className="inline-flex min-h-11 items-center gap-2 rounded-xl px-3.5 text-xs font-semibold disabled:opacity-40"
            style={{ background: canStop ? 'var(--cp-danger)' : 'var(--cp-surface-2)', color: canStop ? 'var(--cp-surface)' : 'var(--cp-muted)' }}
          >
            <Square size={13} aria-hidden="true" />
            {t('appService.action.stop', 'Stop')}
          </button>
        </div>
      </header>

      {logsOpen && (
        <section>
          <SectionTitle icon={ScrollText}>{t('appService.detail.runtimeLog', 'Runtime log')}</SectionTitle>
          <div
            className="overflow-x-auto rounded-[18px] p-4 font-mono text-xs leading-6"
            data-testid="app-service-runtime-log"
            style={{ background: 'var(--cp-bg-strong)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          >
            {service.logs.length > 0
              ? service.logs.map((line) => <div key={line} className="whitespace-nowrap">{line}</div>)
              : <span style={{ color: 'var(--cp-muted)' }}>{t('appService.detail.noLogs', 'No runtime log entries are available.')}</span>}
          </div>
        </section>
      )}

      <div className="grid gap-6 lg:grid-cols-[minmax(0,1.15fr)_minmax(280px,0.85fr)]">
        <section>
          <SectionTitle icon={Server}>{t('appService.detail.statusOverview', 'Status overview')}</SectionTitle>
          <div className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <div className="flex min-h-10 items-center justify-between gap-4 border-b pb-3" style={{ borderColor: 'var(--cp-border)' }}>
              <span className="text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{t('appService.detail.appStatus', 'Application')}</span>
              <StateValue state={service.status} />
            </div>
            {service.docker ? (
              <div className="pt-1">
                <DependencyRow icon={Server} label={t('appService.detail.dockerEngine', 'Docker Engine')} state={service.docker.engine} />
                <DependencyRow icon={HardDrive} label={t('appService.detail.image', 'Image')} state={service.docker.image} />
                <DependencyRow icon={Container} label={t('appService.detail.container', 'Container')} state={service.docker.container} last />
              </div>
            ) : (
              <p className="pt-3 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
                {t('appService.detail.nativeRuntime', 'This service runs directly on the node without a Docker dependency chain.')}
              </p>
            )}
          </div>
        </section>

        <section>
          <SectionTitle icon={Box}>{t('appService.detail.runtimeInfo', 'Runtime info')}</SectionTitle>
          <div className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <DefinitionRows
              values={{
                version: service.version,
                ...(service.docker ? { image: service.docker.imageName } : {}),
                ...service.serviceInfo,
              }}
            />
          </div>
        </section>
      </div>

      <section>
        <SectionTitle icon={AlertTriangle}>{t('appService.detail.diagnostics', 'Diagnostics')}</SectionTitle>
        <div
          className="rounded-[18px] p-4"
          style={{
            background: service.diagnostics.length > 0
              ? 'color-mix(in srgb, var(--cp-danger) 6%, var(--cp-surface))'
              : 'var(--cp-surface)',
            border: `1px solid ${service.diagnostics.length > 0 ? 'color-mix(in srgb, var(--cp-danger) 24%, var(--cp-border))' : 'var(--cp-border)'}`,
          }}
        >
          {service.diagnostics.length > 0 ? (
            <div className="space-y-3">
              {service.diagnostics.map((message) => (
                <div key={message} className="flex items-start gap-2.5">
                  <AlertTriangle size={15} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-danger)' }} />
                  <span className="text-xs leading-5" style={{ color: 'var(--cp-text)' }}>{message}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="flex items-center gap-2.5">
              <CheckCircle2 size={15} aria-hidden="true" style={{ color: 'var(--cp-success)' }} />
              <span className="text-xs font-medium" style={{ color: 'var(--cp-success)' }}>
                {t('appService.detail.noIssues', 'No issues detected')}
              </span>
            </div>
          )}
        </div>
      </section>

      <div className="grid gap-6 lg:grid-cols-2">
        <section>
          <SectionTitle icon={FileCode2}>{t('appService.detail.spec', 'Installation spec')}</SectionTitle>
          <div className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <DefinitionRows values={service.spec} />
          </div>
        </section>
        <SettingsEditor service={service} />
      </div>
    </div>
  )
}
