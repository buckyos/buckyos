import { Tooltip } from '@mui/material'
import clsx from 'clsx'
import { House, LogOut, Pin } from 'lucide-react'
import { useEffect, useRef, type CSSProperties, type ReactNode } from 'react'
import { AppIcon } from '../components/DesktopVisuals'
import { appIconSurfaceStyle } from '../components/DesktopVisualTokens'
import { useI18n } from '../i18n/provider'
import type { SystemSidebarAppItem, SystemSidebarDataModel } from '../models/ui'
import {
  connectionLabel,
  desktopAvatarOffset,
  desktopAvatarRowHeight,
  desktopAvatarSize,
  desktopTaskbarWidth,
  type ConnectionState,
} from './shell'

type SafeArea = { top: number; bottom: number; left: number; right: number }

const appTileSize = 36

function TaskbarButton({
  children,
  danger = false,
  disabled = false,
  label,
  onClick,
  pressed,
  selected = false,
  testId,
}: {
  children: ReactNode
  danger?: boolean
  disabled?: boolean
  label: string
  onClick: () => void
  /** Set for toggle buttons; renders the pressed state. */
  pressed?: boolean
  selected?: boolean
  testId?: string
}) {
  return (
    <Tooltip title={label} placement="right" disableInteractive>
      <button
        type="button"
        aria-label={label}
        aria-pressed={pressed}
        // `aria-disabled` rather than `disabled`: a disabled button swallows
        // the events the tooltip listens to.
        aria-disabled={disabled || undefined}
        data-testid={testId}
        onClick={disabled ? undefined : onClick}
        className={clsx(
          'relative flex size-11 shrink-0 items-center justify-center rounded-[12px] border transition-[background-color,border-color,color,transform] duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]',
          selected
            ? 'border-[color:color-mix(in_srgb,var(--cp-accent)_24%,var(--cp-border))] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] text-[color:var(--cp-text)]'
            : pressed
              ? 'border-transparent bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] text-[color:var(--cp-accent)]'
              : danger
                ? 'border-transparent text-[color:var(--cp-danger)] hover:bg-[color:color-mix(in_srgb,var(--cp-danger)_10%,transparent)]'
                : 'border-transparent text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]',
          disabled ? 'pointer-events-none opacity-55' : '',
        )}
      >
        {children}
      </button>
    </Tooltip>
  )
}

function TaskbarDivider() {
  return (
    <div className="my-1 h-px w-7 shrink-0 bg-[color:color-mix(in_srgb,var(--cp-border)_80%,transparent)]" />
  )
}

/**
 * The desktop shell's only chrome: a floating avatar in the top-left corner
 * that slides a narrow taskbar out from the left edge. The taskbar floats
 * over the desktop (no backdrop) and hides on the next outside click unless
 * it is pinned, in which case it stays and takes its width from the window
 * workspace. The avatar never moves; it lands in the taskbar's top slot.
 */
