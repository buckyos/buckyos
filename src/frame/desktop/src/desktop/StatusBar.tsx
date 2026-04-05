import { IconButton, Menu, MenuItem } from '@mui/material'
import clsx from 'clsx'
import {
  AlertTriangle,
  BellDot,
  CheckCheck,
  ChevronLeft,
  Ellipsis,
  HardDriveDownload,
  LoaderCircle,
  MessageCircle,
  Minimize2,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../i18n/provider'
import type { AppDefinition, FormFactor, LayoutState, ThemeMode } from '../models/ui'
import { useMobileNavState } from './windows/MobileNavContext'
import {
  connectionLabel,
  connectionTone,
  mobileStatusBarMode,
  shellStatusBarHeight,
  type ConnectionState,
  type StatusTip,
  type StatusTipTone,
  type StatusTrayState,
  useMinuteClock,
} from './shell'

function StatusLogoButton({
  connectionState,
  highlightBorder = false,
  onClick,
}: {
  connectionState: ConnectionState
  highlightBorder?: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      aria-label="BuckyOS"
      onClick={onClick}
      className="pointer-events-auto inline-flex h-9 w-9 items-center justify-center rounded-full border bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] font-display text-sm font-semibold tracking-[-0.04em] text-[color:var(--cp-text)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)] transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]"
      style={{
        borderColor: highlightBorder
          ? `color-mix(in srgb, ${connectionTone(connectionState)} 76%, var(--cp-border))`
          : 'color-mix(in srgb, var(--cp-border) 82%, transparent)',
      }}
    >
      B
    </button>
  )
}

function MobileBackButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      aria-label="Back"
      onClick={onClick}
      className="pointer-events-auto inline-flex h-9 w-9 items-center justify-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] text-[color:var(--cp-text)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)] transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]"
    >
      <ChevronLeft size={20} />
    </button>
  )
}

