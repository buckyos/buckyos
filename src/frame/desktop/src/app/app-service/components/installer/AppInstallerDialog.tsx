import { useState } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { useForm, useWatch } from 'react-hook-form'
import {
  AlertOctagon,
  AlertTriangle,
  ArrowLeft,
  Check,
  CheckCircle2,
  CircleDashed,
  Clipboard,
  CloudDownload,
  Container,
  Database,
  FileArchive,
  FolderOpen,
  HardDriveDownload,
  KeyRound,
  Loader2,
  Minus,
  Network,
  PackageCheck,
  Play,
  Server,
  ShieldAlert,
  ShieldCheck,
  X,
} from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { AppIcon } from '../../../../components/DesktopVisuals'
import { useAppServiceStore } from '../../hooks/use-app-service-store'
import {
  installerApprovalSchema,
  type InstallerApprovalInput,
} from '../../schemas'
import type {
  InstallAppInfo,
  InstallOptions,
  InstallPermission,
  InstallTask,
  InstallTaskStage,
  TrustCheck,
} from '../../mock/types'

function sourceKindLabel(kind: InstallAppInfo['source']['kind'], t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.source.kind.${kind}`, kind)
}

function stageLabel(stage: InstallTaskStage, t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.install.stage.${stage}`, stage)
}

