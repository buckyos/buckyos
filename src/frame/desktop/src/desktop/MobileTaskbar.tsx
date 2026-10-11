import clsx from 'clsx'
import { Grip, House } from 'lucide-react'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type Ref,
} from 'react'
import type { DesktopAppItem } from '../app/types'
import { useI18n } from '../i18n/provider'
import type { LayoutState, TaskbarAppItem } from '../models/ui'
import { AllAppsPanel } from './AllAppsPanel'
import {
  ProfileCard,
  SystemStatusCard,
  SystemStatusLines,
  TaskbarAppTile,
  TaskbarMenu,
  UserAvatar,
  type TaskbarMenuState,
} from './TaskbarParts'
import { useHoverCard, useLongPress, useSystemStatus, type CurrentUser } from './taskbarHooks'
import type { ConnectionState } from './shell'
import { useAppText } from '../presentation/appText'
import { openUsersAgents } from '../app/users-agents/launch'

type SafeArea = { top: number; bottom: number; left: number; right: number }

// The rail's avatar sits right over the status bar's (x 12–48).
const railWidth = 60
const avatarSize = 36
const avatarInset = 12
const appTileSize = 40
const popupGap = 8
/** Where the account is managed from the profile card. */
const accountAppId = 'users-agents'

function RailButton({
  appId,
  buttonRef,
  children,
  label,
  onClick,
  onContextMenu,
  pressHandlers,
  selected = false,
  testId,
}: {
  appId?: string
  buttonRef?: Ref<HTMLButtonElement>
  children: ReactNode
  label: string
  onClick: () => void
  onContextMenu?: (event: ReactMouseEvent<HTMLButtonElement>) => void
  pressHandlers?: ReturnType<typeof useLongPress>['handlers']
  selected?: boolean
  testId?: string
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      aria-label={label}
      data-app-id={appId}
      data-testid={testId}
      {...pressHandlers}
      onClick={onClick}
      onContextMenu={onContextMenu}
      className={clsx(
        'relative flex size-12 shrink-0 select-none items-center justify-center rounded-[14px] border transition-[background-color,border-color,color,transform] duration-150 ease-[var(--cp-ease-emphasis)] [-webkit-touch-callout:none] active:scale-[0.94]',
        selected
          ? 'border-[color:color-mix(in_srgb,var(--cp-accent)_24%,var(--cp-border))] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] text-[color:var(--cp-text)]'
          : 'border-transparent text-[color:var(--cp-muted)] active:bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)]',
      )}
    >
      {children}
    </button>
  )
}

function RailDivider() {
  return <div className="my-1 h-px w-8 shrink-0 bg-[color:color-mix(in_srgb,var(--cp-border)_80%,transparent)]" />
}

/**
 * The mobile shell's taskbar: the status bar's avatar slides out a narrow
 * rail over a dimmed screen. It mirrors the desktop taskbar — avatar (tap
 * for the profile card with Log out), All apps (full screen), pinned and
 * running apps (long-press to pin or unpin), two lines of system status
 * (tap for details) — and ends with a Show desktop button.
 */
