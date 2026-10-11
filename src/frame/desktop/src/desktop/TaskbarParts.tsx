import { Divider, ListItemIcon, Menu, MenuItem } from '@mui/material'
import clsx from 'clsx'
import {
  CircleCheck,
  LogOut,
  OctagonAlert,
  Pin,
  PinOff,
  ShieldAlert,
  ShieldCheck,
  ShieldX,
  TriangleAlert,
  UserRound,
  X,
} from 'lucide-react'
import { useState, type CSSProperties, type ReactNode, type Ref } from 'react'
import type { SystemMetrics } from '../api/system_metrics'
import type { DesktopAppItem } from '../app/types'
import { AppIcon } from '../components/DesktopVisuals'
import { appIconSurfaceStyle } from '../components/DesktopVisualTokens'
import { useI18n } from '../i18n/provider'
import type { TaskbarAppItem } from '../models/ui'
import { connectionLabel, type ConnectionState } from './shell'
import type { CurrentUser, SystemStatusState, useHoverCard } from './taskbarHooks'

type TranslateFn = ReturnType<typeof useI18n>['t']

// ---------------------------------------------------------------------------
// App buttons and their menu (desktop taskbar and mobile rail)
// ---------------------------------------------------------------------------

/** An app's icon tile with the running (and front-most) mark at its left edge. */
export function TaskbarAppTile({
  indicatorOffset,
  item,
  size,
}: {
  /** How far left of the tile the running mark sits. */
  indicatorOffset: number
  item: TaskbarAppItem
  size: number
}) {
  return (
    <>
      {item.windowCount > 0 ? (
        <span
          data-testid={`taskbar-app-running-${item.appId}`}
          className={clsx(
            'absolute top-1/2 w-[3px] -translate-y-1/2 rounded-full transition-[height] duration-150',
            item.active ? 'h-4 bg-[color:var(--cp-accent)]' : 'h-1.5 bg-[color:var(--cp-muted)]',
          )}
          style={{ left: -indicatorOffset }}
        />
      ) : null}
      <span
        className="flex items-center justify-center overflow-hidden rounded-[10px]"
        style={{
          width: size,
          height: size,
          '--icon-size': `${size}px`,
          ...appIconSurfaceStyle(item.accent),
        } as CSSProperties}
      >
        <AppIcon iconKey={item.iconKey} iconUrl={item.iconUrl} fill className="text-white" />
      </span>
    </>
  )
}

export type TaskbarMenuState = { appId?: string; x: number; y: number }

/**
 * The taskbar's context menu. For an app: open it, pin or unpin it, close
 * its windows. Without an app it shows `children` (the taskbar's own items).
 */
