/* The mobile status bar's message tray (red count) and notification bell (blue count). */

import clsx from 'clsx'
import {
  AppWindow,
  Ban,
  Bell,
  Bot,
  CheckCircle2,
  Download,
  Eye,
  EyeOff,
  ListTodo,
  MessageCircle,
  PauseCircle,
  Users,
  X,
  XCircle,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { useI18n } from '../../i18n/provider'
import { globalSettingsStore } from '../../app/settings/data/store'
import {
  badgeLabel,
  isTaskFinished,
  messageSessionPath,
  notificationCount,
  notifyPath,
  taskPath,
  trayStore,
  type TrayMessageItem,
  type TrayNotifyItem,
  type TraySnapshot,
  type TrayTaskItem,
} from './trayModel'

type Translate = ReturnType<typeof useI18n>['t']

/* ── Shared popover ── */

function useDismiss(
  open: boolean,
  containerRef: RefObject<HTMLDivElement | null>,
  panelRef: RefObject<HTMLDivElement | null>,
  buttonRef: RefObject<HTMLButtonElement | null>,
  close: () => void,
) {
  useEffect(() => {
    if (!open) return
    const frameId = window.requestAnimationFrame(() => panelRef.current?.focus())
    // The panel is portaled out of the container: both count as inside.
    const outside = (target: EventTarget | null) =>
      !containerRef.current?.contains(target as Node) && !panelRef.current?.contains(target as Node)
    const onPointerDown = (event: PointerEvent) => { if (outside(event.target)) close() }
    const onFocusIn = (event: FocusEvent) => { if (outside(event.target)) close() }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        close()
        buttonRef.current?.focus()
      }
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('focusin', onFocusIn)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.cancelAnimationFrame(frameId)
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('focusin', onFocusIn)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [open, containerRef, panelRef, buttonRef, close])
}

/**
 * Where the panel goes: just under the button, and the arrow under the
 * button's centre. The panel is fixed to the viewport (portaled to the body:
 * the status pill's backdrop filter would otherwise be its containing block).
 */
function usePlacement(open: boolean, panelRef: RefObject<HTMLDivElement | null>, buttonRef: RefObject<HTMLButtonElement | null>) {
  const [placement, setPlacement] = useState<{ top: number; arrow: number | null }>({ top: 56, arrow: null })
  useEffect(() => {
    if (!open) return
    const sync = () => {
      const button = buttonRef.current?.getBoundingClientRect()
      if (!button) return
      const panel = panelRef.current?.getBoundingClientRect()
      const center = panel ? button.left + button.width / 2 - panel.left : null
      setPlacement({
        top: Math.round(button.bottom + 10),
        arrow: panel && center !== null ? Math.max(24, Math.min(center, panel.width - 24)) : null,
      })
    }
    sync()
    const frameId = window.requestAnimationFrame(sync)
    window.addEventListener('resize', sync)
    return () => {
      window.cancelAnimationFrame(frameId)
      window.removeEventListener('resize', sync)
    }
  }, [open, panelRef, buttonRef])
  return placement
}

function TrayPopover({
  badge,
  badgeTone,
  children,
  icon,
  label,
  onOpen,
  testId,
}: {
  badge: number
  badgeTone: 'danger' | 'accent'
  children: (close: () => void) => ReactNode
  icon: ReactNode
  label: string
  onOpen?: () => void
  testId: string
}) {
  const [open, setOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement | null>(null)
  const panelRef = useRef<HTMLDivElement | null>(null)
  const buttonRef = useRef<HTMLButtonElement | null>(null)
  const close = useCallback(() => setOpen(false), [])
  useDismiss(open, containerRef, panelRef, buttonRef, close)
  const placement = usePlacement(open, panelRef, buttonRef)
  const panelId = `${testId}-panel`

  return (
    <div ref={containerRef} className="pointer-events-none inline-flex items-center justify-center">
      <button
        ref={buttonRef}
        type="button"
        aria-label={badge > 0 ? `${label} (${badge})` : label}
        aria-controls={panelId}
        aria-expanded={open}
        aria-haspopup="dialog"
        data-testid={`${testId}-button`}
        onClick={() => {
          if (!open) onOpen?.()
          setOpen((value) => !value)
        }}
        className={clsx(
          'pointer-events-auto relative inline-flex size-7 items-center justify-center rounded-full text-[color:var(--cp-text)] transition-colors duration-150 ease-[var(--cp-ease-emphasis)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]',
          open
            ? 'bg-[color:color-mix(in_srgb,var(--cp-text)_10%,var(--cp-surface))]'
            : 'hover:bg-[color:color-mix(in_srgb,var(--cp-surface-2)_92%,transparent)]',
        )}
      >
        {icon}
        {badge > 0 ? (
          <span
            data-testid={`${testId}-badge`}
            className={clsx(
              'absolute -right-1.5 -top-1.5 inline-flex min-h-4 min-w-4 items-center justify-center rounded-full px-1 text-[9px] font-semibold leading-none text-white',
              badgeTone === 'danger' ? 'bg-[color:var(--cp-danger)]' : 'bg-[color:var(--cp-accent)]',
            )}
          >
            {badgeLabel(badge)}
          </span>
        ) : null}
      </button>
      {open ? createPortal(
        <div
          id={panelId}
          ref={panelRef}
          role="dialog"
          aria-label={label}
          data-testid={`${testId}-panel`}
          tabIndex={-1}
          className="fixed z-[90] flex flex-col outline-none"
          style={{
            top: placement.top,
            right: 'calc(env(safe-area-inset-right, 0px) + 12px)',
            width: 'min(22rem, calc(100vw - env(safe-area-inset-left, 0px) - env(safe-area-inset-right, 0px) - 24px))',
            maxHeight: `min(65dvh, calc(100dvh - ${placement.top + 16}px))`,
          }}
        >
          <div
            className="absolute -top-[9px] h-[18px] w-[18px] -translate-x-1/2 rotate-45 border-l border-t border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_96%,transparent)]"
            style={{ left: placement.arrow ?? 24, visibility: placement.arrow === null ? 'hidden' : undefined }}
          />
          <div
            className="shell-panel flex flex-col overflow-hidden rounded-[20px] shadow-[0_20px_52px_color-mix(in_srgb,var(--cp-shadow)_18%,transparent)] backdrop-blur-xl"
            style={{ maxHeight: 'inherit' }}
          >
            {children(close)}
          </div>
        </div>,
        document.body,
      ) : null}
    </div>
  )
}

function PanelHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)] px-4 py-2.5">
      <h2 className="min-w-0 flex-1 truncate font-display text-[13px] font-semibold text-[color:var(--cp-text)]">{title}</h2>
      {children}
    </div>
  )
}

