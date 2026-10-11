import { ListItemIcon, MenuItem, Tooltip } from '@mui/material'
import clsx from 'clsx'
import { Check, Grip } from 'lucide-react'
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
import type { TaskbarAppItem } from '../models/ui'
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
import {
  desktopAvatarOffset,
  desktopAvatarRowHeight,
  desktopAvatarSize,
  desktopTaskbarWidth,
  type ConnectionState,
} from './shell'
import { useAppText } from '../presentation/appText'
import { openUsersAgents } from '../app/users-agents/launch'

type SafeArea = { top: number; bottom: number; left: number; right: number }

const appTileSize = 36
const allAppsPanelWidth = 320
const popupGap = 8
/** Where the account is managed from the profile card. */
const accountAppId = 'users-agents'

function TaskbarButton({
  appId,
  buttonRef,
  children,
  expanded,
  label,
  onClick,
  onContextMenu,
  pressHandlers,
  selected = false,
  testId,
}: {
  /** Set on app buttons, for the shared long-press handlers. */
  appId?: string
  buttonRef?: Ref<HTMLButtonElement>
  children: ReactNode
  /** Set for buttons that open a panel. */
  expanded?: boolean
  label: string
  onClick: () => void
  onContextMenu?: (event: ReactMouseEvent<HTMLButtonElement>) => void
  /** Touch long-press handlers (see `useLongPress`). */
  pressHandlers?: ReturnType<typeof useLongPress>['handlers']
  selected?: boolean
  testId?: string
}) {
  return (
    <Tooltip title={label} placement="right" disableInteractive>
      <button
        ref={buttonRef}
        type="button"
        aria-label={label}
        aria-expanded={expanded}
        data-testid={testId}
        data-app-id={appId}
        {...pressHandlers}
        onClick={onClick}
        onContextMenu={onContextMenu}
        className={clsx(
          'relative flex size-11 select-none [-webkit-touch-callout:none] shrink-0 items-center justify-center rounded-[12px] border transition-[background-color,border-color,color,transform] duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]',
          selected
            ? 'border-[color:color-mix(in_srgb,var(--cp-accent)_24%,var(--cp-border))] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] text-[color:var(--cp-text)]'
            : expanded
              ? 'border-transparent bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] text-[color:var(--cp-accent)]'
              : 'border-transparent text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]',
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
 *
 * Top to bottom: the avatar (hover — or click while pinned — for the
 * profile card with Log out), All apps, the app buttons (pinned apps, then
 * other running apps; right-click to pin or unpin), two lines of system
 * status (hover or click for details) and the "Show desktop" strip. Pinning
 * the taskbar itself lives in its right-click menu.
 */
export function DesktopTaskbar({
  apps,
  connectionState,
  isLoggingOut = false,
  onActivateApp,
  onClose,
  onCloseAppWindows,
  onLogout,
  onOpenApp,
  onSetAppPinned,
  onShowDesktop,
  onToggleOpen,
  onTogglePinned,
  open,
  pinned,
  runtimeContainer,
  safeArea,
  taskbarApps,
  user,
}: {
  /** Every installed app, for All apps. */
  apps: DesktopAppItem[]
  connectionState: ConnectionState
  isLoggingOut?: boolean
  /** A taskbar app button was clicked: open, raise or minimize the app. */
  onActivateApp: (appId: string) => void
  onClose: () => void
  onCloseAppWindows: (appId: string) => void
  onLogout: () => void | Promise<void>
  /** Launch an app (or bring its window up). */
  onOpenApp: (appId: string) => void
  onSetAppPinned: (appId: string, pinned: boolean) => void
  onShowDesktop: () => void
  onToggleOpen: () => void
  onTogglePinned: () => void
  open: boolean
  pinned: boolean
  runtimeContainer: string
  safeArea: SafeArea
  taskbarApps: TaskbarAppItem[]
  user: CurrentUser | null
}) {
  const { t } = useI18n()
  const appText = useAppText()
  const rootRef = useRef<HTMLDivElement | null>(null)
  const avatarRef = useRef<HTMLButtonElement | null>(null)
  const allAppsButtonRef = useRef<HTMLButtonElement | null>(null)
  const allAppsPanelRef = useRef<HTMLDivElement | null>(null)
  const profileCardRef = useRef<HTMLDivElement | null>(null)
  const statusButtonRef = useRef<HTMLButtonElement | null>(null)
  const statusCardRef = useRef<HTMLDivElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const visible = open || pinned
  const autoHide = open && !pinned

  const [allAppsOpen, setAllAppsOpen] = useState(false)
  const [menu, setMenu] = useState<TaskbarMenuState | null>(null)
  const profileCard = useHoverCard()
  const statusCard = useHoverCard()
  const { close: closeProfileCard, held: profileHeld } = profileCard
  const { close: closeStatusCard, held: statusHeld } = statusCard
  const systemStatus = useSystemStatus(visible)

  const pinnedAppIds = useMemo(
    () => new Set(taskbarApps.filter((app) => app.pinned).map((app) => app.appId)),
    [taskbarApps],
  )
  const hasAccountApp = apps.some((app) => app.id === accountAppId)

  const closeLayers = useCallback(() => {
    setAllAppsOpen(false)
    setMenu(null)
    closeProfileCard()
    closeStatusCard()
  }, [closeProfileCard, closeStatusCard])

  const layerHeld = allAppsOpen || profileHeld || statusHeld
  useEffect(() => {
    if (!autoHide && !layerHeld && !menu) return

    const inside = (target: Node, ...elements: Array<HTMLElement | null>) =>
      elements.some((element) => element?.contains(target))
    // Listening (not a backdrop) keeps the dismissing click working on
    // whatever it lands on. Each open layer closes when the click misses it
    // and its anchor; an auto-hiding taskbar goes when the click misses
    // everything it owns.
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      // The context menu sits in a portal; its own backdrop dismisses it.
      // A closing menu's backdrop lingers through its exit transition, so
      // it only counts while the menu is open.
      if (menu && inside(target, menuRef.current)) return
      if (allAppsOpen && !inside(target, allAppsPanelRef.current, allAppsButtonRef.current)) {
        setAllAppsOpen(false)
      }
      if (profileHeld && !inside(target, profileCardRef.current, avatarRef.current)) closeProfileCard()
      if (statusHeld && !inside(target, statusCardRef.current, statusButtonRef.current)) closeStatusCard()
      if (autoHide && !inside(target, rootRef.current)) {
        closeLayers()
        onClose()
      }
    }
    // Escape closes the innermost open layer first.
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || menu) return
      if (allAppsOpen) {
        setAllAppsOpen(false)
        allAppsButtonRef.current?.focus()
      } else if (profileHeld || statusHeld) {
        closeProfileCard()
        closeStatusCard()
      } else if (autoHide) {
        onClose()
        avatarRef.current?.focus()
      }
    }
    // A click into an app's iframe never reaches this document; it only
    // blurs the window.
    const handleBlur = () => {
      closeLayers()
      if (autoHide) onClose()
    }

    document.addEventListener('pointerdown', handlePointerDown, true)
    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('blur', handleBlur)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true)
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('blur', handleBlur)
    }
  }, [
    allAppsOpen,
    autoHide,
    closeLayers,
    closeProfileCard,
    closeStatusCard,
    layerHeld,
    menu,
    onClose,
    profileHeld,
    statusHeld,
  ])

  const launchApp = (appId: string) => {
    closeLayers()
    onOpenApp(appId)
  }

  const openAppMenu = (appId: string, point: { x: number; y: number }) => setMenu({ appId, ...point })
  const appLongPress = useLongPress((element, point) => {
    if (element.dataset.appId) openAppMenu(element.dataset.appId, point)
  })

  const menuApp = menu?.appId ? apps.find((app) => app.id === menu.appId) : undefined
  const menuTaskbarItem = menu?.appId ? taskbarApps.find((item) => item.appId === menu.appId) : undefined

  const popupLeft = safeArea.left + desktopTaskbarWidth + popupGap

  return (
    <div ref={rootRef} className="contents">
      <aside
        id="desktop-taskbar"
        aria-label={t('shell.taskbar', 'Taskbar')}
        aria-hidden={!visible}
        inert={!visible}
        data-testid="desktop-taskbar"
        data-pinned={pinned}
        onContextMenu={(event) => {
          event.preventDefault()
          setMenu({ x: event.clientX, y: event.clientY })
        }}
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
        <div className="flex shrink-0 flex-col items-center pt-2">
          <TaskbarButton
            buttonRef={allAppsButtonRef}
            expanded={allAppsOpen}
            label={t('shell.allApps', 'All apps')}
            onClick={() => {
              setMenu(null)
              setAllAppsOpen((current) => !current)
            }}
            testId="taskbar-all-apps"
          >
            <Grip className="size-5" />
          </TaskbarButton>
          <TaskbarDivider />
        </div>
        <nav
          aria-label={t('shell.taskbarApps', 'Apps')}
          className="desktop-scrollbar flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto overflow-x-hidden pb-2"
        >
          {taskbarApps.map((app) => (
            <TaskbarButton
              key={app.appId}
              label={appText(app).title}
              onClick={() => {
                if (appLongPress.consumeLongPress()) return
                closeLayers()
                onActivateApp(app.appId)
              }}
              onContextMenu={(event) => {
                event.preventDefault()
                event.stopPropagation()
                openAppMenu(app.appId, { x: event.clientX, y: event.clientY })
              }}
              pressHandlers={appLongPress.handlers}
              selected={app.active}
              testId={`taskbar-app-${app.appId}`}
              appId={app.appId}
            >
              <TaskbarAppTile indicatorOffset={6} item={app} size={appTileSize} />
            </TaskbarButton>
          ))}
        </nav>
        <div className="flex w-full shrink-0 flex-col items-center">
          <TaskbarDivider />
          <button
            ref={statusButtonRef}
            type="button"
            aria-label={t('shell.status.title', 'System status')}
            aria-expanded={statusCard.open}
            data-testid="taskbar-status"
            {...statusCard.hoverProps}
            onClick={statusCard.toggleHeld}
            className={clsx(
              'mx-1 mb-1.5 flex w-[52px] items-center justify-center rounded-[10px] py-1 transition-colors',
              statusCard.held
                ? 'bg-[color:color-mix(in_srgb,var(--cp-accent)_12%,transparent)]'
                : 'hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)]',
            )}
          >
            <SystemStatusLines connectionState={connectionState} status={systemStatus} />
          </button>
          <Tooltip title={t('shell.showDesktop', 'Show desktop')} placement="right" disableInteractive>
            <button
              type="button"
              aria-label={t('shell.showDesktop', 'Show desktop')}
              data-testid="taskbar-show-desktop"
              onClick={() => {
                closeLayers()
                onShowDesktop()
              }}
              className="h-3.5 w-full shrink-0 border-t border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-border)_22%,transparent)] transition-colors duration-150 hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_30%,transparent)] active:bg-[color:color-mix(in_srgb,var(--cp-accent)_22%,transparent)]"
              style={{ marginBottom: safeArea.bottom }}
            />
          </Tooltip>
        </div>
      </aside>

      {visible && allAppsOpen ? (
        <AllAppsPanel
          apps={apps}
          onAppMenu={openAppMenu}
          onClose={() => setAllAppsOpen(false)}
          onOpenApp={launchApp}
          panelRef={allAppsPanelRef}
          pinnedAppIds={pinnedAppIds}
          style={{
            left: safeArea.left + desktopTaskbarWidth,
            top: 0,
            bottom: 0,
            width: allAppsPanelWidth,
            paddingTop: safeArea.top,
            paddingBottom: safeArea.bottom,
          }}
          variant="panel"
        />
      ) : null}

      {visible && statusCard.open ? (
        <SystemStatusCard
          cardRef={statusCardRef}
          connectionState={connectionState}
          hoverProps={statusCard.hoverProps}
          runtimeContainer={runtimeContainer}
          status={systemStatus}
          style={{ left: popupLeft, bottom: safeArea.bottom + 12 }}
        />
      ) : null}

      {profileCard.open ? (
        <ProfileCard
          cardRef={profileCardRef}
          connectionState={connectionState}
          hoverProps={profileCard.hoverProps}
          isLoggingOut={isLoggingOut}
          onLogout={() => {
            void onLogout()
          }}
          onOpenAccount={hasAccountApp ? () => { closeLayers(); void openUsersAgents({ view: 'self' }) } : undefined}
          runtimeContainer={runtimeContainer}
          style={{ left: popupLeft, top: safeArea.top + desktopAvatarOffset.top }}
          user={user}
        />
      ) : null}

      <button
        ref={avatarRef}
        type="button"
        aria-label="BuckyOS"
        aria-controls="desktop-taskbar"
        aria-expanded={visible}
        data-testid="desktop-taskbar-avatar"
        {...profileCard.hoverProps}
        // A click on an auto-hiding taskbar's avatar is for the taskbar, not
        // the card; a pinned taskbar stays, so the click holds the card.
        onPointerDown={pinned ? undefined : profileCard.cancelPending}
        onClick={
          pinned
            ? profileCard.toggleHeld
            : () => {
                if (visible) closeLayers()
                onToggleOpen()
              }
        }
        className="absolute z-[56] inline-flex items-center justify-center overflow-hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] text-[color:var(--cp-text)] shadow-[0_6px_16px_color-mix(in_srgb,var(--cp-shadow)_14%,transparent)] backdrop-blur-md transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.94]"
        style={{
          left: safeArea.left + desktopAvatarOffset.left,
          top: safeArea.top + desktopAvatarOffset.top,
          width: desktopAvatarSize,
          height: desktopAvatarSize,
        }}
      >
        <UserAvatar size={desktopAvatarSize} user={user} />
      </button>

      <TaskbarMenu
        app={menuApp}
        menu={menu}
        menuRef={menuRef}
        onClose={() => setMenu(null)}
        onCloseWindows={onCloseAppWindows}
        onOpenApp={launchApp}
        onSetPinned={onSetAppPinned}
        pinned={Boolean(menuApp && pinnedAppIds.has(menuApp.id))}
        windowCount={menuTaskbarItem?.windowCount ?? 0}
      >
        <MenuItem
          role="menuitemcheckbox"
          aria-checked={pinned}
          data-testid="taskbar-pin"
          onClick={() => {
            setMenu(null)
            onTogglePinned()
          }}
        >
          <ListItemIcon>{pinned ? <Check className="size-4" /> : null}</ListItemIcon>
          {t('shell.pinTaskbar', 'Keep taskbar visible')}
        </MenuItem>
      </TaskbarMenu>
    </div>
  )
}
