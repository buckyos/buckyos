import { IconButton, Menu, MenuItem } from '@mui/material'
import {
  ChevronLeft,
  Ellipsis,
  Minimize2,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { useI18n } from '../i18n/provider'
import { useAppText } from '../presentation/appText'
import type { AppDefinition, LayoutState, ThemeMode } from '../models/ui'
import { useMobileNavState } from './windows/MobileNavContext'
import { UserAvatar } from './TaskbarParts'
import type { CurrentUser } from './taskbarHooks'
import {
  connectionTone,
  mobileStatusBarHeight,
  mobileStatusBarMode,
  type ConnectionState,
  useMinuteClock,
} from './shell'
import { MessageTray, NotificationTray } from './tray/TrayButtons'
import type { TraySnapshot } from './tray/trayModel'

function StatusLogoButton({
  connectionState,
  highlightBorder = false,
  onClick,
  user,
}: {
  connectionState: ConnectionState
  highlightBorder?: boolean
  onClick: () => void
  user: CurrentUser | null
}) {
  return (
    <button
      type="button"
      aria-label="BuckyOS"
      onClick={onClick}
      className="pointer-events-auto inline-flex h-9 w-9 items-center justify-center overflow-hidden rounded-full border bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] text-[color:var(--cp-text)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)] transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]"
      style={{
        borderColor: highlightBorder
          ? `color-mix(in srgb, ${connectionTone(connectionState)} 76%, var(--cp-border))`
          : 'color-mix(in srgb, var(--cp-border) 82%, transparent)',
      }}
    >
      <UserAvatar size={36} user={user} />
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
  locale,
  now,
  onNavigate,
  tray,
}: {
  locale: string
  now: Date
  onNavigate: (path: string) => void
  tray: TraySnapshot
}) {
  const timeLabel = useMemo(
    () => new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(now),
    [locale, now],
  )

  return (
    <div className="shell-pill ml-auto shrink-0 gap-1.5 px-2 py-1 text-xs">
      <MessageTray onNavigate={onNavigate} snapshot={tray.messages} />
      <NotificationTray onNavigate={onNavigate} snapshot={tray} />
      <span className="pl-0.5 pr-1 font-medium text-[color:var(--cp-text)]">{timeLabel}</span>
    </div>
  )
}

/** The mobile shell's status bar; the desktop shell uses `DesktopTaskbar`. */
export function StatusBar({
  activeApp,
  connectionState,
  deadZone,
  onCycleLocale,
  onMinimizeWindow,
  onNavigate,
  onOpenDiagnostics,
  onOpenSettings,
  onOpenSidebar,
  onToggleTheme,
  safeAreaTop = 0,
  themeMode,
  tray,
  user,
}: {
  activeApp?: AppDefinition
  connectionState: ConnectionState
  deadZone: LayoutState['deadZone']
  onCycleLocale: () => void
  onMinimizeWindow?: () => void
  /** Opens a page of the desktop (the trays' MessageHub / Task Center links). */
  onNavigate: (path: string) => void
  onOpenDiagnostics: () => void
  onOpenSettings: () => void
  onOpenSidebar: () => void
  onToggleTheme: () => void
  safeAreaTop?: number
  themeMode: ThemeMode
  tray: TraySnapshot
  user: CurrentUser | null
}) {
  const { locale, t } = useI18n()
  const appText = useAppText()
  const now = useMinuteClock()
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const mobileNav = useMobileNavState()
  const activeMode = activeApp ? mobileStatusBarMode(activeApp) : null
  const barHeight = mobileStatusBarHeight(activeApp)
  const totalHeight = safeAreaTop + deadZone.top + barHeight
  const showSurface = activeMode === 'standard'

  return (
    <div
      aria-label={t('common.statusBar')}
      className="pointer-events-none fixed inset-x-0 top-0 z-50"
      style={{ height: totalHeight }}
    >
      {showSurface && activeApp ? (
        <div
          className="absolute inset-x-0 top-0 backdrop-blur-xl"
          style={{
            height: totalHeight,
            backgroundColor: `color-mix(in srgb, ${activeApp.accent} 14%, var(--cp-surface-2))`,
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
                <StatusLogoButton connectionState={connectionState} onClick={onOpenSidebar} user={user} />
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
                {mobileNav.titleOverride?.title ?? appText(activeApp).title}
              </p>
              <p className="line-clamp-1 text-xs text-[color:var(--cp-muted)]">
                {mobileNav.titleOverride?.subtitle ?? appText(activeApp).summary}
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
                  highlightBorder
                  onClick={onOpenSidebar}
                  user={user}
                />
              )}
              {activeMode === 'compact' && activeApp ? (
                <div className="min-w-0">
                  <p className="truncate font-display text-sm font-semibold text-[color:var(--cp-text)]">
                    {appText(activeApp).title}
                  </p>
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
                locale={locale}
                now={now}
                onNavigate={onNavigate}
                tray={tray}
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