export function TaskbarMenu({
  app,
  children,
  menu,
  menuRef,
  onClose,
  onCloseWindows,
  onOpenApp,
  onSetPinned,
  pinned,
  windowCount,
}: {
  app?: DesktopAppItem
  children?: ReactNode
  menu: TaskbarMenuState | null
  menuRef?: Ref<HTMLDivElement>
  onClose: () => void
  onCloseWindows: (appId: string) => void
  onOpenApp: (appId: string) => void
  onSetPinned: (appId: string, pinned: boolean) => void
  pinned: boolean
  windowCount: number
}) {
  const { t } = useI18n()
  return (
    <Menu
      ref={menuRef}
      open={Boolean(menu)}
      onClose={onClose}
      anchorReference="anchorPosition"
      anchorPosition={menu ? { top: menu.y, left: menu.x } : undefined}
      // The page's own context menu must not open over this one.
      onContextMenu={(event) => event.preventDefault()}
    >
      {app
        ? [
            <MenuItem key="open" onClick={() => onOpenApp(app.id)}>
              <ListItemIcon>
                <span
                  className="flex size-5 items-center justify-center overflow-hidden rounded-[6px]"
                  style={{ '--icon-size': '20px', ...appIconSurfaceStyle(app.accent) } as CSSProperties}
                >
                  <AppIcon iconKey={app.iconKey} iconUrl={app.iconUrl} fill className="text-white" />
                </span>
              </ListItemIcon>
              {t(app.labelKey)}
            </MenuItem>,
            <Divider key="divider" />,
            <MenuItem
              key="pin"
              data-testid="taskbar-menu-pin-app"
              onClick={() => {
                onClose()
                onSetPinned(app.id, !pinned)
              }}
            >
              <ListItemIcon>{pinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}</ListItemIcon>
              {pinned ? t('shell.unpinFromTaskbar', 'Unpin from taskbar') : t('shell.pinToTaskbar', 'Pin to taskbar')}
            </MenuItem>,
            windowCount > 0 ? (
              <MenuItem
                key="close"
                onClick={() => {
                  onClose()
                  onCloseWindows(app.id)
                }}
              >
                <ListItemIcon>
                  <X className="size-4" />
                </ListItemIcon>
                {windowCount > 1 ? t('shell.closeAllWindows', 'Close all windows') : t('shell.closeWindow', 'Close window')}
              </MenuItem>
            ) : null,
          ]
        : children}
    </Menu>
  )
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

export const taskbarPopupSurface =
  'border border-[color:var(--cp-border)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface)_96%,transparent),color-mix(in_srgb,var(--cp-surface-2)_97%,transparent))] shadow-[var(--cp-panel-shadow)] backdrop-blur-xl'

function TaskbarCard({
  cardRef,
  children,
  className,
  hoverProps,
  label,
  style,
  testId,
}: {
  cardRef: Ref<HTMLDivElement>
  children: ReactNode
  className?: string
  hoverProps: ReturnType<typeof useHoverCard>['hoverProps']
  label: string
  style: CSSProperties
  testId: string
}) {
  return (
    <div
      ref={cardRef}
      role="dialog"
      aria-label={label}
      data-testid={testId}
      {...hoverProps}
      className={clsx(
        'absolute z-[57] rounded-[18px] text-[color:var(--cp-text)] [animation:cp-taskbar-pop_160ms_var(--cp-ease-emphasis)]',
        taskbarPopupSurface,
        className,
      )}
      style={style}
    >
      {children}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Current user
// ---------------------------------------------------------------------------

export function UserAvatar({ size, user }: { size: number; user: CurrentUser | null }) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null)
  if (user?.avatarUrl && failedUrl !== user.avatarUrl) {
    return (
      <img
        src={user.avatarUrl}
        alt=""
        draggable={false}
        className="size-full rounded-full object-cover"
        onError={() => setFailedUrl(user.avatarUrl ?? null)}
      />
    )
  }
  const initial = user ? Array.from(user.displayName)[0]?.toUpperCase() : undefined
  return (
    <span
      aria-hidden
      className="font-display font-semibold tracking-[-0.04em]"
      style={{ fontSize: Math.round(size * 0.46) }}
    >
      {initial ?? 'B'}
    </span>
  )
}

export function ProfileCard({
  cardRef,
  connectionState,
  hoverProps,
  isLoggingOut,
  onLogout,
  onOpenAccount,
  runtimeContainer,
  style,
  user,
}: {
  cardRef: Ref<HTMLDivElement>
  connectionState: ConnectionState
  hoverProps: ReturnType<typeof useHoverCard>['hoverProps']
  isLoggingOut: boolean
  onLogout: () => void
  /** Opens where the account is managed; absent when that app is not installed. */
  onOpenAccount?: () => void
  runtimeContainer: string
  style: CSSProperties
  user: CurrentUser | null
}) {
  const { t } = useI18n()
  const ConnectionIcon =
    connectionState === 'online' ? ShieldCheck : connectionState === 'degraded' ? ShieldAlert : ShieldX
  const rowClassName = 'flex min-w-0 items-baseline gap-3 text-[12px] leading-5'
  const rowLabelClassName = 'w-14 shrink-0 text-[color:var(--cp-muted)]'

  return (
    <TaskbarCard
      cardRef={cardRef}
      hoverProps={hoverProps}
      label={t('shell.profile', 'Account')}
      style={{ width: 288, ...style }}
      testId="taskbar-profile-card"
    >
      <div className="flex items-center gap-3 px-4 pb-3 pt-4">
        <span className="inline-flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,var(--cp-surface))]">
          <UserAvatar size={48} user={user} />
        </span>
        <div className="min-w-0">
          <p className="truncate font-display text-[15px] font-semibold">
            {user?.displayName ?? t('shell.profileLoading', 'Loading account...')}
          </p>
          {user ? (
            <p className="truncate text-[12px] text-[color:var(--cp-muted)]">
              {user.userType ? `${user.userId} · ${user.userType}` : user.userId}
            </p>
          ) : null}
        </div>
      </div>
      <div className="space-y-1 border-t border-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)] px-4 py-3">
        {user?.did ? (
          <div className={rowClassName}>
            <span className={rowLabelClassName}>DID</span>
            <span className="min-w-0 truncate font-mono text-[11px]" title={user.did}>
              {user.did}
            </span>
          </div>
        ) : null}
        <div className={rowClassName}>
          <span className={rowLabelClassName}>{t('shell.session', 'Session')}</span>
          <span className="flex min-w-0 items-center gap-1.5">
            <ConnectionIcon className="size-3.5 shrink-0 self-center text-[color:var(--cp-muted)]" />
            <span className="truncate">
              {t(`runtime.${runtimeContainer}`, runtimeContainer)} · {connectionLabel(connectionState, t)}
            </span>
          </span>
        </div>
      </div>
      <div className="flex items-center gap-2 border-t border-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)] px-3 py-3">
        {onOpenAccount ? (
          <button
            type="button"
            onClick={onOpenAccount}
            className="inline-flex h-9 min-w-0 flex-1 items-center justify-center gap-2 rounded-[12px] px-3 text-[13px] font-medium text-[color:var(--cp-text)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,transparent)]"
          >
            <UserRound className="size-4 shrink-0 text-[color:var(--cp-muted)]" />
            <span className="truncate">{t('shell.manageAccount', 'Manage account')}</span>
          </button>
        ) : (
          <span className="flex-1" />
        )}
        <button
          type="button"
          aria-disabled={isLoggingOut || undefined}
          onClick={isLoggingOut ? undefined : onLogout}
          className={clsx(
            'inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-[12px] px-3 text-[13px] font-medium text-[color:var(--cp-danger)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-danger)_10%,transparent)]',
            isLoggingOut ? 'pointer-events-none opacity-55' : '',
          )}
        >
          <LogOut className="size-4 shrink-0" />
          {isLoggingOut ? t('shell.loggingOut', 'Logging out...') : t('shell.logout', 'Log out')}
        </button>
      </div>
    </TaskbarCard>
  )
}

// ---------------------------------------------------------------------------
// System status
// ---------------------------------------------------------------------------

type StatusLevel = 'normal' | 'warning' | 'critical'

function usageLevel(percent: number): StatusLevel {
  if (percent >= 95) return 'critical'
  if (percent >= 85) return 'warning'
  return 'normal'
}

function worstLevel(metrics: SystemMetrics): StatusLevel {
  const levels = [metrics.cpu.usagePercent, metrics.memory.usagePercent, metrics.disk.usagePercent].map(usageLevel)
  if (levels.includes('critical')) return 'critical'
  if (levels.includes('warning')) return 'warning'
  return 'normal'
}

const levelColor: Record<StatusLevel, string> = {
  normal: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  critical: 'var(--cp-danger)',
}

function formatPercent(value: number) {
  return `${Math.round(value)}%`
}

function formatGb(value: number, locale: string) {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: value < 100 ? 1 : 0 }).format(value)
}

