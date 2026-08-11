/* eslint-disable react-refresh/only-export-components */
import { useEffect, useMemo, useState } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { useFieldArray, useForm, useWatch } from 'react-hook-form'
import { useLocation, useNavigate } from 'react-router-dom'
import { z } from 'zod'
import {
  AlertOctagon,
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  ChevronDown,
  CircleDashed,
  Clipboard,
  CloudDownload,
  Container,
  Database,
  FileArchive,
  FolderOpen,
  KeyRound,
  Loader2,
  Minus,
  Network,
  PackageCheck,
  Play,
  Plus,
  Server,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { AppIcon } from '../components/DesktopVisuals'
import { useSharedAppServiceStore } from '../app/app-service/hooks/use-app-service-store'
import {
  installerApprovalSchema,
  installerSudoSchema,
  type InstallerApprovalInput,
  type InstallerSudoInput,
} from '../app/app-service/schemas'
import type {
  InstallAppInfo,
  AppPrice,
  InstallLaunchRequest,
  InstallPermission,
  InstallTargetNode,
  InstallTask,
  InstallTaskStage,
  TrustCheck,
} from '../app/app-service/mock/types'

export interface AppInstallerLaunchOptions {
  target?: {
    node_did?: string
    node_id?: string
  }
  install_params?: Record<string, unknown>
  offline?: boolean
}

export type AppInstallerLaunchParams =
  | { task_id: string }
  | {
      identifier: string
      ref?: string
      options?: AppInstallerLaunchOptions
    }

export type AppInstallerLaunchErrorCode =
  | 'duplicate_parameter'
  | 'unknown_parameter'
  | 'conflicting_parameters'
  | 'invalid_task_id'
  | 'invalid_identifier'
  | 'invalid_options'
  | 'invalid_target'
  | 'source_unrecognized'

export type AppInstallerLaunchQueryResult =
  | { ok: true; params: AppInstallerLaunchParams }
  | { ok: false; code: AppInstallerLaunchErrorCode }

const maxTaskId = BigInt('9223372036854775807')
const launchTargetSchema = z.object({
  node_did: z.string().trim().min(1).max(512).optional(),
  node_id: z.string().trim().min(1).max(256).optional(),
}).strict().refine((target) => Boolean(target.node_did || target.node_id))
const launchOptionsSchema = z.object({
  target: launchTargetSchema.optional(),
  install_params: z.record(z.string(), z.unknown()).optional(),
  offline: z.boolean().optional(),
}).strict()
const taskIdSchema = z.string().regex(/^[1-9]\d*$/).refine((value) => BigInt(value) <= maxTaskId)
const appInstallerLaunchParamsSchema = z.union([
  z.object({ task_id: taskIdSchema }).strict(),
  z.object({
    identifier: z.string().trim().min(1).max(32_768),
    ref: z.string().trim().min(1).max(2_048).optional(),
    options: launchOptionsSchema.optional(),
  }).strict(),
])

function validateTarget(target: AppInstallerLaunchOptions['target']):
  | { ok: true; targetNode?: InstallTargetNode }
  | { ok: false } {
  if (!target) return { ok: true }

  const nodeId = target.node_id as InstallTargetNode | undefined
  if (nodeId && nodeId !== 'ood-primary' && nodeId !== 'ood-backup') {
    return { ok: false }
  }

  let didNode: InstallTargetNode | undefined
  if (target.node_did) {
    if (target.node_did.endsWith('ood-primary')) didNode = 'ood-primary'
    if (target.node_did.endsWith('ood-backup')) didNode = 'ood-backup'
    if (!didNode) return { ok: false }
  }

  if (nodeId && didNode && nodeId !== didNode) return { ok: false }
  return { ok: true, targetNode: nodeId ?? didNode }
}

export function parseAppInstallerLaunchQuery(search: string): AppInstallerLaunchQueryResult {
  const searchParams = new URLSearchParams(search)
  const allowedKeys = new Set(['task_id', 'identifier', 'ref', 'options'])

  for (const key of searchParams.keys()) {
    if (!allowedKeys.has(key)) return { ok: false, code: 'unknown_parameter' }
    if (searchParams.getAll(key).length !== 1) return { ok: false, code: 'duplicate_parameter' }
  }

  const taskId = searchParams.get('task_id')
  const identifier = searchParams.get('identifier')
  const ref = searchParams.get('ref')
  const serializedOptions = searchParams.get('options')

  if (taskId !== null) {
    if (identifier !== null || ref !== null || serializedOptions !== null) {
      return { ok: false, code: 'conflicting_parameters' }
    }
    if (!taskIdSchema.safeParse(taskId).success) {
      return { ok: false, code: 'invalid_task_id' }
    }
    return { ok: true, params: { task_id: taskId } }
  }

  if (identifier === null || identifier.trim().length === 0) {
    return { ok: false, code: 'invalid_identifier' }
  }

  let options: AppInstallerLaunchOptions | undefined
  if (serializedOptions !== null) {
    if (serializedOptions.length > 16_384) return { ok: false, code: 'invalid_options' }
    try {
      const result = launchOptionsSchema.safeParse(JSON.parse(serializedOptions))
      if (!result.success) return { ok: false, code: 'invalid_options' }
      options = result.data
    } catch {
      return { ok: false, code: 'invalid_options' }
    }
  }

  const parsed = appInstallerLaunchParamsSchema.safeParse({
    identifier,
    ...(ref !== null ? { ref } : {}),
    ...(options ? { options } : {}),
  })
  if (!parsed.success || 'task_id' in parsed.data) {
    return { ok: false, code: 'invalid_identifier' }
  }

  if (!validateTarget(parsed.data.options?.target).ok) {
    return { ok: false, code: 'invalid_target' }
  }
  return { ok: true, params: parsed.data }
}

function validateAppInstallerLaunchParams(value: unknown): AppInstallerLaunchQueryResult {
  const parsed = appInstallerLaunchParamsSchema.safeParse(value)
  if (!parsed.success) {
    if (value && typeof value === 'object' && 'task_id' in value) {
      return { ok: false, code: 'invalid_task_id' }
    }
    return { ok: false, code: 'invalid_identifier' }
  }
  if ('options' in parsed.data && !validateTarget(parsed.data.options?.target).ok) {
    return { ok: false, code: 'invalid_target' }
  }
  return { ok: true, params: parsed.data }
}

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

function formatPrice(
  price: AppPrice | null,
  locale: string,
  t: ReturnType<typeof useI18n>['t'],
) {
  if (!price || price.amount === 0) return t('appService.install.free', 'Free')
  return new Intl.NumberFormat(locale, {
    style: 'currency',
    currency: price.currency,
  }).format(price.amount)
}

function formatPublishedAt(
  publishedAt: string,
  locale: string,
  t: ReturnType<typeof useI18n>['t'],
) {
  const date = new Date(publishedAt)
  if (Number.isNaN(date.getTime())) return publishedAt

  const dayMs = 86_400_000
  const relativeDays = Math.round((date.getTime() - Date.now()) / dayMs)
  if (Math.abs(relativeDays) >= 365) {
    return t('appService.install.publishedOn', 'Published {{date}}', {
      date: new Intl.DateTimeFormat(locale, { dateStyle: 'long' }).format(date),
    })
  }

  let value = relativeDays
  let unit: Intl.RelativeTimeFormatUnit = 'day'
  if (Math.abs(relativeDays) >= 30) {
    value = Math.round(relativeDays / 30)
    unit = 'month'
  } else if (Math.abs(relativeDays) >= 7) {
    value = Math.round(relativeDays / 7)
    unit = 'week'
  }
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(value, unit)
  return t('appService.install.released', 'Released {{time}}', { time: relative })
}

type TrustLevel = 'strongest' | 'trusted' | 'caution' | 'unresolved' | 'untrusted'

function getTrustLevel(checks: TrustCheck[]): TrustLevel {
  if (checks.some((check) => check.status === 'failed')) return 'untrusted'
  if (checks.some((check) => check.status === 'pending')) return 'unresolved'
  if (checks.some((check) => check.status === 'unknown')) return 'caution'
  if (checks.some((check) => check.status === 'warning')) return 'trusted'
  return 'strongest'
}

function trustLevelColor(level: TrustLevel) {
  switch (level) {
    case 'strongest': return 'light-dark(oklch(42% 0.13 155), oklch(76% 0.14 155))'
    case 'trusted': return 'light-dark(oklch(52% 0.15 145), oklch(79% 0.14 145))'
    case 'caution': return 'light-dark(oklch(59% 0.15 90), oklch(84% 0.15 90))'
    case 'unresolved': return 'light-dark(oklch(58% 0.18 48), oklch(79% 0.17 55))'
    case 'untrusted': return 'light-dark(oklch(50% 0.2 27), oklch(75% 0.18 27))'
  }
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
  onClose,
  children,
}: {
  onClose: () => void
  children: React.ReactNode
}) {
  const { t } = useI18n()

  return (
    <div
      className="relative mx-auto overflow-hidden rounded-[24px]"
      data-testid="app-installer-dialog"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-window-shadow)' }}
    >
      <header className="border-b px-5 py-4 sm:px-6" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="flex items-center justify-between gap-4">
          <h1 className="font-display text-xl font-semibold sm:text-2xl" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.installApp', 'Install application')}
          </h1>
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
      </header>
      {children}
    </div>
  )
}