function HeaderButton({ children, label, onClick, testId }: { children: ReactNode; label: string; onClick: () => void; testId?: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      data-testid={testId}
      className="inline-flex h-7 shrink-0 items-center gap-1 rounded-full px-2 text-[11px] font-medium text-[color:var(--cp-muted)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-surface-2)_92%,transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]"
    >
      {children}
    </button>
  )
}

function EmptyState({ children }: { children: ReactNode }) {
  return <p className="px-4 py-8 text-center text-xs text-[color:var(--cp-muted)]">{children}</p>
}

function useRelativeTime() {
  const { locale } = useI18n()
  const format = useMemo(() => new Intl.RelativeTimeFormat(locale, { numeric: 'auto', style: 'narrow' }), [locale])
  return (timestamp: number) => {
    const seconds = Math.round((timestamp - Date.now()) / 1000)
    const abs = Math.abs(seconds)
    if (abs < 60) return format.format(0, 'second')
    if (abs < 3600) return format.format(Math.round(seconds / 60), 'minute')
    if (abs < 86_400) return format.format(Math.round(seconds / 3600), 'hour')
    if (abs < 7 * 86_400) return format.format(Math.round(seconds / 86_400), 'day')
    return new Date(timestamp).toLocaleDateString(locale, { month: 'short', day: 'numeric' })
  }
}

const rowClass =
  'flex w-full items-start gap-3 rounded-[14px] px-3 py-2.5 text-left transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)] active:bg-[color:color-mix(in_srgb,var(--cp-text)_10%,transparent)]'

/* ── Messages ── */

function isImageSource(value: string | undefined): value is string {
  return Boolean(value && /^(data:image\/|https?:\/\/|\/)/.test(value))
}

