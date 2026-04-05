import { useEffect, useState } from 'react'
import { useI18n } from '../i18n/provider'
import type { AppDefinition, FormFactor } from '../models/ui'

const statusBarHeights = {
  desktop: 42,
  mobileHome: 40,
  mobileCompact: 46,
  mobileStandard: 58,
} as const

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

export function shellStatusBarHeight(
  formFactor: FormFactor,
  activeApp?: AppDefinition,
) {
  if (formFactor === 'desktop') {
    return statusBarHeights.desktop
  }

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