function AppIdentity({ app }: { app: InstallAppInfo }) {
  const { locale, t } = useI18n()
  const ownerProfileUrl = `/userprofile?user=${encodeURIComponent(app.ownerDid)}`
  return (
    <div className="flex items-start gap-4">
      <div
        className="flex size-14 shrink-0 items-center justify-center rounded-[16px]"
        style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
      >
        <AppIcon iconKey={app.iconKey} className="!size-7" />
      </div>
      <div className="min-w-0 flex-1">
        <h2 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{app.name}</h2>
        <div className="mt-2 flex flex-wrap items-start gap-x-6 gap-y-3">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-semibold tabular-nums" style={{ color: 'var(--cp-text)' }}>v{app.version}</span>
              {app.isLatest && (
                <span
                  className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
                  style={{ color: 'var(--cp-accent)', background: 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))' }}
                >
                  {t('appService.install.latest', 'Latest')}
                </span>
              )}
            </div>
            <div className="mt-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
              {formatPublishedAt(app.publishedAt, locale, t)}
            </div>
          </div>
          <a
            href={ownerProfileUrl}
            className="group/author min-w-0 text-xs leading-5"
            aria-label={t('appService.install.viewAuthorProfile', 'View {{author}} public profile', { author: app.publisher })}
            title={app.ownerDid}
            style={{ color: 'var(--cp-muted)' }}
          >
            <span>{t('appService.install.byAuthor', 'By')} </span>
            <span className="font-semibold underline decoration-transparent underline-offset-4 transition-colors group-hover/author:decoration-current" style={{ color: 'var(--cp-text)' }}>
              {app.publisher}
            </span>
          </a>
        </div>
        <p className="mt-3 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{app.description}</p>
        <div className="mt-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {formatPrice(app.price, locale, t)}
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

function InstallReadiness({ app }: { app: InstallAppInfo }) {
  const { t } = useI18n()
  if (app.installReady) {
    return (
      <div
        className="flex items-start gap-3 rounded-[16px] p-4"
        data-testid="app-installer-install-readiness"
        style={{ background: 'color-mix(in srgb, var(--cp-success) 7%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-success) 24%, var(--cp-border))' }}
      >
        <CheckCircle2 size={18} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-success)' }} />
        <div>
          <div className="text-xs font-semibold" style={{ color: 'var(--cp-success)' }}>
            {t('appService.install.readyToInstall', 'Ready to install')}
          </div>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.readyToInstallBody', 'Trust, platform, and content checks allow this application to continue.')}
          </p>
        </div>
      </div>
    )
  }

  const reason = app.blockingReason
  const label = reason
    ? t(`appService.install.block.${reason}.title`, reason)
    : t('appService.install.checksIncomplete', 'Required installation checks did not pass')
  const body = reason
    ? t(`appService.install.block.${reason}.body`, 'Resolve this issue before continuing installation.')
    : t('appService.install.checksIncompleteBody', 'Review the application information or choose another source.')
  return (
    <div
      className="flex items-start gap-3 rounded-[16px] p-4"
      data-testid="app-installer-blocking-reason"
      style={{ background: 'color-mix(in srgb, var(--cp-danger) 7%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-danger) 24%, var(--cp-border))' }}
    >
      <ShieldAlert size={18} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-danger)' }} />
      <div>
        <div className="text-xs font-semibold" style={{ color: 'var(--cp-danger)' }}>
          {t('appService.install.cannotInstall', 'This application cannot be installed')}
        </div>
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
          <span className="font-semibold">{label}.</span> {body}
        </p>
      </div>
    </div>
  )
}