function ConversationAvatar({ item }: { item: TrayMessageItem }) {
  const [broken, setBroken] = useState(false)
  if (isImageSource(item.avatar) && !broken) {
    return <img src={item.avatar} alt="" className="size-9 shrink-0 rounded-full object-cover" onError={() => setBroken(true)} />
  }
  return (
    <span className="inline-flex size-9 shrink-0 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,var(--cp-surface))] text-sm font-semibold text-[color:var(--cp-accent)]">
      {item.isGroup ? <Users className="size-4" /> : (Array.from(item.title.trim())[0] ?? '?').toUpperCase()}
    </span>
  )
}

function messagePreview(item: TrayMessageItem, showContent: boolean, t: Translate): string {
  if (!showContent) {
    return item.unreadCount === 1
      ? t('tray.messages.newOne', '1 new message')
      : t('tray.messages.newCount', '{{count}} new messages', { count: item.unreadCount })
  }
  const text = item.text || t('tray.messages.noPreview', 'New message')
  return item.senderName ? `${item.senderName}: ${text}` : text
}

function MessageRow({ item, onOpen, showContent }: { item: TrayMessageItem; onOpen: () => void; showContent: boolean }) {
  const { t } = useI18n()
  const relative = useRelativeTime()
  return (
    <button type="button" className={rowClass} onClick={onOpen} data-testid="tray-message-row" data-session-id={item.sessionId}>
      <ConversationAvatar item={item} />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-[color:var(--cp-text)]">{item.title}</span>
          <span className="shrink-0 text-[10px] text-[color:var(--cp-muted)]">{relative(item.timestamp)}</span>
        </span>
        <span className="mt-0.5 flex items-start gap-2">
          <span className="line-clamp-2 min-w-0 flex-1 break-words text-[12px] leading-[18px] text-[color:var(--cp-muted)]" data-testid="tray-row-text">
            {messagePreview(item, showContent, t)}
          </span>
          <span className="mt-0.5 inline-flex min-h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-[color:var(--cp-danger)] px-1 text-[9px] font-semibold leading-none text-white">
            {badgeLabel(item.unreadCount)}
          </span>
        </span>
      </span>
    </button>
  )
}

export function MessageTray({
  onNavigate,
  snapshot,
}: {
  onNavigate: (path: string) => void
  snapshot: TraySnapshot['messages']
}) {
  const { t } = useI18n()
  const showContent = useSyncExternalStore(globalSettingsStore.subscribe, () => globalSettingsStore.getSnapshot().messagePreview)
  return (
    <TrayPopover
      badge={snapshot.unread}
      badgeTone="danger"
      icon={<MessageCircle className="size-4" />}
      label={t('tray.messages.title', 'Messages')}
      onOpen={() => trayStore.refresh()}
      testId="status-tray-messages"
    >
      {(close) => (
        <>
          <PanelHeader title={t('tray.messages.title', 'Messages')}>
            <HeaderButton
              label={showContent ? t('tray.messages.hideContent', 'Hide message content') : t('tray.messages.showContent', 'Show message content')}
              onClick={() => globalSettingsStore.setMessagePreview(!showContent)}
              testId="tray-preview-toggle"
            >
              {showContent ? <Eye className="size-3.5" /> : <EyeOff className="size-3.5" />}
            </HeaderButton>
            <HeaderButton
              label={t('tray.messages.openAll', 'Open MessageHub')}
              onClick={() => { close(); onNavigate('/messagehub') }}
            >
              {t('tray.messages.all', 'All')}
            </HeaderButton>
          </PanelHeader>
          <div className="shell-scrollbar min-h-0 flex-1 overflow-y-auto p-1.5">
            {snapshot.items.length > 0 ? (
              snapshot.items.map((item) => (
                <MessageRow
                  key={item.sessionId}
                  item={item}
                  showContent={showContent}
                  onOpen={() => { close(); onNavigate(messageSessionPath(item)) }}
                />
              ))
            ) : (
              <EmptyState>
                {snapshot.status === 'error'
                  ? t('tray.messages.unavailable', 'Messages could not be loaded.')
                  : snapshot.status === 'ready'
                    ? t('tray.messages.empty', 'No unread messages')
                    : t('tray.loading', 'Loading…')}
              </EmptyState>
            )}
          </div>
        </>
      )}
    </TrayPopover>
  )
}

/* ── Notifications ── */

const categoryIcons = { todo: ListTodo, agent: Bot, app: AppWindow } as const