function StatusTray({
  compact = false,
  locale,
  now,
  panelOffsetTop,
  trayState,
}: {
  compact?: boolean
  locale: string
  now: Date
  panelOffsetTop: number
  trayState: StatusTrayState
}) {
  const { t } = useI18n()
  const timeLabel = new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
  }).format(now)
  const [isTipsOpen, setIsTipsOpen] = useState(false)
  const [arrowOffset, setArrowOffset] = useState<number | null>(null)
  const tipsRef = useRef<HTMLDivElement | null>(null)
  const tipsPanelRef = useRef<HTMLDivElement | null>(null)
  const tipsButtonRef = useRef<HTMLButtonElement | null>(null)
  const isMobile = compact

  useEffect(() => {
    if (!isTipsOpen) {
      return
    }

    const frameId = window.requestAnimationFrame(() => {
      tipsPanelRef.current?.focus()
    })

    const handlePointerDown = (event: PointerEvent) => {
      if (!tipsRef.current?.contains(event.target as Node)) {
        setIsTipsOpen(false)
      }
    }

    const handleFocusIn = (event: FocusEvent) => {
      if (!tipsRef.current?.contains(event.target as Node)) {
        setIsTipsOpen(false)
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setIsTipsOpen(false)
        tipsButtonRef.current?.focus()
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('focusin', handleFocusIn)
    window.addEventListener('keydown', handleKeyDown)

    return () => {
      window.cancelAnimationFrame(frameId)
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('focusin', handleFocusIn)
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [isTipsOpen])

  useEffect(() => {
    if (!isTipsOpen) {
      setArrowOffset(null)
      return
    }

    const syncArrowOffset = () => {
      const panelRect = tipsPanelRef.current?.getBoundingClientRect()
      const buttonRect = tipsButtonRef.current?.getBoundingClientRect()

      if (!panelRect || !buttonRect) {
        return
      }

      const buttonCenter = buttonRect.left + buttonRect.width / 2
      const nextOffset = buttonCenter - panelRect.left
      const clampedOffset = Math.max(24, Math.min(nextOffset, panelRect.width - 24))
      setArrowOffset(clampedOffset)
    }

    const frameId = window.requestAnimationFrame(syncArrowOffset)
    window.addEventListener('resize', syncArrowOffset)

    return () => {
      window.cancelAnimationFrame(frameId)
      window.removeEventListener('resize', syncArrowOffset)
    }
  }, [isTipsOpen])

  return (
    <div
      className={clsx(
        'shell-pill ml-auto shrink-0 px-3 py-1.5 text-xs',
        compact ? 'gap-2 px-2.5 py-1.5' : '',
      )}
    >
      {trayState.backupActive ? (
        <span className="inline-flex items-center gap-1.5 rounded-full bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,var(--cp-surface))] px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-[color:var(--cp-warning)]">
          <HardDriveDownload className="size-3.5" />
          <span className="hidden sm:inline">Backup</span>
        </span>
      ) : null}
      <span className="relative inline-flex items-center justify-center text-[color:var(--cp-text)]">
        <MessageCircle className="size-4" />
        {trayState.messageCount > 0 ? (
          <span className="absolute -right-1.5 -top-1.5 inline-flex min-h-4 min-w-4 items-center justify-center rounded-full bg-[color:var(--cp-accent)] px-1 text-[9px] font-semibold text-white">
            {trayState.messageCount}
          </span>
        ) : null}
      </span>
      <div
        ref={tipsRef}
        className="pointer-events-none inline-flex items-center justify-center"
      >
        <button
          ref={tipsButtonRef}
          type="button"
          aria-label={t('shell.desktopTips')}
          aria-controls="status-tips-panel"
          aria-expanded={isTipsOpen}
          aria-haspopup="dialog"
          data-testid="status-tray-tips-button"
          onClick={() => setIsTipsOpen((prev) => !prev)}
          className={clsx(
            'pointer-events-auto relative inline-flex size-7 items-center justify-center rounded-full text-[color:var(--cp-text)] transition-colors duration-150 ease-[var(--cp-ease-emphasis)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]',
            isTipsOpen
              ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,var(--cp-surface))]'
              : 'hover:bg-[color:color-mix(in_srgb,var(--cp-surface-2)_92%,transparent)]',
          )}
        >
          <BellDot className="size-4" />
          {trayState.notificationCount > 0 ? (
            <span className="absolute -right-1.5 -top-1.5 inline-flex min-h-4 min-w-4 items-center justify-center rounded-full bg-[color:var(--cp-danger)] px-1 text-[9px] font-semibold text-white">
              {trayState.notificationCount}
            </span>
          ) : null}
        </button>
        {isTipsOpen ? (
          <div
            id="status-tips-panel"
            ref={tipsPanelRef}
            role="dialog"
            aria-label={t('shell.desktopTips')}
            data-testid="status-tips-panel"
            tabIndex={-1}
            className="pointer-events-auto absolute z-[90] outline-none"
            style={{
              top: 'calc(100% + 8px)',
              right: isMobile
                ? 'calc(env(safe-area-inset-right, 0px) + 16px)'
                : 'calc(env(safe-area-inset-right, 0px) + 20px)',
              left: undefined,
              width: isMobile
                ? 'min(18.75rem, calc(100vw - env(safe-area-inset-left, 0px) - env(safe-area-inset-right, 0px) - 32px))'
                : '18.5rem',
              maxWidth: isMobile ? undefined : '18.5rem',
              maxHeight: `min(50dvh, calc(100dvh - ${panelOffsetTop + 24}px))`,
            }}
          >
            <div
              className="absolute -top-[9px] h-[18px] w-[18px] -translate-x-1/2 rotate-45 border-l border-t border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_96%,transparent)]"
              style={{ left: arrowOffset ?? 24 }}
            />
            <div
              className="shell-panel overflow-hidden rounded-[20px] shadow-[0_20px_52px_color-mix(in_srgb,var(--cp-shadow)_18%,transparent)]"
              style={{ maxHeight: 'inherit' }}
            >
              <div
                className="shell-scrollbar overflow-y-auto px-3 py-3"
                style={{ maxHeight: 'inherit' }}
              >
                <div className="space-y-1">
                {trayState.tips.map((tip) => (
                  <StatusTipCard key={tip.id} tip={tip} />
                ))}
                </div>
              </div>
            </div>
          </div>
        ) : null}
      </div>
      <span className="font-medium text-[color:var(--cp-text)]">{timeLabel}</span>
    </div>
  )
}

function StatusTipCard({ tip }: { tip: StatusTip }) {
  const toneStyles = statusTipToneStyles(tip.tone)
  const Icon = statusTipToneIcon(tip.tone)

  return (
    <article
      data-testid={`status-tip-card-${tip.id}`}
      className="rounded-[14px] px-2.5 py-2.5"
    >
      <div className="flex items-start gap-2.5">
        <div
          className="mt-0.5 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full"
          style={{
            backgroundColor: toneStyles.iconSurface,
            color: toneStyles.iconColor,
          }}
        >
          <Icon className={clsx('size-4', tip.tone === 'progress' ? 'animate-spin' : '')} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-3">
            <p className="min-w-0 truncate pr-2 font-display text-[13px] font-semibold text-[color:var(--cp-text)]">
              {tip.title}
            </p>
            <span className="w-[3.25rem] shrink-0 pt-0.5 text-right text-[10px] font-normal leading-4 text-[color:var(--cp-muted)]">
              {tip.timeLabel}
            </span>
          </div>
          <p className="mt-1 text-[12px] leading-5 text-[color:var(--cp-muted)]">
            {tip.body}
          </p>
        </div>
      </div>
    </article>
  )
}

function statusTipToneIcon(tone: StatusTipTone) {
  if (tone === 'success') {
    return CheckCheck
  }

  if (tone === 'error') {
    return AlertTriangle
  }

  return LoaderCircle
}

function statusTipToneStyles(tone: StatusTipTone) {
  if (tone === 'success') {
    return {
      iconSurface: 'color-mix(in srgb, var(--cp-success) 14%, var(--cp-surface))',
      iconColor: 'var(--cp-success)',
    }
  }

  if (tone === 'error') {
    return {
      iconSurface: 'color-mix(in srgb, var(--cp-danger) 14%, var(--cp-surface))',
      iconColor: 'var(--cp-danger)',
    }
  }

  return {
    iconSurface: 'color-mix(in srgb, var(--cp-warning) 14%, var(--cp-surface))',
    iconColor: 'var(--cp-warning)',
  }
}

export function StatusBar({
  activeApp,
  connectionState,
  deadZone,
  formFactor,
  onCycleLocale,
  onMinimizeWindow,
  onOpenDiagnostics,
  onOpenSettings,
  onOpenSidebar,
  onToggleTheme,
  safeAreaTop = 0,
  themeMode,
  trayState,
}: {
  activeApp?: AppDefinition
  connectionState: ConnectionState
  deadZone: LayoutState['deadZone']
  formFactor: FormFactor
  onCycleLocale: () => void
  onMinimizeWindow?: () => void
  onOpenDiagnostics: () => void
  onOpenSettings: () => void
  onOpenSidebar: () => void
  onToggleTheme: () => void
  safeAreaTop?: number
  themeMode: ThemeMode
  trayState: StatusTrayState
}) {
  const { locale, t } = useI18n()
  const now = useMinuteClock()
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const mobileNav = useMobileNavState()
  const activeMode =
    formFactor === 'mobile' && activeApp ? mobileStatusBarMode(activeApp) : null
  const barHeight = shellStatusBarHeight(formFactor, activeApp)
  const totalHeight = safeAreaTop + deadZone.top + barHeight
  const isDesktop = formFactor === 'desktop'
  const isMobile = !isDesktop
  const showSurface = isDesktop || activeMode === 'standard'
  const connectionText = connectionLabel(connectionState, t)
  const surfaceStyle =
    activeMode === 'standard' && activeApp
      ? {
          backgroundColor: `color-mix(in srgb, ${activeApp.accent} 14%, var(--cp-surface-2))`,
        }
      : {
          background:
            'linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface)_94%,transparent),color-mix(in_srgb,var(--cp-surface)_72%,transparent))',
        }

  return (
    <div
      aria-label={t('common.statusBar')}
      className={clsx(
        'pointer-events-none inset-x-0 top-0 z-50',
        isMobile ? 'fixed' : 'absolute',
      )}
      style={{ height: totalHeight }}
    >
      {showSurface ? (
        <div
          className="absolute inset-x-0 top-0 backdrop-blur-xl"
          style={{
            height: totalHeight,
            ...surfaceStyle,
          }}
        />
      ) : null}
      {showSurface ? (
        <div
          className="absolute inset-x-0 h-px bg-[color:var(--cp-border)]/80"
          style={{ top: totalHeight }}
        />
      ) : null}

      <div
        className="relative flex items-center justify-between gap-3 px-3 text-[color:var(--cp-text)] sm:px-6"
        style={{
          height: totalHeight,
          paddingTop: safeAreaTop + deadZone.top,
        }}
      >
        {activeMode === 'standard' && activeApp ? (
          <>
            <div className="flex min-w-0 items-center gap-2">
              {mobileNav.canGoBack ? (
                <MobileBackButton onClick={mobileNav.goBack} />
              ) : (
                <StatusLogoButton connectionState={connectionState} onClick={onOpenSidebar} />
              )}
              <button
                type="button"
                onClick={onToggleTheme}
                className="pointer-events-auto hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_78%,transparent)] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.18em] text-[color:var(--cp-muted)] sm:inline-flex"
              >
                {t(themeMode === 'light' ? 'common.light' : 'common.dark')}
              </button>
              <button
                type="button"
                onClick={onCycleLocale}
                className="pointer-events-auto hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_78%,transparent)] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.18em] text-[color:var(--cp-muted)] sm:inline-flex"
              >
                {locale}
              </button>
            </div>
            <div className="absolute left-1/2 top-1/2 flex min-w-0 max-w-[46vw] -translate-x-1/2 -translate-y-1/2 flex-col items-center justify-center text-center">
              <p className="truncate font-display text-sm font-semibold text-[color:var(--cp-text)]">
                {t(activeApp.labelKey)}
              </p>
              <p className="line-clamp-1 text-xs text-[color:var(--cp-muted)]">
                {t(activeApp.summaryKey)}
              </p>
            </div>
            <div className="ml-auto flex shrink-0 items-center gap-1">
              <IconButton
                aria-label={t('shell.appMenu', 'App menu')}
                size="small"
                onClick={(event) => setMenuAnchor(event.currentTarget)}
                sx={{ pointerEvents: 'auto' }}
              >
                <Ellipsis className="size-4" />
              </IconButton>
              {onMinimizeWindow ? (
                <IconButton
                  aria-label={t('common.minimize')}
                  size="small"
                  onClick={onMinimizeWindow}
                  sx={{ pointerEvents: 'auto' }}
                >
                  <Minimize2 className="size-4" />
                </IconButton>
              ) : null}
            </div>
          </>
        ) : (
          <>
            <div className="flex min-w-0 items-center gap-2.5">
              {mobileNav.canGoBack ? (
                <MobileBackButton onClick={mobileNav.goBack} />
              ) : (
                <StatusLogoButton
                  connectionState={connectionState}
                  highlightBorder={!isDesktop}
                  onClick={onOpenSidebar}
                />
              )}
              {activeMode === 'compact' && activeApp ? (
                <div className="min-w-0">
                  <p className="truncate font-display text-sm font-semibold text-[color:var(--cp-text)]">
                    {t(activeApp.labelKey)}
                  </p>
                </div>
              ) : null}
              {isDesktop ? (
                <div className="inline-flex min-w-0 items-center gap-2">
                  <span
                    className="h-2.5 w-2.5 rounded-full"
                    style={{ backgroundColor: connectionTone(connectionState) }}
                  />
                  <span className="text-[11px] font-semibold uppercase tracking-[0.22em] text-[color:var(--cp-muted)]">
                    {connectionText}
                  </span>
                </div>
              ) : null}
            </div>
            {activeMode === 'compact' && activeApp ? (
              <div className="ml-auto flex shrink-0 items-center gap-1">
                <IconButton
                  aria-label={t('shell.appMenu', 'App menu')}
                  size="small"
                  onClick={(event) => setMenuAnchor(event.currentTarget)}
                  sx={{ pointerEvents: 'auto' }}
                >
                  <Ellipsis className="size-4" />
                </IconButton>
                {onMinimizeWindow ? (
                  <IconButton
                    aria-label={t('common.minimize')}
                    size="small"
                    onClick={onMinimizeWindow}
                    sx={{ pointerEvents: 'auto' }}
                  >
                    <Minimize2 className="size-4" />
                  </IconButton>
                ) : null}
              </div>
            ) : (
              <StatusTray
                compact={!isDesktop}
                locale={locale}
                now={now}
                panelOffsetTop={totalHeight}
                trayState={trayState}
              />
            )}
          </>
        )}
      </div>

      <Menu
        open={Boolean(menuAnchor)}
        anchorEl={menuAnchor}
        onClose={() => setMenuAnchor(null)}
      >
        <MenuItem
          onClick={() => {
            setMenuAnchor(null)
            onOpenSettings()
          }}
        >
          {t('apps.settings')}
        </MenuItem>
        <MenuItem
          onClick={() => {
            setMenuAnchor(null)
            onOpenDiagnostics()
          }}
        >
          {t('shell.systemInfo', 'System info')}
        </MenuItem>
        {onMinimizeWindow ? (
          <MenuItem
            onClick={() => {
              setMenuAnchor(null)
              onMinimizeWindow()
            }}
          >
            {t('common.minimize')}
          </MenuItem>
        ) : null}
      </Menu>
    </div>
  )
}