function LaunchRequestEvidence({ request }: { request: InstallLaunchRequest }) {
  const { t } = useI18n()
  return (
    <section className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
      <h3 className="text-xs font-semibold uppercase tracking-[0.12em]" style={{ color: 'var(--cp-muted)' }}>
        {t('appService.install.launchRequest', 'Launch request')}
      </h3>
      <dl className="mt-3">
        <InfoRow
          label={t('appService.install.requestedTarget', 'Requested target')}
          value={request.targetNode ?? t('appService.install.systemSelectedTarget', 'System-selected node')}
          code={Boolean(request.targetNode)}
        />
        <InfoRow
          label={t('appService.install.networkAcquisition', 'Network acquisition')}
          value={request.offline
            ? t('appService.install.forbidden', 'Forbidden')
            : t('appService.install.allowedWhenNeeded', 'Allowed when needed')}
        />
      </dl>
      {request.installParams && (
        <div className="mt-3 border-t pt-3" style={{ borderColor: 'var(--cp-border)' }}>
          <div className="text-[11px] font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.installParams', 'Application install parameters')}
          </div>
          <pre className="desktop-scrollbar mt-2 max-h-36 overflow-auto rounded-xl p-3 text-[11px] leading-5" style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            {JSON.stringify(request.installParams, null, 2)}
          </pre>
        </div>
      )}
    </section>
  )
}