export function DesktopTaskbar({
  connectionState,
  isLoggingOut = false,
  onClose,
  onLogout,
  onOpenApp,
  onReturnDesktop,
  onToggleOpen,
  onTogglePinned,
  open,
  pinned,
  runtimeContainer,
  safeArea,
  uiModel,
}: {
  connectionState: ConnectionState
  isLoggingOut?: boolean
  onClose: () => void
  onLogout: () => void | Promise<void>
  onOpenApp: (appId: string) => void
  onReturnDesktop: () => void
  onToggleOpen: () => void
  onTogglePinned: () => void
  open: boolean
  pinned: boolean
  runtimeContainer: string
  safeArea: SafeArea
  uiModel: SystemSidebarDataModel
}) {
  const { t } = useI18n()
  const rootRef = useRef<HTMLDivElement | null>(null)
  const avatarRef = useRef<HTMLButtonElement | null>(null)
  const visible = open || pinned
  const autoHide = open && !pinned

  useEffect(() => {
    if (!autoHide) return

    // Listening (not a backdrop) keeps the dismissing click working on
    // whatever it lands on.
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) onClose()
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      onClose()
      avatarRef.current?.focus()
    }
    // A click into an app's iframe never reaches this document; it only
    // blurs the window.
    const handleBlur = () => onClose()

    document.addEventListener('pointerdown', handlePointerDown, true)
    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('blur', handleBlur)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true)
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('blur', handleBlur)
    }
  }, [autoHide, onClose])

  const renderApp = (app: SystemSidebarAppItem, running: boolean) => (
    <TaskbarButton
      key={app.appId}
      label={t(app.labelKey)}
      onClick={() => onOpenApp(app.appId)}
      selected={uiModel.currentAppId === app.appId}
      testId={`taskbar-app-${app.appId}`}
    >
      {running ? (
        <span className="absolute left-[-6px] top-1/2 size-1 -translate-y-1/2 rounded-full bg-[color:var(--cp-text)]" />
      ) : null}
      <span
        className="flex items-center justify-center overflow-hidden rounded-[10px]"
        style={{
          width: appTileSize,
          height: appTileSize,
          '--icon-size': `${appTileSize}px`,
          ...appIconSurfaceStyle(app.accent),
        } as CSSProperties}
      >
        <AppIcon iconKey={app.iconKey} iconUrl={app.iconUrl} fill className="text-white" />
      </span>
    </TaskbarButton>
  )

  return (
    <div ref={rootRef} className="contents">
      <aside
        id="desktop-taskbar"
        aria-label={t('shell.taskbar', 'Taskbar')}
        aria-hidden={!visible}
        inert={!visible}
        data-testid="desktop-taskbar"
        data-pinned={pinned}
        className={clsx(
          'absolute inset-y-0 left-0 z-[55] flex flex-col items-center border-r border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface)_96%,transparent),color-mix(in_srgb,var(--cp-surface-2)_94%,transparent))] backdrop-blur-xl transition-[transform,box-shadow] duration-200 ease-[var(--cp-ease-emphasis)]',
          visible ? 'translate-x-0' : '-translate-x-full',
          // No shadow while hidden, or it would peek in at the left edge.
          autoHide ? 'shadow-[0_18px_48px_color-mix(in_srgb,var(--cp-shadow)_18%,transparent)]' : '',
        )}
        style={{
          width: safeArea.left + desktopTaskbarWidth,
          paddingLeft: safeArea.left,
        }}
      >
        {/* The avatar's slot. */}
        <div className="shrink-0" style={{ height: safeArea.top + desktopAvatarRowHeight }} />
        <nav className="desktop-scrollbar flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto overflow-x-hidden pt-2">
          <TaskbarButton label={t('shell.returnDesktop', 'Desktop')} onClick={onReturnDesktop}>
            <House className="size-5" />
          </TaskbarButton>
          {uiModel.switchApps.length > 0 ? (
            <>
              <TaskbarDivider />
              {uiModel.switchApps.map((app) => renderApp(app, true))}
            </>
          ) : null}
          <TaskbarDivider />
          {uiModel.systemApps.map((app) => renderApp(app, false))}
        </nav>
        <div
          className="flex shrink-0 flex-col items-center gap-1 pt-1"
          style={{ paddingBottom: safeArea.bottom + 10 }}
        >
          <TaskbarDivider />
          <TaskbarButton
            label={t('shell.pinTaskbar', 'Keep taskbar visible')}
            onClick={onTogglePinned}
            pressed={pinned}
            testId="taskbar-pin"
          >
            <Pin className="size-5" />
          </TaskbarButton>
          <TaskbarButton
            danger
            disabled={isLoggingOut}
            label={
              isLoggingOut
                ? t('shell.loggingOut', 'Logging out...')
                : t('shell.logout', 'Log out')
            }
            onClick={() => {
              void onLogout()
            }}
          >
            <LogOut className="size-5" />
          </TaskbarButton>
        </div>
      </aside>

      <Tooltip
        title={`${t(`runtime.${runtimeContainer}`, runtimeContainer)} · ${connectionLabel(connectionState, t)}`}
        placement="right"
        disableInteractive
      >
        <button
          ref={avatarRef}
          type="button"
          aria-label="BuckyOS"
          aria-controls="desktop-taskbar"
          aria-expanded={visible}
          data-testid="desktop-taskbar-avatar"
          // A pinned taskbar stays until it is unpinned.
          onClick={pinned ? undefined : onToggleOpen}
          className="absolute z-[56] inline-flex items-center justify-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] font-display text-[13px] font-semibold tracking-[-0.04em] text-[color:var(--cp-text)] shadow-[0_6px_16px_color-mix(in_srgb,var(--cp-shadow)_14%,transparent)] backdrop-blur-md transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.94]"
          style={{
            left: safeArea.left + desktopAvatarOffset.left,
            top: safeArea.top + desktopAvatarOffset.top,
            width: desktopAvatarSize,
            height: desktopAvatarSize,
          }}
        >
          B
        </button>
      </Tooltip>
    </div>
  )
}