function formatRate(bytesPerSec: number, locale: string) {
  const units = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  let value = Math.max(0, bytesPerSec)
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: value < 10 && unit > 0 ? 1 : 0 }).format(value)} ${units[unit]}`
}

function formatUptime(seconds: number, t: TranslateFn) {
  const days = Math.floor(seconds / 86_400)
  const hours = Math.floor((seconds % 86_400) / 3_600)
  const minutes = Math.floor((seconds % 3_600) / 60)
  if (days > 0) return t('shell.status.uptimeDays', '{{days}}d {{hours}}h', { days, hours })
  if (hours > 0) return t('shell.status.uptimeHours', '{{hours}}h {{minutes}}m', { hours, minutes })
  return t('shell.status.uptimeMinutes', '{{minutes}}m', { minutes })
}

/** The taskbar's two status lines: CPU and memory, or the connection when there is no reading. */
export function SystemStatusLines({
  connectionState,
  status,
}: {
  connectionState: ConnectionState
  status: SystemStatusState
}) {
  const { t } = useI18n()
  const { metrics } = status
  const lines: Array<{ label: string; value: string; level: StatusLevel }> = metrics
    ? [
        { label: t('shell.status.cpu', 'CPU'), value: formatPercent(metrics.cpu.usagePercent), level: usageLevel(metrics.cpu.usagePercent) },
        { label: t('shell.status.memoryShort', 'MEM'), value: formatPercent(metrics.memory.usagePercent), level: usageLevel(metrics.memory.usagePercent) },
      ]
    : status.failed
      ? [
          { label: connectionLabel(connectionState, t), value: '', level: 'normal' },
          { label: t('shell.status.noData', 'No data'), value: '', level: 'normal' },
        ]
      : [
          { label: t('shell.status.cpu', 'CPU'), value: '--', level: 'normal' },
          { label: t('shell.status.memoryShort', 'MEM'), value: '--', level: 'normal' },
        ]

  return (
    <span className="flex w-full flex-col items-center text-[10px] leading-[13px] tabular-nums">
      {lines.map((line) => (
        <span key={line.label} className="flex max-w-full items-center gap-[3px] whitespace-nowrap">
          {line.level !== 'normal' ? (
            <span className="size-[5px] shrink-0 rounded-full" style={{ background: levelColor[line.level] }} />
          ) : null}
          <span className="truncate text-[color:var(--cp-muted)]">{line.label}</span>
          {line.value ? <span className="font-medium text-[color:var(--cp-text)]">{line.value}</span> : null}
        </span>
      ))}
    </span>
  )
}

function UsageRow({
  detail,
  label,
  percent,
}: {
  detail: string
  label: string
  percent: number
}) {
  const level = usageLevel(percent)
  return (
    <div className="space-y-1">
      <div className="flex items-baseline justify-between gap-3 text-[12px]">
        <span className="text-[color:var(--cp-muted)]">{label}</span>
        <span className="font-medium tabular-nums">{formatPercent(percent)}</span>
      </div>
      <div className="h-1 overflow-hidden rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]">
        <div
          className="h-full rounded-full transition-[width] duration-300"
          style={{
            width: `${Math.min(100, Math.max(2, percent))}%`,
            background: level === 'normal' ? 'var(--cp-accent)' : levelColor[level],
          }}
        />
      </div>
      <p className="truncate text-[11px] text-[color:var(--cp-muted)]">{detail}</p>
    </div>
  )
}

export function SystemStatusCard({
  cardRef,
  connectionState,
  hoverProps,
  runtimeContainer,
  status,
  style,
}: {
  cardRef: Ref<HTMLDivElement>
  connectionState: ConnectionState
  hoverProps: ReturnType<typeof useHoverCard>['hoverProps']
  runtimeContainer: string
  status: SystemStatusState
  style: CSSProperties
}) {
  const { locale, t } = useI18n()
  const { metrics } = status
  const level = metrics ? worstLevel(metrics) : null
  const LevelIcon = level === 'critical' ? OctagonAlert : level === 'warning' ? TriangleAlert : CircleCheck
  const levelLabel =
    level === 'critical'
      ? t('shell.status.critical', 'Critical')
      : level === 'warning'
        ? t('shell.status.warning', 'Under load')
        : t('shell.status.normal', 'Normal')
  const rowClassName = 'flex items-baseline justify-between gap-3 text-[12px] leading-5'

  return (
    <TaskbarCard
      cardRef={cardRef}
      hoverProps={hoverProps}
      label={t('shell.status.title', 'System status')}
      style={{ width: 296, ...style }}
      testId="taskbar-status-card"
    >
      <div className="flex items-center justify-between gap-3 px-4 pb-2 pt-4">
        <p className="font-display text-[15px] font-semibold">{t('shell.status.title', 'System status')}</p>
        {level ? (
          <span className="inline-flex items-center gap-1.5 text-[12px] font-medium">
            <LevelIcon className="size-3.5" style={{ color: levelColor[level] }} />
            {levelLabel}
          </span>
        ) : null}
      </div>
      {metrics ? (
        <div className="space-y-3 px-4 pb-3 pt-1">
          <UsageRow
            label={t('shell.status.cpu', 'CPU')}
            percent={metrics.cpu.usagePercent}
            detail={t('shell.status.cpuDetail', '{{model}} · {{cores}} cores', {
              model: metrics.cpu.model,
              cores: metrics.cpu.cores,
            })}
          />
          <UsageRow
            label={t('shell.status.memory', 'Memory')}
            percent={metrics.memory.usagePercent}
            detail={`${formatGb(metrics.memory.usedGb, locale)} / ${formatGb(metrics.memory.totalGb, locale)} GB`}
          />
          <UsageRow
            label={t('shell.status.storage', 'Storage')}
            percent={metrics.disk.usagePercent}
            detail={`${formatGb(metrics.disk.usedGb, locale)} / ${formatGb(metrics.disk.totalGb, locale)} GB`}
          />
        </div>
      ) : (
        <p className="px-4 pb-3 text-[12px] text-[color:var(--cp-muted)]">
          {status.failed
            ? t('shell.status.unavailable', 'System status is unavailable right now.')
            : t('shell.status.loading', 'Reading system status...')}
        </p>
      )}
      <div className="space-y-0.5 border-t border-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)] px-4 py-3">
        {metrics ? (
          <>
            <div className={rowClassName}>
              <span className="text-[color:var(--cp-muted)]">{t('shell.status.network', 'Network')}</span>
              <span className="tabular-nums">
                ↓ {formatRate(metrics.network.rxPerSec, locale)} · ↑ {formatRate(metrics.network.txPerSec, locale)}
              </span>
            </div>
            <div className={rowClassName}>
              <span className="text-[color:var(--cp-muted)]">{t('shell.status.load', 'Load average')}</span>
              <span className="tabular-nums">
                {[metrics.loadAverage.one, metrics.loadAverage.five, metrics.loadAverage.fifteen]
                  .map((value) => value.toFixed(2))
                  .join(' · ')}
              </span>
            </div>
            <div className={rowClassName}>
              <span className="text-[color:var(--cp-muted)]">{t('shell.status.processes', 'Processes')}</span>
              <span className="tabular-nums">{metrics.processCount}</span>
            </div>
            <div className={rowClassName}>
              <span className="text-[color:var(--cp-muted)]">{t('shell.status.uptime', 'Uptime')}</span>
              <span className="tabular-nums">{formatUptime(metrics.uptimeSeconds, t)}</span>
            </div>
          </>
        ) : null}
        <div className={rowClassName}>
          <span className="text-[color:var(--cp-muted)]">{t('shell.session', 'Session')}</span>
          <span>
            {t(`runtime.${runtimeContainer}`, runtimeContainer)} · {connectionLabel(connectionState, t)}
          </span>
        </div>
      </div>
    </TaskbarCard>
  )
}