function VerifyStep({
  task,
  onBack,
  onContinue,
  onEnd,
}: {
  task: InstallTask
  onBack: () => void
  onContinue: () => void
  onEnd: () => void
}) {
  const { t } = useI18n()
  const { app } = task
  const trustLevel = getTrustLevel(app.trustChecks)

  return (
    <div className="space-y-6 p-5 sm:p-6">
      <AppIdentity app={app} />
      <section
        className="rounded-[16px] px-4 py-3.5"
        style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
      >
        <h3 className="text-xs font-semibold uppercase tracking-[0.12em]" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.details', 'Details')}
        </h3>
        <p className="mt-2 text-sm leading-6" style={{ color: 'var(--cp-text)' }}>{app.details}</p>
      </section>

      <section
        className="flex flex-col gap-2 rounded-[16px] px-4 py-3"
        style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11px]">
          <Server size={15} className="shrink-0" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
          <span className="font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.platform', 'Target platform')}</span>
          <span style={{ color: 'var(--cp-muted)' }}>{t('appService.install.platformDetail', 'Linux · aarch64 · Docker 26+')}</span>
          <span className="font-semibold" style={{ color: app.platformSupported ? 'var(--cp-success)' : 'var(--cp-danger)' }}>
            {app.platformSupported ? t('appService.install.supported', 'Supported') : t('appService.install.unsupported', 'Unsupported')}
          </span>
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11px]">
          <CloudDownload size={15} className="shrink-0" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
          <span className="font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.contentReadiness', 'Content readiness')}</span>
          <span className="font-semibold" style={{ color: app.content.offlineReady ? 'var(--cp-success)' : 'var(--cp-warning)' }}>
            {app.content.offlineReady ? t('appService.install.offlineReady', 'Offline ready') : t('appService.install.downloadRequired', 'Download required')}
          </span>
          <span style={{ color: 'var(--cp-muted)' }}>
            {t('appService.install.packageSize', 'Package')} {formatBytes(app.content.packageBytes)} · {t('appService.install.downloadSize', 'Download')} {app.content.missingBytes > 0 ? formatBytes(app.content.missingBytes) : t('appService.install.notRequired', 'Not required')} · {t('appService.install.expectedInstallSize', 'Installed')} ~{formatBytes(app.content.expectedInstallBytes)}
          </span>
        </div>
      </section>

      <details
        className="group overflow-hidden rounded-[16px]"
        data-testid="app-installer-trust-evidence"
        style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
      >
        <summary className="flex min-h-12 cursor-pointer list-none items-center gap-3 px-4 py-3 [&::-webkit-details-marker]:hidden">
          <ShieldCheck size={16} className="shrink-0" aria-hidden="true" style={{ color: trustLevelColor(trustLevel) }} />
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
            <span className="font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.trustEvidence', 'Trust evidence')}</span>
            <span className="font-semibold" style={{ color: trustLevelColor(trustLevel) }}>
              {t(`appService.install.trustLevel.${trustLevel}`, trustLevel)}
            </span>
            <span style={{ color: 'var(--cp-muted)' }}>{t(`appService.install.trustReason.${trustLevel}`, trustLevel)}</span>
          </div>
          <ChevronDown size={15} className="shrink-0 transition-transform duration-200 group-open:rotate-180" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
        </summary>
        <div style={{ borderTop: '1px solid var(--cp-border)' }}>
          {app.trustChecks.map((check, index) => (
            <div
              key={check.code}
              className="flex items-start gap-3 px-4 py-3"
              style={{ borderTop: index === 0 ? undefined : '1px solid var(--cp-border)' }}
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
      </details>

      <details
        className="group overflow-hidden rounded-[16px]"
        data-testid="app-installer-source-identity"
        style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
      >
        <summary className="flex min-h-12 cursor-pointer list-none items-center gap-3 px-4 py-3 [&::-webkit-details-marker]:hidden">
          <FileArchive size={16} className="shrink-0" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
            <span className="font-semibold" style={{ color: 'var(--cp-text)' }}>{t('appService.install.sourceAndIdentity', 'Source and identity')}</span>
            <span className="truncate" style={{ color: 'var(--cp-muted)' }}>{sourceKindLabel(app.source.kind, t)} · {app.publisher}</span>
          </div>
          <ChevronDown size={15} className="shrink-0 transition-transform duration-200 group-open:rotate-180" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
        </summary>
        <dl className="px-4 py-3" style={{ borderTop: '1px solid var(--cp-border)' }}>
          <InfoRow label={t('appService.install.inputType', 'Input type')} value={sourceKindLabel(app.source.kind, t)} />
          <InfoRow label={t('appService.install.source', 'Source')} value={app.source.displaySource} />
          <InfoRow label={t('appService.install.appDid', 'App DID')} value={app.appDid} code />
          <InfoRow label={t('appService.install.objectId', 'Document Object ID')} value={app.documentObjectId} code />
          <InfoRow label={t('appService.install.publisher', 'Publisher')} value={app.publisher} />
          <InfoRow label={t('appService.install.ownerDid', 'Owner DID')} value={app.ownerDid} code />
          <InfoRow label={t('appService.install.referrer', 'Referrer')} value={app.referrer} />
        </dl>
      </details>

      {task.launchRequest && <LaunchRequestEvidence request={task.launchRequest} />}

      {app.source.warningCode === 'UNSIGNED_CANDIDATE' && !app.blockingReason && (
        <div className="flex items-start gap-3 rounded-[16px] p-4" style={{ background: 'color-mix(in srgb, var(--cp-warning) 8%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-warning) 25%, var(--cp-border))' }}>
          <AlertTriangle size={17} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
          <p className="text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.unsignedWarning', 'This App Meta JSON is unsigned. Installation is allowed only because its Object ID matches the document currently published by the App DID.')}
          </p>
        </div>
      )}

      <InstallReadiness app={app} />

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
        {app.installReady ? (
          <button
            type="button"
            onClick={onContinue}
            className="min-h-11 rounded-xl px-5 text-sm font-semibold"
            style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
          >
            {t('appService.install.reviewPlan', 'Review installation plan')}
          </button>
        ) : (
          <button
            type="button"
            onClick={onEnd}
            className="min-h-11 rounded-xl px-5 text-sm font-semibold"
            style={{ color: 'var(--cp-surface)', background: 'var(--cp-text)' }}
          >
            {t('appService.install.end', 'End')}
          </button>
        )}
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

function SudoPasswordDialog({
  appName,
  onCancel,
  onConfirm,
}: {
  appName: string
  onCancel: () => void
  onConfirm: () => void
}) {
  const { t } = useI18n()
  const form = useForm<InstallerSudoInput>({
    resolver: zodResolver(installerSudoSchema),
    defaultValues: { password: '' },
  })

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      data-testid="app-installer-sudo-dialog"
    >
      <div
        className="absolute inset-0"
        aria-hidden="true"
        style={{ background: 'color-mix(in srgb, var(--cp-shadow) 30%, transparent)', backdropFilter: 'blur(2px)' }}
      />
      <section
        aria-label={t('appService.install.sudoTitle', 'sudo authorization')}
        aria-modal="true"
        role="dialog"
        className="relative w-full max-w-md overflow-hidden rounded-[22px]"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-window-shadow)' }}
      >
        <header className="border-b px-5 py-4" style={{ borderColor: 'var(--cp-border)' }}>
          <div className="flex items-center gap-3">
            <span
              className="flex size-10 shrink-0 items-center justify-center rounded-full"
              style={{ color: 'var(--cp-accent)', background: 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))' }}
            >
              <KeyRound size={18} aria-hidden="true" />
            </span>
            <div>
              <h2 className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('appService.install.sudoTitle', 'sudo authorization')}
              </h2>
              <p className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
                {t('appService.install.sudoDescription', 'Enter the administrator password to install {{name}}.', { name: appName })}
              </p>
            </div>
          </div>
        </header>
        <form
          className="space-y-4 p-5"
          onSubmit={form.handleSubmit(() => onConfirm())}
          noValidate
        >
          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.adminPassword', 'Administrator password')}
            </span>
            <input
              autoComplete="current-password"
              autoFocus
              type="password"
              {...form.register('password')}
              className="aicc-password-input min-h-11 w-full rounded-xl px-3 text-sm outline-none"
              aria-invalid={Boolean(form.formState.errors.password)}
              style={{
                color: 'var(--cp-text)',
                background: 'var(--cp-surface-2)',
                border: form.formState.errors.password
                  ? '1px solid var(--cp-danger)'
                  : '1px solid var(--cp-border)',
              }}
            />
            <span className="mt-1.5 block text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.adminPasswordHint', 'Used only for this sudo authorization and never stored in task history.')}
            </span>
            {form.formState.errors.password ? (
              <span className="mt-1.5 block text-xs" style={{ color: 'var(--cp-danger)' }}>
                {t('appService.install.passwordRequired', 'Enter the administrator password to continue.')}
              </span>
            ) : null}
          </label>
          <div className="flex flex-col-reverse gap-2 pt-1 sm:flex-row sm:justify-end">
            <button
              type="button"
              onClick={onCancel}
              className="min-h-11 rounded-xl px-4 text-sm font-semibold"
              style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
            >
              {t('common.cancel', 'Cancel')}
            </button>
            <button
              type="submit"
              className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-5 text-sm font-semibold"
              style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
            >
              <ShieldCheck size={15} aria-hidden="true" />
              {t('appService.install.confirmInstall', 'Authorize and install')}
            </button>
          </div>
        </form>
      </section>
    </div>
  )
}