function categoryLabel(category: TrayNotifyItem['category'], t: Translate): string {
  if (category === 'todo') return t('tray.notify.todo', 'To-do')
  if (category === 'agent') return t('tray.notify.agent', 'Agent')
  return t('tray.notify.app', 'App')
}

function RowIcon({ children, tone }: { children: ReactNode; tone: string }) {
  return (
    <span
      className="mt-0.5 inline-flex size-8 shrink-0 items-center justify-center rounded-full"
      style={{ backgroundColor: `color-mix(in srgb, ${tone} 14%, var(--cp-surface))`, color: tone }}
    >
      {children}
    </span>
  )
}

function NotifyRow({ item, onOpen }: { item: TrayNotifyItem; onOpen: () => void }) {
  const { t } = useI18n()
  const relative = useRelativeTime()
  const Icon = categoryIcons[item.category]
  return (
    <button type="button" className={rowClass} onClick={onOpen} data-testid="tray-notify-row" data-notify-id={item.notifyId}>
      <RowIcon tone="var(--cp-accent)"><Icon className="size-4" /></RowIcon>
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-[color:var(--cp-text)]">{item.title}</span>
          <span className="shrink-0 text-[10px] text-[color:var(--cp-muted)]">{relative(item.timestamp)}</span>
        </span>
        {item.message ? (
          <span className="mt-0.5 line-clamp-2 break-words text-[12px] leading-[18px] text-[color:var(--cp-muted)]">{item.message}</span>
        ) : null}
        <span className="mt-1 flex flex-wrap items-center gap-1.5 text-[10px] text-[color:var(--cp-muted)]">
          <span>{categoryLabel(item.category, t)}</span>
          {item.needsAnswer ? (
            <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-warning)_16%,var(--cp-surface))] px-1.5 py-0.5 font-medium text-[color:var(--cp-warning)]">
              {t('tray.notify.needsAnswer', 'Needs your answer')}
            </span>
          ) : null}
        </span>
      </span>
    </button>
  )
}

function taskStateText(task: TrayTaskItem, t: Translate): string {
  switch (task.state) {
    case 'completed':
      return t('tray.task.completed', 'Completed')
    case 'failed':
      return task.message ? t('tray.task.failedWith', 'Failed: {{detail}}', { detail: task.message }) : t('tray.task.failed', 'Failed')
    case 'canceled':
      return t('tray.task.canceled', 'Canceled')
    case 'waiting':
      return task.message || t('tray.task.waiting', 'Waiting')
    case 'paused':
      return t('tray.task.paused', 'Paused')
    default:
      return task.message || t('tray.task.running', 'In progress')
  }
}

function TaskRow({ item, onDismiss, onOpen }: { item: TrayTaskItem; onDismiss: () => void; onOpen: () => void }) {
  const { t } = useI18n()
  const relative = useRelativeTime()
  const finished = isTaskFinished(item.state)
  const tone =
    item.state === 'completed' ? 'var(--cp-success)'
      : item.state === 'failed' ? 'var(--cp-danger)'
        : item.state === 'canceled' ? 'var(--cp-muted)'
          : item.state === 'running' ? 'var(--cp-accent)'
            : 'var(--cp-warning)'
  const icon =
    item.state === 'completed' ? <CheckCircle2 className="size-4" />
      : item.state === 'failed' ? <XCircle className="size-4" />
        : item.state === 'canceled' ? <Ban className="size-4" />
          : item.state === 'running' ? <Download className="size-4" />
            : <PauseCircle className="size-4" />
  return (
    <div className="relative" data-testid="tray-task-row" data-task-id={item.taskId} data-state={item.state}>
      <button type="button" className={clsx(rowClass, finished && 'pr-10')} onClick={onOpen}>
        <RowIcon tone={tone}>{icon}</RowIcon>
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-[color:var(--cp-text)]">{item.title}</span>
            {finished ? <span className="shrink-0 text-[10px] text-[color:var(--cp-muted)]">{relative(item.timestamp)}</span> : null}
          </span>
          <span className="mt-0.5 line-clamp-2 break-words text-[12px] leading-[18px] text-[color:var(--cp-muted)]" data-testid="tray-row-text">
            {taskStateText(item, t)}
          </span>
          {!finished ? (
            <span className="mt-1.5 flex items-center gap-2">
              <span
                className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_80%,transparent)]"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={item.progress ?? undefined}
                aria-label={item.title}
              >
                <span
                  className={clsx('block h-full rounded-full bg-[color:var(--cp-accent)] transition-[width] duration-500', item.progress === null && 'w-1/3 animate-pulse')}
                  style={item.progress === null ? undefined : { width: `${item.progress}%` }}
                />
              </span>
              {item.progress !== null ? (
                <span className="w-8 shrink-0 text-right text-[10px] tabular-nums text-[color:var(--cp-muted)]">{item.progress}%</span>
              ) : null}
            </span>
          ) : null}
        </span>
      </button>
      {finished ? (
        <button
          type="button"
          aria-label={t('tray.task.dismiss', 'Dismiss')}
          title={t('tray.task.dismiss', 'Dismiss')}
          onClick={onDismiss}
          className="absolute right-2 top-2.5 inline-flex size-7 items-center justify-center rounded-full text-[color:var(--cp-muted)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]"
        >
          <X className="size-3.5" />
        </button>
      ) : null}
    </div>
  )
}

