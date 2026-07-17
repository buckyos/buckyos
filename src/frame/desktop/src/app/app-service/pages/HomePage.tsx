import {
  AlertTriangle,
  Boxes,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Download,
  Loader2,
  Plus,
  RotateCcw,
  ServerCog,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { AppIcon } from '../../../components/DesktopVisuals'
import type { AppServiceNav } from '../components/layout/navigation'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import type { AppServiceItem, InstallTask } from '../mock/types'

function statusColor(status: AppServiceItem['status']) {
  switch (status) {
    case 'running':
      return 'var(--cp-success)'
    case 'starting':
      return 'var(--cp-accent)'
    case 'installing':
      return 'var(--cp-warning)'
    case 'error':
    case 'activation_failed':
      return 'var(--cp-danger)'
    case 'stopped':
    default:
      return 'var(--cp-muted)'
  }
}

function StatusIcon({ status }: { status: AppServiceItem['status'] }) {
  const props = { size: 13, 'aria-hidden': true }
  switch (status) {
    case 'running':
      return <CheckCircle2 {...props} />
    case 'starting':
      return <Loader2 {...props} className="animate-spin" />
    case 'installing':
      return <Download {...props} />
    case 'error':
    case 'activation_failed':
      return <AlertTriangle {...props} />
    case 'stopped':
    default:
      return <CircleStop {...props} />
  }
}

function statusLabel(status: AppServiceItem['status'], t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.status.${status}`, status)
}

function StatusBadge({ service }: { service: AppServiceItem }) {
  const { t } = useI18n()
  const color = statusColor(service.status)
  return (
    <span
      className="inline-flex min-h-7 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs font-semibold"
      style={{
        color,
        border: `1px solid color-mix(in srgb, ${color} 24%, var(--cp-border))`,
        background: `color-mix(in srgb, ${color} 9%, var(--cp-surface))`,
      }}
    >
      <StatusIcon status={service.status} />
      {statusLabel(service.status, t)}
    </span>
  )
}

function SectionHeading({
  title,
  count,
  description,
}: {
  title: string
  count: number
  description?: string
}) {
  return (
    <div className="mb-3 flex items-end justify-between gap-4">
      <div>
        <div className="flex items-baseline gap-2">
          <h2 className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
            {title}
          </h2>
          <span className="text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>
            {count}
          </span>
        </div>
        {description && (
          <p className="mt-0.5 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {description}
          </p>
        )}
      </div>
    </div>
  )
}

function AppCard({ service, onOpen }: { service: AppServiceItem; onOpen: () => void }) {
  const { t } = useI18n()
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex min-h-40 w-full flex-col rounded-[20px] p-4 text-left transition-[transform,background-color,border-color] duration-200 hover:-translate-y-0.5"
      data-testid={`app-service-card-${service.id}`}
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
        boxShadow: '0 10px 28px color-mix(in srgb, var(--cp-shadow) 6%, transparent)',
      }}
    >
      <div className="flex w-full items-start gap-3.5">
        <div
          className="flex size-12 shrink-0 items-center justify-center rounded-[14px]"
          style={{
            color: 'var(--cp-text)',
            background: 'color-mix(in srgb, var(--cp-accent-soft) 13%, var(--cp-surface-2))',
            border: '1px solid var(--cp-border)',
          }}
        >
          <AppIcon iconKey={service.iconKey} className="!size-6" />
        </div>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {service.name}
          </h3>
          <p className="mt-1 line-clamp-2 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {service.description}
          </p>
        </div>
        <StatusBadge service={service} />
      </div>

      {service.status === 'installing' && service.installProgress !== undefined && (
        <div className="mt-4 w-full" aria-label={t('appService.home.installProgress', 'Installation progress')}>
          <div className="mb-1.5 flex justify-between text-[11px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>
            <span>{t('appService.status.installing', 'Installing')}</span>
            <span>{service.installProgress}%</span>
          </div>
          <div className="h-1.5 overflow-hidden rounded-full" style={{ background: 'var(--cp-surface-2)' }}>
            <div
              className="h-full rounded-full transition-transform duration-300"
              style={{
                width: `${service.installProgress}%`,
                background: 'var(--cp-warning)',
              }}
            />
          </div>
        </div>
      )}

      <div
        className="mt-auto flex w-full items-center justify-between gap-3 border-t pt-3 text-[11px]"
        style={{ borderColor: 'var(--cp-border)', color: 'var(--cp-muted)' }}
      >
        <span className="min-w-0 truncate">
          {service.serviceInfo.node ?? t('appService.home.localNode', 'Local node')} · v{service.version}
        </span>
        <span className="inline-flex shrink-0 items-center gap-1 font-semibold group-hover:text-[var(--cp-text)]">
          {t('appService.home.openDetails', 'Open details')}
          <ChevronRight size={13} aria-hidden="true" />
        </span>
      </div>
    </button>
  )
}

function ServiceLedger({ services }: { services: AppServiceItem[] }) {
  const { t } = useI18n()
  return (
    <div className="overflow-hidden rounded-[18px]" style={{ border: '1px solid var(--cp-border)' }}>
      {services.map((service, index) => (
        <div
          key={service.id}
          className="grid min-h-16 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-4 py-3 sm:grid-cols-[minmax(190px,0.8fr)_minmax(220px,1.25fr)_auto]"
          style={{
            background: index % 2 === 0 ? 'var(--cp-surface)' : 'color-mix(in srgb, var(--cp-surface-2) 56%, var(--cp-surface))',
            borderTop: index === 0 ? undefined : '1px solid var(--cp-border)',
          }}
        >
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {service.name}
            </div>
            <div className="mt-0.5 truncate text-[11px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>
              v{service.version} · {service.serviceInfo.node ?? t('appService.home.localNode', 'Local node')}
            </div>
          </div>
          <p className="col-span-2 row-start-2 truncate text-xs sm:col-span-1 sm:col-start-2 sm:row-start-1" style={{ color: 'var(--cp-muted)' }}>
            {service.description}
          </p>
          <div className="col-start-2 row-start-1 sm:col-start-3">
            <StatusBadge service={service} />
          </div>
        </div>
      ))}
    </div>
  )
}

function taskActionLabel(task: InstallTask, t: ReturnType<typeof useI18n>['t']) {
  if (task.status === 'completed') return t('appService.task.viewResult', 'View result')
  if (task.status === 'failed') return t('appService.task.reviewFailure', 'Review failure')
  if (task.status === 'waiting_for_approval') return t('appService.task.continueInstall', 'Continue installation')
  return t('appService.task.viewProgress', 'View progress')
}

function ActiveTaskBanner({ task, onOpen }: { task: InstallTask; onOpen: () => void }) {
  const { t } = useI18n()
  const tone = task.status === 'failed'
    ? 'var(--cp-danger)'
    : task.status === 'completed'
      ? task.result?.autoStart === 'failed' ? 'var(--cp-warning)' : 'var(--cp-success)'
      : 'var(--cp-accent)'
  return (
    <section
      className="grid gap-4 rounded-[20px] p-4 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center"
      data-testid="app-service-active-task"
      style={{
        background: `color-mix(in srgb, ${tone} 8%, var(--cp-surface))`,
        border: `1px solid color-mix(in srgb, ${tone} 24%, var(--cp-border))`,
      }}
    >
      <div
        className="flex size-10 items-center justify-center rounded-full"
        style={{ background: `color-mix(in srgb, ${tone} 14%, transparent)`, color: tone }}
      >
        {task.status === 'running' ? <Loader2 size={18} className="animate-spin" /> : task.status === 'failed' ? <AlertTriangle size={18} /> : <Boxes size={18} />}
      </div>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {task.app.name}
          </h2>
          <code className="truncate text-[10px]" style={{ color: 'var(--cp-muted)' }}>
            {task.taskId}
          </code>
        </div>
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {task.summary}
        </p>
      </div>
      <button
        type="button"
        onClick={onOpen}
        className="min-h-11 rounded-xl px-4 text-sm font-semibold"
        style={{ background: tone, color: 'var(--cp-surface)' }}
      >
        {taskActionLabel(task, t)}
      </button>
    </section>
  )
}

function LoadingState() {
  const { t } = useI18n()
  return (
    <div className="space-y-6" aria-label={t('appService.state.loading', 'Loading application services')}>
      <div className="h-24 animate-pulse rounded-[20px]" style={{ background: 'var(--cp-surface)' }} />
      <div className="grid gap-3 md:grid-cols-2">
        {[0, 1, 2, 3].map((item) => (
          <div key={item} className="h-40 animate-pulse rounded-[20px]" style={{ background: 'var(--cp-surface)' }} />
        ))}
      </div>
    </div>
  )
}

function ErrorState({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n()
  return (
    <div className="mx-auto flex max-w-xl flex-col items-start py-16">
      <AlertTriangle size={24} style={{ color: 'var(--cp-danger)' }} />
      <h2 className="mt-4 font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('appService.state.errorTitle', 'App Service data is unavailable')}
      </h2>
      <p className="mt-2 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
        {t('appService.state.errorBody', 'The runtime snapshot could not be loaded. Retry without changing any application state.')}
      </p>
      <button
        type="button"
        onClick={onRetry}
        className="mt-5 inline-flex min-h-11 items-center gap-2 rounded-xl px-4 text-sm font-semibold"
        style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}
      >
        <RotateCcw size={15} aria-hidden="true" />
        {t('common.retry', 'Retry')}
      </button>
    </div>
  )
}

function EmptyApps({ onAdd }: { onAdd: () => void }) {
  const { t } = useI18n()
  return (
    <div
      className="flex flex-col items-start rounded-[20px] px-5 py-8"
      style={{ background: 'var(--cp-surface)', border: '1px dashed var(--cp-border)' }}
    >
      <Boxes size={24} style={{ color: 'var(--cp-accent)' }} />
      <h3 className="mt-4 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('appService.state.emptyTitle', 'No applications are installed')}
      </h3>
      <p className="mt-1 max-w-lg text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
        {t('appService.state.emptyBody', 'Add an App DID, App Meta URL, or .pikg package to begin managing it here.')}
      </p>
      <button
        type="button"
        onClick={onAdd}
        className="mt-4 inline-flex min-h-11 items-center gap-2 rounded-xl px-4 text-sm font-semibold"
        style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}
      >
        <Plus size={16} aria-hidden="true" />
        {t('appService.home.addApp', 'Add app')}
      </button>
    </div>
  )
}

interface HomePageProps {
  onNavigate: (nav: AppServiceNav) => void
}

export function HomePage({ onNavigate }: HomePageProps) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const apps = store.getByLayer('app')
  const systemServices = store.getByLayer('system')
  const kernelServices = store.getByLayer('kernel')

  const openInstaller = () => onNavigate({ page: 'install' })

  return (
    <div className="space-y-8">
      <header className="flex items-start justify-between gap-5">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.18em]" style={{ color: 'var(--cp-muted)' }}>
            <ServerCog size={14} aria-hidden="true" />
            {t('appService.home.kicker', 'System control plane')}
          </div>
          <h1 className="mt-2 font-display text-2xl font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.title', 'App Service')}
          </h1>
          <p className="mt-1 max-w-xl text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.subtitle', 'Install applications, observe runtime state, and understand failures.')}
          </p>
        </div>
        <button
          type="button"
          onClick={openInstaller}
          className="inline-flex min-h-11 shrink-0 items-center gap-2 rounded-xl px-3.5 text-sm font-semibold sm:px-4"
          aria-label={t('appService.home.addApp', 'Add app')}
          style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}
        >
          <Plus size={18} aria-hidden="true" />
          <span className="hidden sm:inline">{t('appService.home.addApp', 'Add app')}</span>
        </button>
      </header>

      {store.viewStatus === 'loading' && <LoadingState />}
      {store.viewStatus === 'error' && <ErrorState onRetry={() => store.retryLoad()} />}

      {store.viewStatus === 'ready' && (
        <>
          {store.activeTask && (
            <ActiveTaskBanner
              task={store.activeTask}
              onOpen={() => onNavigate({ page: 'install', taskId: store.activeTask?.taskId })}
            />
          )}

          <section>
            <SectionHeading
              title={t('appService.layer.apps', 'Applications')}
              count={apps.length}
              description={t('appService.layer.appsDescription', 'Installed apps and their current runtime state.')}
            />
            {apps.length === 0 ? (
              <EmptyApps onAdd={openInstaller} />
            ) : (
              <div className="grid gap-3 md:grid-cols-2">
                {apps.map((service) => (
                  <AppCard
                    key={service.id}
                    service={service}
                    onOpen={() => onNavigate({ page: 'detail', serviceId: service.id })}
                  />
                ))}
              </div>
            )}
          </section>

          <section>
            <SectionHeading
              title={t('appService.layer.system', 'System Services')}
              count={systemServices.length}
              description={t('appService.layer.systemDescription', 'Zone services that support applications and system state.')}
            />
            <ServiceLedger services={systemServices} />
          </section>

          <section>
            <SectionHeading
              title={t('appService.layer.kernel', 'Kernel')}
              count={kernelServices.length}
              description={t('appService.layer.kernelDescription', 'Low-level node services are shown as read-only runtime information.')}
            />
            <ServiceLedger services={kernelServices} />
          </section>
        </>
      )}
    </div>
  )
}