function ApprovalStep({ task, onBack }: { task: InstallTask; onBack: () => void }) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const [pendingApproval, setPendingApproval] = useState<InstallerApprovalInput | null>(null)
  const form = useForm<InstallerApprovalInput>({
    resolver: zodResolver(installerApprovalSchema),
    mode: 'onBlur',
    defaultValues: structuredClone(task.plan.options),
  })
  const {
    fields: mountFields,
    append: appendMount,
    remove: removeMount,
  } = useFieldArray({ control: form.control, name: 'mounts' })
  const {
    fields: envFields,
    append: appendEnv,
    remove: removeEnv,
  } = useFieldArray({ control: form.control, name: 'envVars' })
  const serviceSettings = useWatch({ control: form.control, name: 'serviceSettings' })
  const permissionGrants = useWatch({ control: form.control, name: 'permissionGrants' })
  const launchRequest = task.launchRequest
  const permissions = [...task.app.permissions].sort((left, right) => {
    if (left.risk === right.risk) return left.kind.localeCompare(right.kind)
    return left.risk === 'high' ? -1 : 1
  })
  const riskyParams = [
    { name: 'start_param', value: task.app.startParam },
    { name: 'container_param', value: task.app.containerParam },
  ].filter((item): item is { name: string; value: string } => Boolean(item.value))

  const registerPath = (path: string) => form.register(path as never)
  const setValue = (path: string, value: unknown) => {
    form.setValue(path as never, value as never, { shouldDirty: true, shouldValidate: true })
  }

  return (
    <>
      <form
        className="space-y-6 p-5 sm:p-6"
        onSubmit={form.handleSubmit((values) => setPendingApproval(values))}
        noValidate
      >
        <div>
          <h2 className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('appService.install.optionsTitle', 'Configure application')}
          </h2>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.install.optionsBody', 'Review the important access, storage, environment, and permission settings. Defaults are ready to use.')}
          </p>
        </div>

        <section
          className="space-y-4 rounded-[18px] p-4"
          data-testid="app-installer-access-settings"
          style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
        >
          <div>
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.accessSettings', 'Access settings')}
            </h3>
            <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.accessSettingsHint', 'Choose how people and services reach this application.')}
            </p>
          </div>

          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.fullAppHost', 'Full application host')}
            </span>
            <input
              readOnly
              value={task.app.appHost}
              className="min-h-11 w-full rounded-xl px-3 font-mono text-sm outline-none"
              style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            />
          </label>

          <label className="block">
            <span className="mb-1.5 block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.shortcutDomain', 'Shortcut domain')}
            </span>
            <select
              {...form.register('shortcutDomain')}
              className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
              style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            >
              <option value="">{t('appService.install.noShortcutDomain', 'No shortcut')}</option>
              {task.app.shortcutDomains.map((domain) => (
                <option key={domain} value={domain}>{domain}</option>
              ))}
            </select>
          </label>

          <fieldset className="space-y-3">
            <legend className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.portExposure', 'Port exposure')}
            </legend>
            {task.plan.options.serviceSettings.map((service, index) => {
              const current = serviceSettings[index] ?? service
              const route = current.expose.route
              return (
                <div
                  key={service.serviceName}
                  className="space-y-3 rounded-[14px] p-3"
                  style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
                >
                  <label className="flex min-h-11 items-center justify-between gap-4">
                    <span className="min-w-0">
                      <span className="block text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>{service.label}</span>
                      <span className="mt-0.5 block text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                        {service.serviceName} · {service.protocol.toUpperCase()} · {t('appService.install.containerPort', 'container port')} {service.innerPort}
                      </span>
                    </span>
                    <input
                      type="checkbox"
                      {...registerPath('serviceSettings.' + index + '.enabled')}
                      className="size-4 shrink-0 accent-[var(--cp-accent)]"
                    />
                  </label>
                  <label className="block">
                    <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                      {t('appService.install.exposureRoute', 'Exposure route')}
                    </span>
                    <select
                      value={route.type}
                      onChange={(event) => {
                        setValue(
                          'serviceSettings.' + index + '.expose.route',
                          event.target.value === 'port'
                            ? { type: 'port', exposePort: service.innerPort }
                            : { type: 'web', subHostname: [] },
                        )
                      }}
                      className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                    >
                      <option value="web">{t('appService.install.webRoute', 'Application HTTPS host')}</option>
                      <option value="port">{t('appService.install.directPort', 'Direct Zone port')}</option>
                    </select>
                  </label>
                  {route.type === 'port' ? (
                    <label className="block">
                      <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                        {t('appService.install.exposedPort', 'Exposed port')}
                      </span>
                      <input
                        type="number"
                        min={1}
                        max={65535}
                        value={route.exposePort}
                        onChange={(event) => {
                          setValue(
                            'serviceSettings.' + index + '.expose.route',
                            { type: 'port', exposePort: Number(event.target.value) },
                          )
                        }}
                        className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                        style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                      />
                    </label>
                  ) : null}
                  <label className="block">
                    <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                      {t('appService.install.exposureScope', 'Scope')}
                    </span>
                    <input
                      {...registerPath('serviceSettings.' + index + '.expose.scope')}
                      placeholder={t('appService.install.zoneScope', 'Zone users')}
                      className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                    />
                  </label>
                  <label className="flex min-h-11 items-center justify-between gap-4">
                    <span className="text-xs font-medium" style={{ color: 'var(--cp-text)' }}>
                      {t('appService.install.allowGuest', 'Allow guest access')}
                    </span>
                    <input
                      type="checkbox"
                      {...registerPath('serviceSettings.' + index + '.expose.allowGuest')}
                      className="size-4 shrink-0 accent-[var(--cp-accent)]"
                    />
                  </label>
                </div>
              )
            })}
          </fieldset>
        </section>

        <section
          className="space-y-4 rounded-[18px] p-4"
          data-testid="app-installer-mount-settings"
          style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
        >
          <div>
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.directoryMounts', 'Directory mounts')}
            </h3>
            <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.directoryMountsHint', 'Choose declared data directories or add an explicit container mapping.')}
            </p>
          </div>
          <div className="space-y-3">
            {mountFields.map((field, index) => (
              <div
                key={field.id}
                className="space-y-3 rounded-[14px] p-3"
                style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
              >
                <div className="flex min-h-8 items-start justify-between gap-3">
                  {field.declared ? (
                    <label className="flex min-w-0 items-start gap-3">
                      <input
                        type="checkbox"
                        {...registerPath('mounts.' + index + '.enabled')}
                        className="mt-0.5 size-4 shrink-0 accent-[var(--cp-accent)]"
                      />
                      <span className="min-w-0 text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                        {field.name}
                        <span className="ml-1 font-normal" style={{ color: 'var(--cp-muted)' }}>({field.containerPath})</span>
                      </span>
                    </label>
                  ) : (
                    <span className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                      {t('appService.install.customMapping', 'Custom mapping')}
                    </span>
                  )}
                  {!field.declared ? (
                    <button
                      type="button"
                      onClick={() => removeMount(index)}
                      className="flex size-10 shrink-0 items-center justify-center rounded-xl"
                      aria-label={t('appService.install.removeMapping', 'Remove mapping')}
                      style={{ color: 'var(--cp-danger)', border: '1px solid var(--cp-border)' }}
                    >
                      <Trash2 size={15} aria-hidden="true" />
                    </button>
                  ) : null}
                </div>
                {!field.declared ? (
                  <label className="block">
                    <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                      {t('appService.install.containerPath', 'Container path')}
                    </span>
                    <input
                      {...registerPath('mounts.' + index + '.containerPath')}
                      className="min-h-11 w-full rounded-xl px-3 font-mono text-sm outline-none"
                      aria-invalid={Boolean(form.formState.errors.mounts?.[index]?.containerPath)}
                      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                    />
                  </label>
                ) : null}
                <label className="block">
                  <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                    {t('appService.install.targetDirectory', 'Mapped directory')}
                  </span>
                  <input
                    {...registerPath('mounts.' + index + '.targetPath')}
                    className="min-h-11 w-full rounded-xl px-3 font-mono text-sm outline-none"
                    aria-invalid={Boolean(form.formState.errors.mounts?.[index]?.targetPath)}
                    style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                  />
                </label>
                {!field.declared ? (
                  <label className="block">
                    <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                      {t('appService.install.mountAccess', 'Access')}
                    </span>
                    <select
                      {...registerPath('mounts.' + index + '.access')}
                      className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                    >
                      <option value="read_only">{t('appService.install.readOnly', 'Read only')}</option>
                      <option value="read_write">{t('appService.install.readWrite', 'Read and write')}</option>
                      <option value="read_write_append">{t('appService.install.readWriteAppend', 'Read, write, and append')}</option>
                    </select>
                  </label>
                ) : null}
                {form.formState.errors.mounts?.[index] ? (
                  <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>
                    {t('appService.install.mountPathError', 'Both paths must be absolute and begin with /.')}
                  </p>
                ) : null}
              </div>
            ))}
          </div>
          <button
            type="button"
            onClick={() => appendMount({
              name: 'Custom mapping',
              containerPath: '/container/path',
              targetPath: '/data/path',
              access: 'read_write',
              enabled: true,
              declared: false,
            })}
            className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            <Plus size={15} aria-hidden="true" />
            {t('appService.install.addMapping', 'Add mapping')}
          </button>
        </section>

        <section
          className="space-y-4 rounded-[18px] p-4"
          data-testid="app-installer-environment-settings"
          style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
        >
          <div>
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.environmentVariables', 'Environment variables')}
            </h3>
            <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.environmentVariablesHint', 'Configure values declared by the application or add another variable.')}
            </p>
          </div>
          <div className="space-y-3">
            {envFields.map((field, index) => (
              <div
                key={field.id}
                className="space-y-3 rounded-[14px] p-3"
                style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
              >
                <div className="flex items-start justify-between gap-3">
                  {field.declared ? (
                    <div className="min-w-0">
                      <div className="break-all font-mono text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                        {field.name}
                        {field.required ? (
                          <span className="ml-2 font-sans text-[10px] uppercase tracking-wide" style={{ color: 'var(--cp-warning)' }}>
                            {t('appService.install.required', 'Required')}
                          </span>
                        ) : null}
                      </div>
                      <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>{field.description}</p>
                    </div>
                  ) : (
                    <label className="min-w-0 flex-1">
                      <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                        {t('appService.install.variableName', 'Name')}
                      </span>
                      <input
                        {...registerPath('envVars.' + index + '.name')}
                        className="min-h-11 w-full rounded-xl px-3 font-mono text-sm outline-none"
                        aria-invalid={Boolean(form.formState.errors.envVars?.[index]?.name)}
                        style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                      />
                    </label>
                  )}
                  {!field.declared ? (
                    <button
                      type="button"
                      onClick={() => removeEnv(index)}
                      className="mt-[22px] flex size-10 shrink-0 items-center justify-center rounded-xl"
                      aria-label={t('appService.install.removeVariable', 'Remove variable')}
                      style={{ color: 'var(--cp-danger)', border: '1px solid var(--cp-border)' }}
                    >
                      <Trash2 size={15} aria-hidden="true" />
                    </button>
                  ) : null}
                </div>
                <label className="block">
                  <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                    {t('appService.install.variableValue', 'Value')}
                  </span>
                  <input
                    {...registerPath('envVars.' + index + '.value')}
                    className="min-h-11 w-full rounded-xl px-3 font-mono text-sm outline-none"
                    style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
                  />
                </label>
                {form.formState.errors.envVars?.[index] ? (
                  <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>
                    {t('appService.install.environmentError', 'Use a valid environment variable name.')}
                  </p>
                ) : null}
              </div>
            ))}
          </div>
          <button
            type="button"
            onClick={() => appendEnv({
              name: '',
              value: '',
              description: '',
              required: false,
              declared: false,
            })}
            className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            <Plus size={15} aria-hidden="true" />
            {t('appService.install.addVariable', 'Add variable')}
          </button>
        </section>

        {riskyParams.length > 0 ? (
          <section
            className="space-y-3 rounded-[18px] p-4"
            data-testid="app-installer-risky-params"
            style={{
              background: 'color-mix(in srgb, var(--cp-danger) 7%, var(--cp-surface))',
              border: '1px solid color-mix(in srgb, var(--cp-danger) 28%, var(--cp-border))',
            }}
          >
            <div className="flex items-start gap-3">
              <AlertOctagon size={18} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-danger)' }} />
              <div>
                <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-danger)' }}>
                  {t('appService.install.otherParameters', 'Other parameters')}
                </h3>
                <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
                  {t('appService.install.highRiskParametersWarning', 'High risk: these parameters can change the container process or runtime isolation. Continue only if you trust the application publisher.')}
                </p>
              </div>
            </div>
            <dl className="overflow-hidden rounded-[14px]" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
              {riskyParams.map((item, index) => (
                <div
                  key={item.name}
                  className="space-y-1 px-3 py-3"
                  style={{ borderTop: index === 0 ? undefined : '1px solid var(--cp-border)' }}
                >
                  <dt className="font-mono text-[11px] font-semibold" style={{ color: 'var(--cp-danger)' }}>{item.name}</dt>
                  <dd className="break-all font-mono text-xs" style={{ color: 'var(--cp-text)' }}>{item.value}</dd>
                </div>
              ))}
            </dl>
          </section>
        ) : null}

        {(launchRequest?.offline || launchRequest?.installParams) ? (
          <details
            className="group overflow-hidden rounded-[16px]"
            style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
          >
            <summary className="flex min-h-12 cursor-pointer list-none items-center gap-3 px-4 py-3 [&::-webkit-details-marker]:hidden">
              <div className="min-w-0 flex-1">
                <span className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                  {t('appService.install.callerSuggestions', 'Caller-provided suggestions')}
                </span>
                <span className="ml-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                  {t('appService.install.callerSuggestionsHint', 'Review before authorizing.')}
                </span>
              </div>
              <ChevronDown size={15} className="shrink-0 transition-transform duration-200 group-open:rotate-180" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
            </summary>
            <div className="space-y-3 px-4 py-3" style={{ borderTop: '1px solid var(--cp-border)' }}>
              {launchRequest.offline ? (
                <div className="inline-flex rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide" style={{ color: 'var(--cp-warning)', background: 'color-mix(in srgb, var(--cp-warning) 12%, transparent)' }}>
                  {t('appService.install.offlineOnly', 'Offline acquisition only')}
                </div>
              ) : null}
              {launchRequest.installParams ? (
                <pre className="desktop-scrollbar max-h-36 overflow-auto rounded-xl p-3 text-[11px] leading-5" style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
                  {JSON.stringify(launchRequest.installParams, null, 2)}
                </pre>
              ) : null}
            </div>
          </details>
        ) : null}

        <details
          open
          className="group overflow-hidden rounded-[18px]"
          data-testid="app-installer-permissions"
          style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
        >
          <summary className="flex min-h-12 cursor-pointer list-none items-center gap-3 px-4 py-3 [&::-webkit-details-marker]:hidden">
            <ShieldAlert size={17} className="shrink-0" aria-hidden="true" style={{ color: 'var(--cp-warning)' }} />
            <div className="min-w-0 flex-1">
              <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('appService.install.permissionRequests', 'Permission requests')}
              </span>
              <span className="ml-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {t('appService.install.permissionCount', '{{count}} requested', { count: permissions.length })}
              </span>
            </div>
            <ChevronDown size={15} className="shrink-0 transition-transform duration-200 group-open:rotate-180" aria-hidden="true" style={{ color: 'var(--cp-muted)' }} />
          </summary>
          <div style={{ borderTop: '1px solid var(--cp-border)' }}>
            {permissions.map((permission, index) => {
              const grantIndex = permissionGrants.findIndex((item) => item.scope === permission.scope)
              return (
                <div
                  key={permission.scope}
                  className="space-y-3 px-4 py-4"
                  style={{ borderTop: index === 0 ? undefined : '1px solid var(--cp-border)' }}
                >
                  <div className="flex items-start gap-3">
                    <PermissionIcon kind={permission.kind} />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                          {t('appService.install.permission.' + permission.kind, permission.kind)}
                        </span>
                        {permission.risk === 'high' ? (
                          <span
                            className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
                            style={{ color: 'var(--cp-danger)', background: 'color-mix(in srgb, var(--cp-danger) 10%, transparent)' }}
                          >
                            {t('appService.install.highRisk', 'High risk')}
                          </span>
                        ) : null}
                        {permission.required ? (
                          <span className="text-[10px] font-semibold uppercase tracking-wide" style={{ color: 'var(--cp-muted)' }}>
                            {t('appService.install.required', 'Required')}
                          </span>
                        ) : null}
                      </div>
                      <div className="mt-1 break-all font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }}>{permission.scope}</div>
                      <p className="mt-2 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>{permission.detail}</p>
                    </div>
                  </div>
                  <label className="block">
                    <span className="mb-1.5 block text-[11px] font-semibold" style={{ color: 'var(--cp-muted)' }}>
                      {t('appService.install.permissionGrant', 'Permission granted by you')}
                    </span>
                    <select
                      {...registerPath('permissionGrants.' + Math.max(0, grantIndex) + '.grant')}
                      className="min-h-11 w-full rounded-xl px-3 text-sm outline-none"
                      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
                    >
                      {permission.grantOptions.map((option) => (
                        <option key={option} value={option}>
                          {t('appService.install.permissionGrant.' + option, option)}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              )
            })}
          </div>
        </details>

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
            {t('appService.install.next', 'Next')}
          </button>
        </footer>
      </form>

      {pendingApproval ? (
        <SudoPasswordDialog
          appName={task.app.name}
          onCancel={() => setPendingApproval(null)}
          onConfirm={() => {
            store.approveTask(task.taskId, pendingApproval)
            setPendingApproval(null)
          }}
        />
      ) : null}
    </>
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
  const store = useSharedAppServiceStore()
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

export interface AppInstallerProps {
  launchParams: AppInstallerLaunchParams
  onBackground: () => void
  onChangeSource: () => void
  onClose: () => void
  onViewApp: (serviceId: string) => void
  onTaskCreated?: (taskId: string) => void
}

function LaunchStateCard({
  error,
  onClose,
}: {
  error?: AppInstallerLaunchErrorCode
  onClose: () => void
}) {
  const { t } = useI18n()

  return (
    <div
      className="mx-auto w-full max-w-xl rounded-[22px] p-6"
      data-testid={error ? 'app-installer-launch-error' : 'app-installer-launch-loading'}
      role={error ? 'alert' : 'status'}
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-window-shadow)' }}
    >
      {error ? (
        <AlertTriangle size={24} style={{ color: 'var(--cp-danger)' }} aria-hidden="true" />
      ) : (
        <Loader2 size={24} className="animate-spin" style={{ color: 'var(--cp-accent)' }} aria-hidden="true" />
      )}
      <h1 className="mt-4 font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {error
          ? t('appService.install.launchErrorTitle', 'Cannot open App Installer')
          : t('appService.install.resolvingSource', 'Resolving installation source')}
      </h1>
      <p className="mt-2 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
        {error
          ? t(`appService.install.launchError.${error}`, 'The launch parameters are invalid or unsupported.')
          : t('appService.install.resolvingSourceBody', 'The source is being normalized before an installation task is created.')}
      </p>
      {error && (
        <button
          type="button"
          onClick={onClose}
          className="mt-5 min-h-11 rounded-xl px-4 text-sm font-semibold"
          style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface)' }}
        >
          {t('common.close', 'Close')}
        </button>
      )}
    </div>
  )
}