export function MobileTaskbar({
  apps,
  avatarRowHeight,
  connectionState,
  deadZone,
  isLoggingOut = false,
  onClose,
  onCloseAppWindows,
  onLogout,
  onOpenApp,
  onSetAppPinned,
  onShowDesktop,
  open,
  runtimeContainer,
  safeArea,
  taskbarApps,
  user,
}: {
  apps: DesktopAppItem[]
  /** Height of the status bar row the rail's avatar lines up with. */
  avatarRowHeight: number
  connectionState: ConnectionState
  deadZone: LayoutState['deadZone']
  isLoggingOut?: boolean
  onClose: () => void
  onCloseAppWindows: (appId: string) => void
  onLogout: () => void | Promise<void>
  /** Launch an app (or bring its window up) and close the rail. */
  onOpenApp: (appId: string) => void
  onSetAppPinned: (appId: string, pinned: boolean) => void
  onShowDesktop: () => void
  open: boolean
  runtimeContainer: string
  safeArea: SafeArea
  taskbarApps: TaskbarAppItem[]
  user: CurrentUser | null
}) {
  const { t } = useI18n()
  const appText = useAppText()
  const avatarRef = useRef<HTMLButtonElement | null>(null)
  const profileCardRef = useRef<HTMLDivElement | null>(null)
  const statusButtonRef = useRef<HTMLButtonElement | null>(null)
  const statusCardRef = useRef<HTMLDivElement | null>(null)
  const [allAppsOpen, setAllAppsOpen] = useState(false)
  const [menu, setMenu] = useState<TaskbarMenuState | null>(null)
  const profileCard = useHoverCard()
  const statusCard = useHoverCard()
  const { close: closeProfileCard, held: profileHeld } = profileCard
  const { close: closeStatusCard, held: statusHeld } = statusCard
  const systemStatus = useSystemStatus(open)

  const pinnedAppIds = useMemo(
    () => new Set(taskbarApps.filter((app) => app.pinned).map((app) => app.appId)),
    [taskbarApps],
  )
  const hasAccountApp = apps.some((app) => app.id === accountAppId)

  const closeCards = useCallback(() => {
    closeProfileCard()
    closeStatusCard()
  }, [closeProfileCard, closeStatusCard])

  const close = useCallback(() => {
    closeCards()
    onClose()
  }, [closeCards, onClose])

  useEffect(() => {
    if (!open && !allAppsOpen) return
    const inside = (target: Node, ...elements: Array<HTMLElement | null>) =>
      elements.some((element) => element?.contains(target))
    // A held card goes when a tap misses it and its button.
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (profileHeld && !inside(target, profileCardRef.current, avatarRef.current)) closeProfileCard()
      if (statusHeld && !inside(target, statusCardRef.current, statusButtonRef.current)) closeStatusCard()
    }
    // Escape closes the innermost layer first.
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || menu) return
      if (allAppsOpen) setAllAppsOpen(false)
      else if (profileHeld || statusHeld) closeCards()
      else close()
    }
    document.addEventListener('pointerdown', handlePointerDown, true)
    window.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true)
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [allAppsOpen, close, closeCards, closeProfileCard, closeStatusCard, menu, open, profileHeld, statusHeld])

  const launchApp = (appId: string) => {
    setMenu(null)
    setAllAppsOpen(false)
    closeCards()
    onOpenApp(appId)
  }

  const openAppMenu = (appId: string, point: { x: number; y: number }) => setMenu({ appId, ...point })
  const appLongPress = useLongPress((element, point) => {
    if (element.dataset.appId) openAppMenu(element.dataset.appId, point)
  })

  const menuApp = menu?.appId ? apps.find((app) => app.id === menu.appId) : undefined
  const menuTaskbarItem = menu?.appId ? taskbarApps.find((item) => item.appId === menu.appId) : undefined

  const insetTop = safeArea.top + deadZone.top
  const insetBottom = safeArea.bottom + deadZone.bottom
  const popupLeft = safeArea.left + railWidth + popupGap
  const popupWidth = (width: number) => `min(${width}px, calc(100vw - ${popupLeft + popupGap}px))`

  return (
    <>
      <div
        className={clsx(
          'absolute inset-0 z-[60] transition-opacity duration-200 ease-[var(--cp-ease-emphasis)]',
          open ? 'pointer-events-auto opacity-100' : 'pointer-events-none opacity-0',
        )}
        aria-hidden={!open}
        inert={!open}
      >
        <button
          type="button"
          aria-label={t('common.close')}
          onClick={close}
          className="absolute inset-0 bg-[color:color-mix(in_srgb,var(--cp-shadow)_32%,transparent)] backdrop-blur-[2px]"
        />
        <aside
          aria-label={t('shell.taskbar', 'Taskbar')}
          data-testid="mobile-taskbar"
          data-open={open}
          className={clsx(
            'absolute inset-y-0 left-0 flex flex-col items-center border-r border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface)_98%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))] shadow-[0_18px_56px_color-mix(in_srgb,var(--cp-shadow)_18%,transparent)] backdrop-blur-xl transition-transform duration-250 ease-[var(--cp-ease-emphasis)]',
            open ? 'translate-x-0' : '-translate-x-full',
          )}
          style={{ width: safeArea.left + railWidth, paddingLeft: safeArea.left }}
        >
          <div
            className="flex w-full shrink-0 items-center"
            style={{ height: insetTop + avatarRowHeight, paddingTop: insetTop, paddingLeft: avatarInset }}
          >
            <button
              ref={avatarRef}
              type="button"
              aria-label={t('shell.profile', 'Account')}
              aria-expanded={profileCard.open}
              data-testid="mobile-taskbar-avatar"
              {...profileCard.hoverProps}
              onClick={profileCard.toggleHeld}
              className="inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] text-[color:var(--cp-text)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)] transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.94]"
              style={{ width: avatarSize, height: avatarSize }}
            >
              <UserAvatar size={avatarSize} user={user} />
            </button>
          </div>
          <div className="flex shrink-0 flex-col items-center pt-2">
            <RailButton
              label={t('shell.allApps', 'All apps')}
              onClick={() => {
                close()
                setAllAppsOpen(true)
              }}
              testId="taskbar-all-apps"
            >
              <Grip className="size-5" />
            </RailButton>
            <RailDivider />
          </div>
          <nav
            aria-label={t('shell.taskbarApps', 'Apps')}
            className="desktop-scrollbar flex min-h-0 w-full flex-1 flex-col items-center gap-1.5 overflow-y-auto overflow-x-hidden pb-2"
          >
            {taskbarApps.map((app) => (
              <RailButton
                key={app.appId}
                appId={app.appId}
                label={appText(app).title}
                onClick={() => {
                  if (!appLongPress.consumeLongPress()) launchApp(app.appId)
                }}
                onContextMenu={(event) => {
                  event.preventDefault()
                  openAppMenu(app.appId, { x: event.clientX, y: event.clientY })
                }}
                pressHandlers={appLongPress.handlers}
                selected={app.active}
                testId={`taskbar-app-${app.appId}`}
              >
                <TaskbarAppTile indicatorOffset={4} item={app} size={appTileSize} />
              </RailButton>
            ))}
          </nav>
          <div className="flex w-full shrink-0 flex-col items-center gap-1" style={{ paddingBottom: insetBottom + 10 }}>
            <RailDivider />
            <button
              ref={statusButtonRef}
              type="button"
              aria-label={t('shell.status.title', 'System status')}
              aria-expanded={statusCard.open}
              data-testid="taskbar-status"
              {...statusCard.hoverProps}
              onClick={statusCard.toggleHeld}
              className={clsx(
                'flex w-[52px] items-center justify-center rounded-[10px] py-1.5 transition-colors',
                statusCard.held ? 'bg-[color:color-mix(in_srgb,var(--cp-accent)_12%,transparent)]' : '',
              )}
            >
              <SystemStatusLines connectionState={connectionState} status={systemStatus} />
            </button>
            <RailButton
              label={t('shell.showDesktop', 'Show desktop')}
              onClick={() => {
                closeCards()
                onShowDesktop()
              }}
              testId="taskbar-show-desktop"
            >
              <House className="size-5" />
            </RailButton>
          </div>
        </aside>

        {open && statusCard.open ? (
          <SystemStatusCard
            cardRef={statusCardRef}
            connectionState={connectionState}
            hoverProps={statusCard.hoverProps}
            runtimeContainer={runtimeContainer}
            status={systemStatus}
            style={{ left: popupLeft, bottom: insetBottom + 12, width: popupWidth(296) }}
          />
        ) : null}

        {open && profileCard.open ? (
          <ProfileCard
            cardRef={profileCardRef}
            connectionState={connectionState}
            hoverProps={profileCard.hoverProps}
            isLoggingOut={isLoggingOut}
            onLogout={() => {
              void onLogout()
            }}
            onOpenAccount={hasAccountApp ? () => { setMenu(null); setAllAppsOpen(false); closeCards(); void openUsersAgents({ view: 'self' }) } : undefined}
            runtimeContainer={runtimeContainer}
            style={{ left: popupLeft, top: insetTop + (avatarRowHeight - avatarSize) / 2, width: popupWidth(288) }}
            user={user}
          />
        ) : null}
      </div>

      {allAppsOpen ? (
        <AllAppsPanel
          apps={apps}
          insets={{ top: insetTop, bottom: insetBottom }}
          onAppMenu={openAppMenu}
          onClose={() => setAllAppsOpen(false)}
          onOpenApp={launchApp}
          pinnedAppIds={pinnedAppIds}
          variant="fullscreen"
        />
      ) : null}

      <TaskbarMenu
        app={menuApp}
        menu={menuApp ? menu : null}
        onClose={() => setMenu(null)}
        onCloseWindows={onCloseAppWindows}
        onOpenApp={launchApp}
        onSetPinned={onSetAppPinned}
        pinned={Boolean(menuApp && pinnedAppIds.has(menuApp.id))}
        windowCount={menuTaskbarItem?.windowCount ?? 0}
      />
    </>
  )
}
