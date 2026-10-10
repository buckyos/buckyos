import { useEffect, useState } from 'react'
import { useI18n } from '../i18n/provider'
import type { AppDefinition } from '../models/ui'
import { desktopWindowTitleBarHeight } from './windows/geometry'

const statusBarHeights = {
  mobileHome: 40,
  mobileCompact: 46,
  mobileStandard: 58,
} as const

// Desktop has no status bar: a floating avatar in the top-left corner opens
// a narrow taskbar (dock) on the left, which can be pinned open. The avatar
// fits inside the title-bar row of a maximized window and lands centred in
// the taskbar's top slot.
export const desktopTaskbarWidth = 60
export const desktopAvatarSize = 28
export const desktopAvatarRowHeight = desktopWindowTitleBarHeight
export const desktopAvatarOffset = {
  left: (desktopTaskbarWidth - desktopAvatarSize) / 2,
  top: (desktopAvatarRowHeight - desktopAvatarSize) / 2,
} as const

/** Space the desktop shell takes from the window workspace. */
export function desktopWorkspaceInsets(taskbarPinned: boolean) {
  return { top: 0, left: taskbarPinned ? desktopTaskbarWidth : 0 }
}

export type ConnectionState = 'online' | 'degraded' | 'offline'

export type StatusTipTone = 'success' | 'error' | 'progress'

export type StatusTip = {
  id: string
  tone: StatusTipTone
  taskLabel: string
  title: string
  body: string
  statusLabel: string
  timeLabel: string
}

export type StatusTrayState = {
  backupActive: boolean
  messageCount: number
  notificationCount: number
  tips: StatusTip[]
}

export function mobileStatusBarMode(app?: AppDefinition) {
  return app?.manifest.mobileStatusBarMode ?? 'compact'
}

export function mobileStatusBarHeight(activeApp?: AppDefinition) {
  if (!activeApp) {
    return statusBarHeights.mobileHome
  }

  return mobileStatusBarMode(activeApp) === 'standard'
    ? statusBarHeights.mobileStandard
    : statusBarHeights.mobileCompact
}

export function connectionTone(state: ConnectionState) {
  if (state === 'online') {
    return 'var(--cp-success)'
  }

  if (state === 'degraded') {
    return 'var(--cp-warning)'
  }

  return 'var(--cp-danger)'
}

type TranslateFn = ReturnType<typeof useI18n>['t']

export function connectionLabel(state: ConnectionState, t: TranslateFn) {
  if (state === 'online') {
    return t('shell.online')
  }

  if (state === 'degraded') {
    return t('shell.connectionDegraded', 'Relay')
  }

  return t('shell.offline', 'Offline')
}

export function useMinuteClock() {
  const [now, setNow] = useState(() => new Date())

  useEffect(() => {
    let intervalId: number | undefined
    const timeoutId = window.setTimeout(() => {
      setNow(new Date())
      intervalId = window.setInterval(() => {
        setNow(new Date())
      }, 60_000)
    }, 60_000 - (Date.now() % 60_000))

    return () => {
      window.clearTimeout(timeoutId)
      if (intervalId) {
        window.clearInterval(intervalId)
      }
    }
  }, [])

  return now
}