export function AppInstaller({
  launchParams,
  onBackground,
  onChangeSource,
  onClose,
  onViewApp,
  onTaskCreated,
}: AppInstallerProps) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const [approvalOpen, setApprovalOpen] = useState(false)
  const validatedLaunch = useMemo(
    () => validateAppInstallerLaunchParams(launchParams),
    [launchParams],
  )
  const [launchState, setLaunchState] = useState<
    | { status: 'loading' }
    | { status: 'ready'; taskId: string }
    | { status: 'error'; code: AppInstallerLaunchErrorCode }
  >(() => {
    if (!validatedLaunch.ok) {
      return { status: 'error', code: validatedLaunch.code }
    }
    if ('task_id' in validatedLaunch.params) {
      return { status: 'ready', taskId: validatedLaunch.params.task_id }
    }
    return { status: 'loading' }
  })

  useEffect(() => {
    if (!validatedLaunch.ok || 'task_id' in validatedLaunch.params) return
    const identifierParams = validatedLaunch.params

    let cancelled = false
    const createTask = async () => {
      const source = await store.analyzeInstallSource(identifierParams.identifier)
      if (cancelled) return
      if (!source.ok) {
        setLaunchState({ status: 'error', code: 'source_unrecognized' })
        return
      }

      const target = validateTarget(identifierParams.options?.target)
      if (!target.ok) {
        setLaunchState({ status: 'error', code: 'invalid_target' })
        return
      }
      const request: InstallLaunchRequest = {
        identifier: identifierParams.identifier,
        referrer: identifierParams.ref,
        targetNode: target.targetNode,
        offline: identifierParams.options?.offline ?? false,
        installParams: identifierParams.options?.install_params,
      }
      const taskId = store.createInstallTask(source.source, request)
      if (cancelled) return
      setLaunchState({ status: 'ready', taskId })
      onTaskCreated?.(taskId)
    }

    void createTask()
    return () => {
      cancelled = true
    }
  }, [onTaskCreated, store, validatedLaunch])

  if (launchState.status === 'loading') {
    return <LaunchStateCard onClose={onClose} />
  }
  if (launchState.status === 'error') {
    return <LaunchStateCard error={launchState.code} onClose={onClose} />
  }

  const task = store.getTask(launchState.taskId)

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
      <InstallerFrame onClose={onBackground}>
        <ResultStep task={task} onClose={onClose} onViewApp={() => onViewApp(task.app.id)} />
      </InstallerFrame>
    )
  }

  if (task.status === 'failed') {
    return (
      <InstallerFrame onClose={onBackground}>
        <FailureStep task={task} onChangeSource={onChangeSource} />
      </InstallerFrame>
    )
  }

  if (task.status === 'running') {
    return (
      <InstallerFrame onClose={onBackground}>
        <ProgressStep task={task} onBackground={onBackground} />
      </InstallerFrame>
    )
  }

  if (approvalOpen) {
    return (
      <InstallerFrame onClose={onBackground}>
        <ApprovalStep task={task} onBack={() => setApprovalOpen(false)} />
      </InstallerFrame>
    )
  }

  return (
    <InstallerFrame onClose={onBackground}>
      <VerifyStep task={task} onBack={onChangeSource} onContinue={() => setApprovalOpen(true)} onEnd={onClose} />
    </InstallerFrame>
  )
}