type NotificationEntry =
  | { kind: 'notify'; item: TrayNotifyItem; timestamp: number }
  | { kind: 'task'; item: TrayTaskItem; timestamp: number }

export function NotificationTray({
  onNavigate,
  snapshot,
}: {
  onNavigate: (path: string) => void
  snapshot: TraySnapshot
}) {
  const { t } = useI18n()
  const { notifies, tasks, status } = snapshot.notifications
  const running = tasks.filter((task) => !isTaskFinished(task.state))
  const finished = tasks.filter((task) => isTaskFinished(task.state))
  // Running tasks stay on top; notifications and finished tasks by time.
  const entries: NotificationEntry[] = [
    ...notifies.map((item) => ({ kind: 'notify' as const, item, timestamp: item.timestamp })),
    ...finished.map((item) => ({ kind: 'task' as const, item, timestamp: item.timestamp })),
  ].sort((a, b) => b.timestamp - a.timestamp)

  const openTask = (task: TrayTaskItem, close: () => void) => {
    if (isTaskFinished(task.state)) trayStore.dismissTask(task.taskId)
    close()
    onNavigate(taskPath(task.taskId))
  }

  return (
    <TrayPopover
      badge={notificationCount(snapshot)}
      badgeTone="accent"
      icon={<Bell className="size-4" />}
      label={t('tray.notify.title', 'Notifications')}
      onOpen={() => trayStore.refresh()}
      testId="status-tray-notifications"
    >
      {(close) => (
        <>
          <PanelHeader title={t('tray.notify.title', 'Notifications')}>
            {finished.length > 0 ? (
              <HeaderButton
                label={t('tray.task.clearFinished', 'Clear finished tasks')}
                onClick={() => trayStore.dismissFinishedTasks()}
                testId="tray-clear-finished"
              >
                {t('tray.task.clear', 'Clear')}
              </HeaderButton>
            ) : null}
            <HeaderButton
              label={t('tray.notify.openAll', 'Open the Task Center')}
              onClick={() => { close(); onNavigate('/taskcenter') }}
            >
              {t('tray.notify.tasks', 'Tasks')}
            </HeaderButton>
          </PanelHeader>
          <div className="shell-scrollbar min-h-0 flex-1 overflow-y-auto p-1.5">
            {running.map((item) => (
              <TaskRow key={item.taskId} item={item} onOpen={() => openTask(item, close)} onDismiss={() => undefined} />
            ))}
            {entries.map((entry) =>
              entry.kind === 'notify' ? (
                <NotifyRow
                  key={`notify-${entry.item.notifyId}`}
                  item={entry.item}
                  onOpen={() => { close(); onNavigate(notifyPath(entry.item.notifyId)) }}
                />
              ) : (
                <TaskRow
                  key={`task-${entry.item.taskId}`}
                  item={entry.item}
                  onOpen={() => openTask(entry.item, close)}
                  onDismiss={() => trayStore.dismissTask(entry.item.taskId)}
                />
              ),
            )}
            {running.length === 0 && entries.length === 0 ? (
              <EmptyState>
                {status === 'error'
                  ? t('tray.notify.unavailable', 'Notifications could not be loaded.')
                  : status === 'ready'
                    ? t('tray.notify.empty', 'No new notifications')
                    : t('tray.loading', 'Loading…')}
              </EmptyState>
            ) : null}
          </div>
        </>
      )}
    </TrayPopover>
  )
}