function formatBytes(bytes: number) {
  if (bytes === 0) return '0 MB'
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`
  return `${Math.ceil(bytes / 1_048_576)} MB`
}

function trustColor(status: TrustCheck['status']) {
  switch (status) {
    case 'verified':
      return 'var(--cp-success)'
    case 'warning':
    case 'pending':
    case 'unknown':
      return 'var(--cp-warning)'
    case 'failed':
      return 'var(--cp-danger)'
  }
}

function TrustStateIcon({ status }: { status: TrustCheck['status'] }) {
  const color = trustColor(status)
  if (status === 'verified') return <CheckCircle2 size={15} style={{ color }} aria-hidden="true" />
  if (status === 'pending') return <Loader2 size={15} className="animate-spin" style={{ color }} aria-hidden="true" />
  if (status === 'failed') return <AlertOctagon size={15} style={{ color }} aria-hidden="true" />
  return <AlertTriangle size={15} style={{ color }} aria-hidden="true" />
}

function InstallerFrame({
  task,
  step,
  onClose,
  children,
}: {
  task: InstallTask
  step: 'verify' | 'approval' | 'install' | 'result'
  onClose: () => void
  children: React.ReactNode
}) {
  const { t } = useI18n()
  const steps = [
    { key: 'verify', label: t('appService.install.step.verify', 'Verify') },
    { key: 'approval', label: t('appService.install.step.plan', 'Plan') },
    { key: 'install', label: t('appService.install.step.install', 'Install') },
    { key: 'result', label: t('appService.install.step.result', 'Result') },
  ] as const
  const activeIndex = steps.findIndex((item) => item.key === step)

  return (
    <div
      className="mx-auto overflow-hidden rounded-[24px]"
      data-testid="app-installer-dialog"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-window-shadow)' }}
    >
      <header className="border-b px-5 py-4 sm:px-6" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[10px] font-semibold uppercase tracking-[0.2em]" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.systemInstaller', 'System App Installer')}
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1">
              <h1 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('appService.install.installApp', 'Install application')}
              </h1>
              <code className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>{task.taskId}</code>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="flex size-11 shrink-0 items-center justify-center rounded-xl"
            aria-label={t('appService.install.runInBackground', 'Run in background')}
            style={{ color: 'var(--cp-muted)' }}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>

        <ol className="mt-5 grid grid-cols-4 gap-1" aria-label={t('appService.install.progressSteps', 'Installation steps')}>
          {steps.map((item, index) => {
            const completed = index < activeIndex
            const active = index === activeIndex
            return (
              <li key={item.key} className="min-w-0">
                <div
                  className="h-1 rounded-full"
                  style={{ background: completed || active ? 'var(--cp-accent)' : 'var(--cp-surface-2)' }}
                />
                <div className="mt-1.5 truncate text-[10px] font-semibold" style={{ color: active ? 'var(--cp-text)' : 'var(--cp-muted)' }}>
                  {item.label}
                </div>
              </li>
            )
          })}
        </ol>
      </header>
      {children}
    </div>
  )
}

function AppIdentity({ app }: { app: InstallAppInfo }) {
  const { t } = useI18n()
  return (
    <div className="flex items-start gap-4">
      <div
        className="flex size-14 shrink-0 items-center justify-center rounded-[16px]"
        style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
      >
        <AppIcon iconKey={app.iconKey} className="!size-7" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{app.name}</h2>
          <span className="text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>v{app.version}</span>
        </div>
        <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{app.description}</p>
        <div className="mt-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.release', 'Release')} {app.releaseVersion}
        </div>
      </div>
    </div>
  )
}

function InfoRow({ label, value, code }: { label: string; value: string; code?: boolean }) {
  return (
    <div className="grid gap-1 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[150px_minmax(0,1fr)] sm:gap-4">
      <dt className="text-xs" style={{ color: 'var(--cp-muted)' }}>{label}</dt>
      <dd className={`min-w-0 break-words text-xs font-medium sm:text-right ${code ? 'font-mono' : ''}`} style={{ color: 'var(--cp-text)' }}>{value}</dd>
    </div>
  )
}

function BlockingCallout({ reason }: { reason: NonNullable<InstallAppInfo['blockingReason']> }) {
  const { t } = useI18n()
  const label = t(`appService.install.block.${reason}.title`, reason)
  const body = t(`appService.install.block.${reason}.body`, 'Resolve this issue before continuing installation.')
  return (
    <div
      className="flex items-start gap-3 rounded-[16px] p-4"
      data-testid="app-installer-blocking-reason"
      style={{ background: 'color-mix(in srgb, var(--cp-danger) 7%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-danger) 24%, var(--cp-border))' }}
    >
      <ShieldAlert size={18} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-danger)' }} />
      <div>
        <div className="text-xs font-semibold" style={{ color: 'var(--cp-danger)' }}>{label}</div>
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>{body}</p>
      </div>
    </div>
  )
}

function VerifyStep({
  task,
  onBack,
  onContinue,
}: {
  task: InstallTask
  onBack: () => void
  onContinue: () => void
}) {
  const { t } = useI18n()
  const { app } = task

  return (
    <div className="space-y-6 p-5 sm:p-6">
      <AppIdentity app={app} />

      <div className="grid gap-5 lg:grid-cols-2">
        <section>
          <div className="mb-2 flex items-center gap-2">
            <FileArchive size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
            <h3 className="text-xs font-semibold uppercase tracking-[0.12em]" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.sourceAndIdentity', 'Source and identity')}
            </h3>
          </div>
          <dl className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
            <InfoRow label={t('appService.install.inputType', 'Input type')} value={sourceKindLabel(app.source.kind, t)} />
            <InfoRow label={t('appService.install.source', 'Source')} value={app.source.displaySource} />
            <InfoRow label={t('appService.install.appDid', 'App DID')} value={app.appDid} code />
            <InfoRow label={t('appService.install.objectId', 'Document Object ID')} value={app.documentObjectId} code />
            <InfoRow label={t('appService.install.publisher', 'Publisher')} value={app.publisher} />
            <InfoRow label={t('appService.install.referrer', 'Referrer')} value={app.referrer} />
          </dl>
        </section>

        <section>
          <div className="mb-2 flex items-center gap-2">
            <ShieldCheck size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
            <h3 className="text-xs font-semibold uppercase tracking-[0.12em]" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.trustEvidence', 'Trust evidence')}
            </h3>
          </div>
          <div className="overflow-hidden rounded-[16px]" style={{ border: '1px solid var(--cp-border)' }}>
            {app.trustChecks.map((check, index) => (
              <div
                key={check.code}
                className="flex items-start gap-3 px-4 py-3"
                style={{ borderTop: index === 0 ? undefined : '1px solid var(--cp-border)', background: 'var(--cp-surface-2)' }}
              >
                <TrustStateIcon status={check.status} />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                      {t(`appService.install.trust.${check.code}`, check.code)}
                    </span>
                    <span className="text-[10px] font-semibold uppercase tracking-wide" style={{ color: trustColor(check.status) }}>
                      {t(`appService.install.trustStatus.${check.status}`, check.status)}
                    </span>
                  </div>
                  <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>{check.detail}</p>
                </div>
              </div>
            ))}
          </div>
        </section>
      </div>

      <section className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
          <div className="flex items-center justify-between gap-3">
            <span className="inline-flex items-center gap-2 text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              <Server size={15} aria-hidden="true" />
              {t('appService.install.platform', 'Target platform')}
            </span>
            <span className="text-xs font-semibold" style={{ color: app.platformSupported ? 'var(--cp-success)' : 'var(--cp-danger)' }}>
              {app.platformSupported ? t('appService.install.supported', 'Supported') : t('appService.install.unsupported', 'Unsupported')}
            </span>
          </div>
          <p className="mt-2 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.install.platformDetail', 'Linux · aarch64 · Docker 26+')}
          </p>
        </div>
        <div className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
          <div className="flex items-center justify-between gap-3">
            <span className="inline-flex items-center gap-2 text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              <CloudDownload size={15} aria-hidden="true" />
              {t('appService.install.contentReadiness', 'Content readiness')}
            </span>
            <span className="text-xs font-semibold" style={{ color: app.content.offlineReady ? 'var(--cp-success)' : 'var(--cp-warning)' }}>
              {app.content.offlineReady ? t('appService.install.offlineReady', 'Offline ready') : t('appService.install.downloadRequired', 'Download required')}
            </span>
          </div>
          <p className="mt-2 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
            {app.content.offlineReady
              ? t('appService.install.noMissingContent', 'All required content is available in controlled staging.')
              : t('appService.install.missingContent', '{{size}} missing · {{source}}', { size: formatBytes(app.content.missingBytes), source: app.content.availableSource })}
          </p>
        </div>
      </section>

      {app.blockingReason && <BlockingCallout reason={app.blockingReason} />}
      {app.source.warningCode === 'UNSIGNED_CANDIDATE' && !app.blockingReason && (
        <div className="flex items-start gap-3 rounded-[16px] p-4" style={{ background: 'color-mix(in srgb, var(--cp-warning) 8%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-warning) 25%, var(--cp-border))' }}>
          <AlertTriangle size={17} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
          <p className="text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.unsignedWarning', 'This App Meta JSON is unsigned. Installation is allowed only because its Object ID matches the document currently published by the App DID.')}
          </p>
        </div>
      )}

      <footer className="flex flex-col-reverse gap-2 border-t pt-5 sm:flex-row sm:justify-between" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          onClick={onBack}
          className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          <ArrowLeft size={15} aria-hidden="true" />
          {t('appService.install.changeSource', 'Change source')}
        </button>
        <button
          type="button"
          onClick={onContinue}
          disabled={!app.installReady}
          className="min-h-11 rounded-xl px-5 text-sm font-semibold disabled:opacity-40"
          style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
        >
          {t('appService.install.reviewPlan', 'Review installation plan')}
        </button>
      </footer>
    </div>
  )
}

function PermissionIcon({ kind }: { kind: InstallPermission['kind'] }) {
  switch (kind) {
    case 'files': return <FolderOpen size={15} aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
    case 'network': return <Network size={15} aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
    case 'database': return <Database size={15} aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
    case 'system': return <Container size={15} aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
  }
}

function ApprovalStep({ task, onBack }: { task: InstallTask; onBack: () => void }) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const form = useForm<InstallerApprovalInput>({
    resolver: zodResolver(installerApprovalSchema),
    mode: 'onBlur',
    defaultValues: {
      ...task.plan.options,
      password: '',
    },
  })
  const watched = useWatch({ control: form.control })
  const options: InstallOptions = {
    targetNode: watched.targetNode ?? task.plan.options.targetNode,
    components: watched.components ?? task.plan.options.components,
    dataDir: watched.dataDir ?? task.plan.options.dataDir,
    networkMode: watched.networkMode ?? task.plan.options.networkMode,
    autoStart: watched.autoStart ?? task.plan.options.autoStart,
  }
  const preview = store.previewInstallPlan(task.app, options)

  return (
    <form className="space-y-6 p-5 sm:p-6" onSubmit={form.handleSubmit((values) => store.approveTask(task.taskId, values))} noValidate>
      <div>
        <h2 className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.install.optionsTitle', 'Installation plan')}
        </h2>
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.optionsBody', 'Changing the target or settings recalculates content readiness and technical impact before approval.')}
        </p>
      </div>

      <div className="grid gap-5 lg:grid-cols-2">
        <section className="space-y-4 rounded-[18px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.targetNode', 'Target node')}
            </span>
            <select
              {...form.register('targetNode')}
              className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
              style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            >
              <option value="ood-primary">{t('appService.install.target.oodPrimary', 'OOD Primary · aarch64')}</option>
              <option value="ood-backup">{t('appService.install.target.oodBackup', 'OOD Backup · aarch64')}</option>
            </select>
          </label>

          <fieldset>
            <legend className="mb-2 text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.components', 'Components')}
            </legend>
            <div className="space-y-2">
              {task.app.availableComponents.map((component) => (
                <label key={component} className="flex min-h-11 items-center gap-3 rounded-xl px-3" style={{ background: 'var(--cp-surface)' }}>
                  <input type="checkbox" value={component} {...form.register('components')} className="size-4 accent-[var(--cp-accent)]" />
                  <span className="text-sm font-medium capitalize" style={{ color: 'var(--cp-text)' }}>{component}</span>
                </label>
              ))}
            </div>
            {form.formState.errors.components && (
              <p className="mt-1.5 text-xs" style={{ color: 'var(--cp-danger)' }}>
                {t('appService.install.componentsRequired', 'Choose at least one component.')}
              </p>
            )}
          </fieldset>

          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.dataDirectory', 'Data directory')}
            </span>
            <input
              {...form.register('dataDir')}
              className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
              aria-invalid={Boolean(form.formState.errors.dataDir)}
              style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: `1px solid ${form.formState.errors.dataDir ? 'var(--cp-danger)' : 'var(--cp-border)'}` }}
            />
            {form.formState.errors.dataDir && (
              <p className="mt-1.5 text-xs" style={{ color: 'var(--cp-danger)' }}>
                {t('appService.install.dataDirectoryError', 'Enter an absolute data directory beginning with /.')}
              </p>
            )}
          </label>

          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.networkMode', 'Network access')}
            </span>
            <select
              {...form.register('networkMode')}
              className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
              style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            >
              <option value="zone">{t('appService.install.networkZone', 'Zone HTTPS route')}</option>
              <option value="private">{t('appService.install.networkPrivate', 'Private node access only')}</option>
            </select>
          </label>

          <label className="flex min-h-11 items-center justify-between gap-4 rounded-xl px-3" style={{ background: 'var(--cp-surface)' }}>
            <span>
              <span className="block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.autoStart', 'Start after installation')}</span>
              <span className="mt-0.5 block text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('appService.install.autoStartHint', 'Installation success remains separate from startup success.')}</span>
            </span>
            <input type="checkbox" {...form.register('autoStart')} className="size-4 shrink-0 accent-[var(--cp-accent)]" />
          </label>
        </section>

        <div className="space-y-5">
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-[0.12em]" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.requiredPermissions', 'Required permissions')}
            </h3>
            <div className="overflow-hidden rounded-[16px]" style={{ border: '1px solid var(--cp-border)' }}>
              {preview.permissions.map((permission, index) => {
                return (
                  <div key={`${permission.kind}-${permission.scope}`} className="flex items-center gap-3 px-4 py-3" style={{ borderTop: index === 0 ? undefined : '1px solid var(--cp-border)' }}>
                    <PermissionIcon kind={permission.kind} />
                    <div className="min-w-0">
                      <div className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                        {t(`appService.install.permission.${permission.kind}`, permission.kind)}
                      </div>
                      <div className="mt-0.5 truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{permission.scope}</div>
                    </div>
                  </div>
                )
              })}
            </div>
          </section>

          <section className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
            <h3 className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.technicalImpact', 'Technical impact')}</h3>
            <ul className="mt-3 space-y-2">
              {preview.impacts.map((impact) => (
                <li key={impact} className="flex items-start gap-2 text-[11px] leading-5" style={{ color: 'var(--cp-muted)' }}>
                  <Check size={13} className="mt-1 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />
                  {t(`appService.install.impact.${impact}`, impact)}
                </li>
              ))}
            </ul>
            <div className="mt-4 flex items-center justify-between gap-3 border-t pt-3" style={{ borderColor: 'var(--cp-border)' }}>
              <span className="inline-flex items-center gap-2 text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                <HardDriveDownload size={15} aria-hidden="true" />
                {t('appService.install.downloadSize', 'Download')}
              </span>
              <span className="text-xs font-semibold tabular-nums" style={{ color: preview.content.missingBytes > 0 ? 'var(--cp-warning)' : 'var(--cp-success)' }}>
                {preview.content.missingBytes > 0 ? formatBytes(preview.content.missingBytes) : t('appService.install.notRequired', 'Not required')}
              </span>
            </div>
          </section>

          <section className="rounded-[16px] p-4" style={{ background: 'color-mix(in srgb, var(--cp-accent) 6%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))' }}>
            <div className="flex items-start gap-3">
              <KeyRound size={17} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />
              <div className="min-w-0 flex-1">
                <label htmlFor="app-installer-password" className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                  {t('appService.install.adminPassword', 'Administrator password')}
                </label>
                <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
                  {t('appService.install.adminPasswordHint', 'Required for this system-level installation. The password is never stored in task history.')}
                </p>
                <input
                  id="app-installer-password"
                  type="password"
                  autoComplete="current-password"
                  {...form.register('password')}
                  className="aicc-password-input mt-3 min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                  aria-invalid={Boolean(form.formState.errors.password)}
                  style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: `1px solid ${form.formState.errors.password ? 'var(--cp-danger)' : 'var(--cp-border)'}` }}
                />
                {form.formState.errors.password && (
                  <p className="mt-1.5 text-xs" style={{ color: 'var(--cp-danger)' }}>
                    {t('appService.install.passwordRequired', 'Enter the administrator password to continue.')}
                  </p>
                )}
              </div>
            </div>
          </section>
        </div>
      </div>

      <footer className="flex flex-col-reverse gap-2 border-t pt-5 sm:flex-row sm:justify-between" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          onClick={onBack}
          className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          <ArrowLeft size={15} aria-hidden="true" />
          {t('appService.install.backToVerify', 'Back to verification')}
        </button>
        <button
          type="submit"
          className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-5 text-sm font-semibold"
          style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
        >
          <ShieldCheck size={15} aria-hidden="true" />
          {t('appService.install.confirmInstall', 'Authorize and install')}
        </button>
      </footer>
    </form>
  )
}

const taskStages: InstallTaskStage[] = ['resolve', 'inspect', 'acquire', 'verify', 'prepare', 'deploy', 'activate']

function ProgressStep({ task, onBackground }: { task: InstallTask; onBackground: () => void }) {
  const { t } = useI18n()
  return (
    <div className="space-y-6 p-5 sm:p-6">
      <div className="flex items-start gap-4">
        <span className="flex size-12 shrink-0 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))', color: 'var(--cp-accent)' }}>
          <Loader2 size={22} className="animate-spin" aria-hidden="true" />
        </span>
        <div>
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.installingName', 'Installing {{name}}', { name: task.app.name })}
          </h2>
          <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{task.summary}</p>
        </div>
      </div>

      <section className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
        <div className="flex items-center justify-between gap-4 text-xs">
          <span style={{ color: 'var(--cp-muted)' }}>{t('appService.install.currentStage', 'Current stage')}</span>
          <span className="font-semibold" style={{ color: 'var(--cp-text)' }}>{stageLabel(task.stage, t)}</span>
        </div>
        {task.progress !== null && (
          <>
            <div className="mt-3 h-2 overflow-hidden rounded-full" style={{ background: 'var(--cp-bg-strong)' }}>
              <div className="h-full rounded-full transition-[width] duration-300" style={{ width: `${task.progress}%`, background: 'var(--cp-accent)' }} />
            </div>
            <div className="mt-1.5 text-right text-[11px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>{task.progress}%</div>
          </>
        )}
        {task.currentResource && (
          <div className="mt-3 flex items-center gap-2 border-t pt-3 text-[11px]" style={{ borderColor: 'var(--cp-border)', color: 'var(--cp-muted)' }}>
            <CloudDownload size={14} aria-hidden="true" />
            <span className="truncate">{task.currentResource}</span>
          </div>
        )}
      </section>

      <ol className="grid gap-2 sm:grid-cols-2">
        {taskStages.map((stage) => {
          const history = task.history.find((item) => item.stage === stage)
          const status = history?.status ?? 'pending'
          return (
            <li key={stage} className="flex min-h-11 items-center gap-3 rounded-xl px-3" style={{ background: 'var(--cp-surface-2)' }}>
              {status === 'completed' && <CheckCircle2 size={15} aria-hidden="true" style={{ color: 'var(--cp-success)' }} />}
              {status === 'current' && <Loader2 size={15} className="animate-spin" aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />}
              {status === 'skipped' && <Minus size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />}
              {status === 'pending' && <CircleDashed size={15} aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />}
              <span className="text-xs font-medium" style={{ color: status === 'pending' ? 'var(--cp-muted)' : 'var(--cp-text)' }}>{stageLabel(stage, t)}</span>
              {status === 'skipped' && <span className="ml-auto text-[10px]" style={{ color: 'var(--cp-muted)' }}>{t('appService.install.skipped', 'Skipped')}</span>}
            </li>
          )
        })}
      </ol>

      <div className="rounded-[16px] p-4 text-xs leading-5" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}>
        {t('appService.install.taskManagerHint', 'This installation continues as a system task. Detailed activity remains available in Task Center under the same task ID.')}
      </div>

      <footer className="flex justify-end border-t pt-5" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          onClick={onBackground}
          className="min-h-11 rounded-xl px-5 text-sm font-semibold"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          {t('appService.install.runInBackground', 'Run in background')}
        </button>
      </footer>
    </div>
  )
}

function FailureStep({ task, onChangeSource }: { task: InstallTask; onChangeSource: () => void }) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const failure = task.failure

  const copyDetails = async () => {
    if (!failure) return
    try {
      await navigator.clipboard.writeText(failure.technicalDetail)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div className="space-y-6 p-5 sm:p-6">
      <div className="flex items-start gap-4">
        <span className="flex size-12 shrink-0 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-danger) 11%, var(--cp-surface))', color: 'var(--cp-danger)' }}>
          <AlertOctagon size={22} aria-hidden="true" />
        </span>
        <div>
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.failedTitle', 'Installation stopped')}
          </h2>
          <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
            {failure?.message ?? t('appService.install.failedUnknown', 'The installation task could not continue.')}
          </p>
        </div>
      </div>

      <dl className="rounded-[18px] p-4" style={{ background: 'color-mix(in srgb, var(--cp-danger) 6%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-danger) 24%, var(--cp-border))' }}>
        <InfoRow label={t('appService.install.failedStage', 'Failed stage')} value={stageLabel(failure?.stage ?? task.stage, t)} />
        <InfoRow label={t('appService.install.errorCategory', 'Error category')} value={failure?.code ?? 'UNKNOWN'} code />
        <InfoRow label={t('appService.install.taskId', 'Task ID')} value={task.taskId} code />
      </dl>

      <div className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
        <h3 className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.nextActions', 'What you can do')}</h3>
        <p className="mt-2 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.failureNextAction', 'Retry the same task, change the target or settings, or return to the source if the package location is no longer valid.')}
        </p>
      </div>

      <footer className="flex flex-col gap-2 border-t pt-5 sm:flex-row sm:flex-wrap sm:justify-between" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="flex flex-col gap-2 sm:flex-row">
          <button
            type="button"
            onClick={copyDetails}
            className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          >
            <Clipboard size={15} aria-hidden="true" />
            {copied ? t('common.copied', 'Copied') : t('appService.install.copyDetails', 'Copy safe details')}
          </button>
          <button
            type="button"
            onClick={onChangeSource}
            className="min-h-11 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          >
            {t('appService.install.changeSource', 'Change source')}
          </button>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <button
            type="button"
            onClick={() => store.returnTaskToApproval(task.taskId)}
            className="min-h-11 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)' }}
          >
            {t('appService.install.modifyOptions', 'Modify options')}
          </button>
          <button
            type="button"
            onClick={() => store.retryTask(task.taskId)}
            className="min-h-11 rounded-xl px-5 text-sm font-semibold"
            style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
          >
            {t('common.retry', 'Retry')}
          </button>
        </div>
      </footer>
    </div>
  )
}

function ResultStep({ task, onClose, onViewApp }: { task: InstallTask; onClose: () => void; onViewApp: () => void }) {
  const { t } = useI18n()
  const activationFailed = task.result?.autoStart === 'failed'
  return (
    <div className="space-y-6 p-5 sm:p-6">
      <div className="flex items-start gap-4">
        <span className="flex size-12 shrink-0 items-center justify-center rounded-full" style={{ background: `color-mix(in srgb, ${activationFailed ? 'var(--cp-warning)' : 'var(--cp-success)'} 12%, var(--cp-surface))`, color: activationFailed ? 'var(--cp-warning)' : 'var(--cp-success)' }}>
          {activationFailed ? <AlertTriangle size={22} aria-hidden="true" /> : <PackageCheck size={22} aria-hidden="true" />}
        </span>
        <div>
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
            {activationFailed ? t('appService.install.installedStartFailed', 'Installed, but startup failed') : t('appService.install.successTitle', 'Installation complete')}
          </h2>
          <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
            {activationFailed
              ? t('appService.install.installedStartFailedBody', 'The installation result is preserved. Open the application to review its runtime diagnosis and try Start again.')
              : t('appService.install.successBody', '{{name}} is installed and running.', { name: task.app.name })}
          </p>
        </div>
      </div>

      <dl className="rounded-[18px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
        <InfoRow label={t('appService.install.installedVersion', 'Installed version')} value={task.result?.installedVersion ?? task.app.version} />
        <InfoRow label={t('appService.install.targetNode', 'Target node')} value={task.result?.targetNode ?? task.plan.options.targetNode} />
        <InfoRow
          label={t('appService.install.autoStartResult', 'Automatic startup')}
          value={task.result?.autoStart === 'running'
            ? t('appService.stateLabel.running', 'Running')
            : task.result?.autoStart === 'failed'
              ? t('appService.stateLabel.error', 'Failed')
              : t('appService.install.skipped', 'Skipped')}
        />
        <InfoRow label={t('appService.install.taskId', 'Task ID')} value={task.taskId} code />
      </dl>

      <footer className="flex flex-col-reverse gap-2 border-t pt-5 sm:flex-row sm:justify-end" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          onClick={onClose}
          className="min-h-11 rounded-xl px-4 text-sm font-semibold"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          {t('common.close', 'Close')}
        </button>
        <button
          type="button"
          onClick={onViewApp}
          className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-5 text-sm font-semibold"
          style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
        >
          <Play size={15} aria-hidden="true" />
          {t('appService.install.viewApplication', 'View application')}
        </button>
      </footer>
    </div>
  )
}

interface AppInstallerDialogProps {
  taskId: string
  onBackground: () => void
  onChangeSource: () => void
  onClose: () => void
  onViewApp: (serviceId: string) => void
}

export function AppInstallerDialog({ taskId, onBackground, onChangeSource, onClose, onViewApp }: AppInstallerDialogProps) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const [approvalOpen, setApprovalOpen] = useState(false)
  const task = store.getTask(taskId)

  if (!task) {
    return (
      <div className="mx-auto max-w-xl rounded-[22px] p-6" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
        <AlertTriangle size={24} style={{ color: 'var(--cp-danger)' }} />
        <h1 className="mt-4 font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.install.taskNotFound', 'Installation task not found')}
        </h1>
        <p className="mt-2 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.taskNotFoundBody', 'The task ID is no longer available in this prototype session.')}
        </p>
        <button type="button" onClick={onClose} className="mt-5 min-h-11 rounded-xl px-4 text-sm font-semibold" style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}>
          {t('appService.detail.back', 'Back to applications')}
        </button>
      </div>
    )
  }

  if (task.status === 'completed') {
    return (
      <InstallerFrame task={task} step="result" onClose={onBackground}>
        <ResultStep task={task} onClose={onClose} onViewApp={() => onViewApp(task.app.id)} />
      </InstallerFrame>
    )
  }

  if (task.status === 'failed') {
    return (
      <InstallerFrame task={task} step="install" onClose={onBackground}>
        <FailureStep task={task} onChangeSource={onChangeSource} />
      </InstallerFrame>
    )
  }

  if (task.status === 'running') {
    return (
      <InstallerFrame task={task} step="install" onClose={onBackground}>
        <ProgressStep task={task} onBackground={onBackground} />
      </InstallerFrame>
    )
  }

  if (approvalOpen) {
    return (
      <InstallerFrame task={task} step="approval" onClose={onBackground}>
        <ApprovalStep task={task} onBack={() => setApprovalOpen(false)} />
      </InstallerFrame>
    )
  }

  return (
    <InstallerFrame task={task} step="verify" onClose={onBackground}>
      <VerifyStep task={task} onBack={onChangeSource} onContinue={() => setApprovalOpen(true)} />
    </InstallerFrame>
  )
}