export function AppInstallerRoute() {
  const location = useLocation()
  const navigate = useNavigate()
  const parsed = useMemo(
    () => parseAppInstallerLaunchQuery(location.search),
    [location.search],
  )

  const closeStandalone = () => {
    if (window.opener && !window.opener.closed) {
      window.close()
      return
    }
    void navigate('/')
  }

  const normalizeTaskUrl = (taskId: string) => {
    const url = new URL(window.location.href)
    url.search = ''
    url.searchParams.set('task_id', taskId)
    window.history.replaceState(window.history.state, '', `${url.pathname}${url.search}${url.hash}`)
  }

  return (
    <main className="relative z-10 flex min-h-dvh items-center justify-center overflow-y-auto p-3 sm:p-6">
      <div
        className="fixed inset-0 bg-[color:color-mix(in_srgb,var(--cp-shadow)_24%,transparent)] backdrop-blur-[2px]"
        aria-hidden="true"
      />
      <div className="relative z-10 w-full max-w-3xl">
        {parsed.ok ? (
          <AppInstaller
            launchParams={parsed.params}
            onBackground={closeStandalone}
            onChangeSource={closeStandalone}
            onClose={closeStandalone}
            onTaskCreated={normalizeTaskUrl}
            onViewApp={closeStandalone}
          />
        ) : (
          <LaunchStateCard error={parsed.code} onClose={closeStandalone} />
        )}
      </div>
    </main>
  )
}
