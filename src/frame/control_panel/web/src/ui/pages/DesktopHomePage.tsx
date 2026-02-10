import type { PointerEvent } from 'react'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import {
  fetchAppsList,
  fetchLayout,
  fetchSystemMetrics,
  fetchSystemStatus,
  mockDappStoreData,
  mockLayoutData,
  mockSystemMetrics,
  mockSystemStatus,
  querySystemLogs,
} from '@/api'
import { NetworkTrendChart, ResourceTrendChart } from '../components/MonitorTrendCharts'
import usePrefersReducedMotion from '../charts/usePrefersReducedMotion'
import Icon from '../icons'

type DesktopMode = 'desktop' | 'jarvis'

type WindowId = 'monitor' | 'storage' | 'logs' | 'apps' | 'settings' | 'users'

type DesktopWindow = {
  id: WindowId
  title: string
  icon: IconName
  x: number
  y: number
  width: number
  height: number
  z: number
  minimized: boolean
}

type ResizeEdge = 'top' | 'right' | 'bottom' | 'left' | 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max)

const formatBytes = (value: number) => {
  if (!Number.isFinite(value) || value <= 0) {
    return '0 B'
  }
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  const scaled = value / 1024 ** index
  return `${scaled.toFixed(scaled >= 100 || index === 0 ? 0 : 1)} ${units[index]}`
}

const formatRate = (value: number) => `${formatBytes(value)}/s`

const WINDOW_MARGIN = 24
const WINDOW_TOP_MARGIN = 80
const MIN_WINDOW_WIDTH = 420
const MIN_WINDOW_HEIGHT = 280

const SETTINGS_MODULE_PREVIEW = [
  {
    name: 'General',
    detail: 'Node identity, locale, and host naming baseline.',
    state: 'ready' as const,
  },
  {
    name: 'Security',
    detail: 'MFA policy, session timeout, and trusted devices.',
    state: 'review' as const,
  },
  {
    name: 'Networking',
    detail: 'SN endpoints, DNS fallback, and gateway behavior.',
    state: 'ready' as const,
  },
  {
    name: 'Storage',
    detail: 'Snapshot cadence, retention, and replication policy.',
    state: 'review' as const,
  },
  {
    name: 'Notifications',
    detail: 'Alert channels and severity routing matrix.',
    state: 'draft' as const,
  },
  {
    name: 'Integrations',
    detail: 'Repo hooks, observability export, and webhook secrets.',
    state: 'ready' as const,
  },
]

const USER_TEAM_PREVIEW = [
  {
    name: 'Alice Johnson',
    role: 'Owner',
    group: 'Administrators',
    status: 'active' as const,
    access: 'Full',
  },
  {
    name: 'Leo Martins',
    role: 'Admin',
    group: 'Administrators',
    status: 'active' as const,
    access: 'Ops',
  },
  {
    name: 'Rina Patel',
    role: 'Editor',
    group: 'Product',
    status: 'pending' as const,
    access: 'Limited',
  },
  {
    name: 'Tomas Silva',
    role: 'Viewer',
    group: 'Guests',
    status: 'disabled' as const,
    access: 'Read',
  },
]

const DesktopHomePage = () => {
  const navigate = useNavigate()
  const navigateTo = useCallback((to: string) => navigate(to), [navigate])
  const prefersReducedMotion = usePrefersReducedMotion()

  const [mode, setMode] = useState<DesktopMode>('desktop')
  const [layout, setLayout] = useState<RootLayoutData>(mockLayoutData)
  const [layoutError, setLayoutError] = useState<string | null>(null)
  const [metrics, setMetrics] = useState<SystemMetrics>(mockSystemMetrics)
  const [status, setStatus] = useState<SystemStatusResponse>(mockSystemStatus)
  const [apps, setApps] = useState<DappCard[]>([])
  const [appsError, setAppsError] = useState<string | null>(null)
  const [logPeek, setLogPeek] = useState<SystemLogEntry[] | null>(null)
  const [logPeekError, setLogPeekError] = useState<string | null>(null)

  const zCounterRef = useRef(10)
  const [windows, setWindows] = useState<DesktopWindow[]>([])
  const windowMemoryRef = useRef<Partial<Record<WindowId, Pick<DesktopWindow, 'x' | 'y' | 'width' | 'height'>>>>({})
  const logsWindowOpen = windows.some((win) => win.id === 'logs' && !win.minimized)
  const dragRef = useRef<{
    id: WindowId
    pointerId: number
    startX: number
    startY: number
    originX: number
    originY: number
    width: number
    height: number
  } | null>(null)
  const dragRafRef = useRef<number | null>(null)
  const dragPositionRef = useRef<{ id: WindowId; x: number; y: number } | null>(null)
  const resizeRef = useRef<{
    id: WindowId
    pointerId: number
    edge: ResizeEdge
    startX: number
    startY: number
    originX: number
    originY: number
    originWidth: number
    originHeight: number
  } | null>(null)
  const resizeRafRef = useRef<number | null>(null)
  const resizeRectRef = useRef<{ id: WindowId; x: number; y: number; width: number; height: number } | null>(null)

  const windowSpec = useMemo(
    () =>
      ({
        monitor: { title: 'System Monitor', icon: 'dashboard' as const, width: 896, height: 616 },
        storage: { title: 'Storage Manager', icon: 'storage' as const, width: 896, height: 648 },
        logs: { title: 'System Logs', icon: 'chart' as const, width: 1008, height: 728 },
        apps: { title: 'Applications', icon: 'apps' as const, width: 896, height: 648 },
        settings: { title: 'Settings', icon: 'settings' as const, width: 1008, height: 728 },
        users: { title: 'Users', icon: 'users' as const, width: 872, height: 616 },
      }) satisfies Record<WindowId, { title: string; icon: IconName; width: number; height: number }>,
    [],
  )

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      const { data, error } = await fetchLayout()
      if (cancelled) return
      setLayout(data ?? mockLayoutData)
      if (error) {
        const message =
          error instanceof Error ? error.message : typeof error === 'string' ? error : 'Layout request failed'
        setLayoutError(message)
      } else {
        setLayoutError(null)
      }
    }
    load()
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadMetrics = async () => {
      const { data } = await fetchSystemMetrics({ lite: true })
      if (cancelled || !data) return
      setMetrics(data)
    }
    loadMetrics()
    const intervalId = window.setInterval(loadMetrics, 5000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadStatus = async () => {
      const { data } = await fetchSystemStatus()
      if (cancelled || !data) return
      setStatus(data)
    }
    loadStatus()
    const intervalId = window.setInterval(loadStatus, 15000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadApps = async () => {
      const { data, error } = await fetchAppsList()
      if (cancelled) return
      if (error) {
        const message = error instanceof Error ? error.message : typeof error === 'string' ? error : 'Apps request failed'
        setAppsError(message)
      } else {
        setAppsError(null)
      }
      setApps(data ?? mockDappStoreData)
    }
    loadApps()
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!logsWindowOpen) {
      return undefined
    }

    let cancelled = false
    const loadLogPeek = async () => {
      const { data, error } = await querySystemLogs({
        services: ['control-panel', 'cyfs_gateway', 'node_daemon'],
        direction: 'backward',
        limit: 16,
      })

      if (cancelled) return
      const message =
        error instanceof Error ? error.message : typeof error === 'string' ? error : error ? 'Log query failed' : null
      setLogPeekError(message)
      setLogPeek(data?.entries ?? null)
    }

    loadLogPeek()
    const intervalId = window.setInterval(loadLogPeek, 8000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [logsWindowOpen])

  const bringToFront = useCallback((id: WindowId) => {
    zCounterRef.current += 1
    const nextZ = zCounterRef.current
    setWindows((prev) =>
      prev.map((win) => (win.id === id ? { ...win, z: nextZ, minimized: false } : win)),
    )
  }, [])

  const openWindow = useCallback((id: WindowId) => {
    setMode('desktop')
    setWindows((prev) => {
      const existing = prev.find((win) => win.id === id)
      if (existing) {
        const next = prev.filter((win) => win.id !== id)
        zCounterRef.current += 1
        return [...next, { ...existing, minimized: false, z: zCounterRef.current }]
      }

      const spec = windowSpec[id]
      const viewportWidth = typeof window === 'undefined' ? 1200 : window.innerWidth
      const viewportHeight = typeof window === 'undefined' ? 800 : window.innerHeight
      const remembered = windowMemoryRef.current[id]
      const initialWidth = remembered?.width ?? spec.width
      const initialHeight = remembered?.height ?? spec.height
      const baseX = Math.round((viewportWidth - initialWidth) / 2)
      const baseY = Math.round((viewportHeight - initialHeight) / 2)
      const offset = prev.length * 24
      zCounterRef.current += 1
      const x = clamp(
        (remembered ? remembered.x : baseX + offset),
        WINDOW_MARGIN,
        Math.max(WINDOW_MARGIN, viewportWidth - initialWidth - WINDOW_MARGIN),
      )
      const y = clamp(
        (remembered ? remembered.y : baseY + offset),
        WINDOW_TOP_MARGIN,
        Math.max(WINDOW_TOP_MARGIN, viewportHeight - initialHeight - WINDOW_MARGIN),
      )

      const next: DesktopWindow = {
        id,
        title: spec.title,
        icon: spec.icon,
        x,
        y,
        width: initialWidth,
        height: initialHeight,
        z: zCounterRef.current,
        minimized: false,
      }
      return [...prev, next]
    })
  }, [windowSpec])

  const closeWindow = useCallback((id: WindowId) => {
    setWindows((prev) => {
      const closing = prev.find((win) => win.id === id)
      if (closing) {
        windowMemoryRef.current[id] = {
          x: closing.x,
          y: closing.y,
          width: closing.width,
          height: closing.height,
        }
      }
      return prev.filter((win) => win.id !== id)
    })
  }, [])

  const toggleMinimize = useCallback(
    (id: WindowId) =>
      setWindows((prev) => prev.map((win) => (win.id === id ? { ...win, minimized: !win.minimized } : win))),
    [],
  )

  const startWindowDrag = useCallback(
    (id: WindowId, originX: number, originY: number, width: number, height: number, event: PointerEvent<HTMLDivElement>) => {
      bringToFront(id)
      event.stopPropagation()

      if (event.button !== 0) {
        return
      }

      const target = event.target as HTMLElement | null
      const isControl = Boolean(
        target?.closest('[data-window-control="true"],button,a,input,textarea,select'),
      )
      if (isControl) {
        return
      }

      if (typeof window !== 'undefined' && window.innerWidth < 768) {
        return
      }

      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
      dragRef.current = {
        id,
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        originX,
        originY,
        width,
        height,
      }
    },
    [bringToFront],
  )

  const handleTitlePointerMove = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) {
      return
    }
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight
    const maxX = Math.max(WINDOW_MARGIN, viewportWidth - drag.width - WINDOW_MARGIN)
    const maxY = Math.max(WINDOW_TOP_MARGIN, viewportHeight - drag.height - WINDOW_MARGIN)
    const nextX = clamp(drag.originX + (event.clientX - drag.startX), WINDOW_MARGIN, maxX)
    const nextY = clamp(drag.originY + (event.clientY - drag.startY), WINDOW_TOP_MARGIN, maxY)

    dragPositionRef.current = { id: drag.id, x: nextX, y: nextY }
    if (dragRafRef.current !== null) {
      return
    }

    dragRafRef.current = window.requestAnimationFrame(() => {
      dragRafRef.current = null
      const dragPosition = dragPositionRef.current
      if (!dragPosition) {
        return
      }
      setWindows((prev) =>
        prev.map((win) =>
          win.id === dragPosition.id ? { ...win, x: dragPosition.x, y: dragPosition.y } : win,
        ),
      )
    })
  }, [])

  const handleTitlePointerUp = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) {
      return
    }

    if (dragRafRef.current !== null) {
      window.cancelAnimationFrame(dragRafRef.current)
      dragRafRef.current = null
      const dragPosition = dragPositionRef.current
      if (dragPosition) {
        setWindows((prev) =>
          prev.map((win) =>
            win.id === dragPosition.id ? { ...win, x: dragPosition.x, y: dragPosition.y } : win,
          ),
        )
      }
    }

    dragPositionRef.current = null
    try {
      event.currentTarget.releasePointerCapture(event.pointerId)
    } catch {
      // ignore
    }
    dragRef.current = null
  }, [])

  const startWindowResize = useCallback(
    (
      id: WindowId,
      edge: ResizeEdge,
      originX: number,
      originY: number,
      originWidth: number,
      originHeight: number,
      event: PointerEvent<HTMLDivElement>,
    ) => {
      bringToFront(id)
      event.stopPropagation()

      if (event.button !== 0) {
        return
      }

      if (typeof window !== 'undefined' && window.innerWidth < 768) {
        return
      }

      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
      resizeRef.current = {
        id,
        pointerId: event.pointerId,
        edge,
        startX: event.clientX,
        startY: event.clientY,
        originX,
        originY,
        originWidth,
        originHeight,
      }
    },
    [bringToFront],
  )

  const handleResizePointerMove = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const resize = resizeRef.current
    if (!resize || resize.pointerId !== event.pointerId) {
      return
    }

    const dx = event.clientX - resize.startX
    const dy = event.clientY - resize.startY
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight
    const rightEdge = resize.originX + resize.originWidth
    const bottomEdge = resize.originY + resize.originHeight

    let nextX = resize.originX
    let nextY = resize.originY
    let nextWidth = resize.originWidth
    let nextHeight = resize.originHeight

    if (resize.edge.includes('right')) {
      nextWidth = clamp(
        resize.originWidth + dx,
        MIN_WINDOW_WIDTH,
        Math.max(MIN_WINDOW_WIDTH, viewportWidth - resize.originX - WINDOW_MARGIN),
      )
    }

    if (resize.edge.includes('left')) {
      const maxLeft = rightEdge - MIN_WINDOW_WIDTH
      nextX = clamp(resize.originX + dx, WINDOW_MARGIN, Math.max(WINDOW_MARGIN, maxLeft))
      nextWidth = rightEdge - nextX
    }

    if (resize.edge.includes('bottom')) {
      nextHeight = clamp(
        resize.originHeight + dy,
        MIN_WINDOW_HEIGHT,
        Math.max(MIN_WINDOW_HEIGHT, viewportHeight - resize.originY - WINDOW_MARGIN),
      )
    }

    if (resize.edge.includes('top')) {
      const maxTop = bottomEdge - MIN_WINDOW_HEIGHT
      nextY = clamp(resize.originY + dy, WINDOW_TOP_MARGIN, Math.max(WINDOW_TOP_MARGIN, maxTop))
      nextHeight = bottomEdge - nextY
    }

    resizeRectRef.current = {
      id: resize.id,
      x: nextX,
      y: nextY,
      width: nextWidth,
      height: nextHeight,
    }

    if (resizeRafRef.current !== null) {
      return
    }

    resizeRafRef.current = window.requestAnimationFrame(() => {
      resizeRafRef.current = null
      const rect = resizeRectRef.current
      if (!rect) {
        return
      }
      setWindows((prev) =>
        prev.map((win) =>
          win.id === rect.id
            ? { ...win, x: rect.x, y: rect.y, width: rect.width, height: rect.height }
            : win,
        ),
      )
    })
  }, [])

  const handleResizePointerUp = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const resize = resizeRef.current
    if (!resize || resize.pointerId !== event.pointerId) {
      return
    }

    if (resizeRafRef.current !== null) {
      window.cancelAnimationFrame(resizeRafRef.current)
      resizeRafRef.current = null
      const rect = resizeRectRef.current
      if (rect) {
        setWindows((prev) =>
          prev.map((win) =>
            win.id === rect.id
              ? { ...win, x: rect.x, y: rect.y, width: rect.width, height: rect.height }
              : win,
          ),
        )
      }
    }

    resizeRectRef.current = null
    try {
      event.currentTarget.releasePointerCapture(event.pointerId)
    } catch {
      // ignore
    }
    resizeRef.current = null
  }, [])

  useEffect(() => {
    return () => {
      if (dragRafRef.current !== null) {
        window.cancelAnimationFrame(dragRafRef.current)
        dragRafRef.current = null
      }
      if (resizeRafRef.current !== null) {
        window.cancelAnimationFrame(resizeRafRef.current)
        resizeRafRef.current = null
      }
    }
  }, [])

  const systemPill = useMemo(() => {
    const state = status.state
    const labels: Record<SystemStatusResponse['state'], string> = {
      online: 'System Online',
      warning: 'Attention Needed',
      critical: 'Critical Alerts',
    }
    const tones: Record<SystemStatusResponse['state'], string> = {
      online: 'bg-emerald-500/20 text-emerald-50 ring-emerald-400/30',
      warning: 'bg-amber-500/20 text-amber-50 ring-amber-300/30',
      critical: 'bg-rose-500/20 text-rose-50 ring-rose-300/30',
    }
    const dot: Record<SystemStatusResponse['state'], string> = {
      online: 'bg-emerald-300',
      warning: 'bg-amber-300',
      critical: 'bg-rose-300',
    }
    return {
      label: labels[state],
      tone: tones[state],
      dot: dot[state],
    }
  }, [status.state])

  const desktopApps = useMemo(
    () =>
      [
        {
          id: 'monitor' as const,
          label: 'Monitor',
          icon: 'dashboard' as const,
          tile: 'bg-blue-500',
        },
        {
          id: 'storage' as const,
          label: 'Storage',
          icon: 'storage' as const,
          tile: 'bg-teal-500',
        },
        {
          id: 'logs' as const,
          label: 'System Logs',
          icon: 'chart' as const,
          tile: 'bg-orange-500',
        },
        {
          id: 'apps' as const,
          label: 'Apps',
          icon: 'apps' as const,
          tile: 'bg-sky-500',
        },
        {
          id: 'settings' as const,
          label: 'Settings',
          icon: 'settings' as const,
          tile: 'bg-gray-600',
        },
        {
          id: 'users' as const,
          label: 'Users',
          icon: 'users' as const,
          tile: 'bg-green-500',
        },
      ],
    [],
  )

  const wallpaperStyle = useMemo(
    () => ({
      backgroundImage:
        'radial-gradient(1200px circle at 14% 10%, rgba(255, 255, 255, 0.36) 0%, transparent 64%),\n'
        + 'radial-gradient(980px circle at 84% 16%, rgba(245, 158, 11, 0.28) 0%, transparent 62%),\n'
        + 'radial-gradient(860px circle at 72% 86%, rgba(125, 211, 252, 0.32) 0%, transparent 62%),\n'
        + 'linear-gradient(146deg, #2a8da2 0%, #86d9e4 54%, #2f8b9d 100%)',
    }),
    [],
  )

  const cpuPercent = Math.round(metrics.cpu?.usagePercent ?? 0)
  const memoryPercent = Math.round(metrics.memory?.usagePercent ?? 0)
  const diskPercent = Math.round(metrics.disk?.usagePercent ?? 0)
  const rxRate = metrics.network?.rxPerSec ?? 0
  const txRate = metrics.network?.txPerSec ?? 0

  const windowData = useMemo(
    () => ({
      metrics,
      status,
      layout,
      apps,
      appsError,
      logPeek,
      logPeekError,
      cpuPercent,
      memoryPercent,
      diskPercent,
      rxRate,
      txRate,
      navigateTo,
    }),
    [
      apps,
      appsError,
      cpuPercent,
      diskPercent,
      layout,
      logPeek,
      logPeekError,
      memoryPercent,
      metrics,
      navigateTo,
      rxRate,
      status,
      txRate,
    ],
  )

  const onNotificationsClick = useCallback(() => navigateTo('/notifications'), [navigateTo])
  const onNavigateSettings = useCallback(() => navigateTo('/settings'), [navigateTo])
  const goDesktop = useCallback(() => setMode('desktop'), [])
  const goJarvis = useCallback(() => setMode('jarvis'), [])
  const profileFirstName = useMemo(() => {
    const raw = layout.profile.name
    const first = raw.split(' ')[0]
    return first || raw
  }, [layout.profile.name])

  return (
    <div className="relative min-h-screen overflow-hidden text-white">
      <div className="absolute inset-0" style={wallpaperStyle} aria-hidden />
      <div
        className="absolute inset-0"
        style={{
          backgroundImage:
            'radial-gradient(760px circle at 20% 28%, rgba(255, 255, 255, 0.26) 0%, transparent 62%), radial-gradient(720px circle at 80% 60%, rgba(255, 255, 255, 0.2) 0%, transparent 64%), repeating-linear-gradient(135deg, rgba(255, 255, 255, 0.04) 0px, rgba(255, 255, 255, 0.04) 1px, transparent 1px, transparent 10px)',
        }}
        aria-hidden
      />

      <DesktopHeader
        layoutError={layoutError}
        profileAvatar={layout.profile.avatar}
        profileName={layout.profile.name}
        profileEmail={layout.profile.email}
        systemPill={systemPill}
        prefersReducedMotion={prefersReducedMotion}
        rxRate={rxRate}
        txRate={txRate}
        onNotificationsClick={onNotificationsClick}
      />

      <main className="relative z-10 min-h-screen">
        {mode === 'jarvis' ? (
          <JarvisView
            profileFirstName={profileFirstName}
            prefersReducedMotion={prefersReducedMotion}
            openWindow={openWindow}
            onNavigateSettings={onNavigateSettings}
          />
        ) : (
          <DesktopView
            desktopApps={desktopApps}
            prefersReducedMotion={prefersReducedMotion}
            openWindow={openWindow}
            statusState={status.state}
            cpuPercent={cpuPercent}
            memoryPercent={memoryPercent}
            diskPercent={diskPercent}
            appsError={appsError}
          />
        )}
      </main>

      <DesktopDock mode={mode} onDesktopClick={goDesktop} onJarvisClick={goJarvis} />

      <WindowLayer
        windows={windows}
        windowData={windowData}
        bringToFront={bringToFront}
        startWindowDrag={startWindowDrag}
        startWindowResize={startWindowResize}
        onTitlePointerMove={handleTitlePointerMove}
        onTitlePointerUp={handleTitlePointerUp}
        onResizePointerMove={handleResizePointerMove}
        onResizePointerUp={handleResizePointerUp}
        closeWindow={closeWindow}
        toggleMinimize={toggleMinimize}
      />
    </div>
  )
}

type DesktopHeaderProps = {
  layoutError: string | null
  profileAvatar: string
  profileName: string
  profileEmail: string
  systemPill: { label: string; tone: string; dot: string }
  prefersReducedMotion: boolean
  rxRate: number
  txRate: number
  onNotificationsClick: () => void
}

const DesktopHeader = memo((props: DesktopHeaderProps) => {
  const {
    layoutError,
    profileAvatar,
    profileName,
    profileEmail,
    systemPill,
    prefersReducedMotion,
    rxRate,
    txRate,
    onNotificationsClick,
  } = props

  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b border-white/10 bg-white/10 backdrop-blur-md">
      <div className="flex h-14 items-center justify-between px-4 sm:px-5 md:px-6">
        <div className="flex min-w-0 items-center gap-3">
          <div className="inline-flex size-10 items-center justify-center rounded-2xl bg-white/15 ring-1 ring-white/15">
            <span className="font-semibold">B</span>
          </div>
          <div className="min-w-0 leading-tight">
            <p className="truncate font-semibold tracking-tight">BuckyOS</p>
            <p className="truncate text-xs text-white/70">NAS Control Desktop</p>
          </div>
          <div
            className={`ml-2 hidden items-center gap-2 rounded-full px-3 py-1 text-xs font-semibold ring-1 md:inline-flex ${systemPill.tone}`}
          >
            <span
              className={`inline-flex size-2 rounded-full ${systemPill.dot} ${
                prefersReducedMotion ? '' : 'animate-pulse'
              }`}
              aria-hidden
            />
            {systemPill.label}
          </div>
        </div>

        <div className="flex items-center gap-3">
          {layoutError ? (
            <div className="hidden items-center gap-2 rounded-full border border-amber-200/25 bg-amber-500/15 px-3 py-1 text-xs text-amber-100 md:flex">
              <Icon name="alert" className="size-4" />
              Mock layout
            </div>
          ) : null}

          <button
            type="button"
            className="group relative inline-flex size-10 items-center justify-center rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/30"
            aria-label="Open notifications"
            onClick={onNotificationsClick}
          >
            <span className="inline-flex size-8 items-center justify-center rounded-xl bg-white/10 ring-1 ring-white/15 transition group-hover:bg-white/15">
              <Icon name="bell" className="size-5" />
            </span>
            <span className="absolute right-1.5 top-1.5 size-2 rounded-full bg-[var(--cp-accent)]" />
          </button>

          <div className="hidden items-center gap-2 rounded-xl bg-white/10 px-2.5 py-1.5 ring-1 ring-white/15 md:flex">
            <Icon name="network" className="size-4 text-white/80" />
            <p className="text-[11px] font-semibold text-white">
              Down {formatRate(rxRate)}
              <span className="mx-2 text-white/40" aria-hidden>
                |
              </span>
              Up {formatRate(txRate)}
            </p>
          </div>

          <div className="flex items-center gap-3">
            <img
              src={profileAvatar}
              alt={`${profileName} avatar`}
              className="size-8 rounded-full border border-white/15 object-cover"
            />
            <div className="hidden min-w-0 leading-tight sm:block">
              <p className="truncate text-sm font-semibold">{profileName}</p>
              <p className="truncate text-xs text-white/70">{profileEmail}</p>
            </div>
          </div>
        </div>
      </div>
    </header>
  )
})
DesktopHeader.displayName = 'DesktopHeader'

type DesktopDockProps = {
  mode: DesktopMode
  onDesktopClick: () => void
  onJarvisClick: () => void
}

const DesktopDock = memo((props: DesktopDockProps) => {
  const { mode, onDesktopClick, onJarvisClick } = props
  return (
    <div className="fixed bottom-6 left-1/2 z-50 -translate-x-1/2 px-4">
      <div className="flex items-center gap-2 rounded-full border border-white/10 bg-black/55 p-2 backdrop-blur-xl shadow-[0_18px_60px_-40px_rgba(0,0,0,0.85)]">
        <button
          type="button"
          onClick={onDesktopClick}
          className={`inline-flex items-center gap-2 rounded-full px-4 py-2 text-sm font-semibold transition ${
            mode === 'desktop' ? 'bg-white text-slate-900 shadow' : 'text-white/85 hover:bg-white/10'
          }`}
        >
          <Icon name="desktop" className="size-4" />
          Desktop
        </button>
        <button
          type="button"
          onClick={onJarvisClick}
          className={`inline-flex items-center gap-2 rounded-full px-4 py-2 text-sm font-semibold transition ${
            mode === 'jarvis'
              ? 'bg-gradient-to-r from-sky-400 to-emerald-300 text-slate-900 shadow'
              : 'text-white/85 hover:bg-white/10'
          }`}
        >
          <Icon name="spark" className="size-4" />
          Jarvis
        </button>
      </div>
    </div>
  )
})
DesktopDock.displayName = 'DesktopDock'

type JarvisViewProps = {
  profileFirstName: string
  prefersReducedMotion: boolean
  openWindow: (id: WindowId) => void
  onNavigateSettings: () => void
}

const JarvisView = memo((props: JarvisViewProps) => {
  const { profileFirstName, prefersReducedMotion, openWindow, onNavigateSettings } = props
  return (
    <section className="mx-auto flex max-w-4xl flex-col items-center px-4 pb-28 pt-24 text-center">
      <div
        className={`mb-8 inline-flex size-44 items-center justify-center rounded-full bg-gradient-to-br from-emerald-400/30 via-sky-400/15 to-amber-300/25 ring-1 ring-white/10 ${
          prefersReducedMotion ? '' : 'motion-safe:animate-pulse'
        }`}
        aria-hidden
      >
        <div className="size-24 rounded-full bg-white/10 ring-1 ring-white/15" />
      </div>
      <p className="text-sm font-semibold tracking-wide text-white/70">Assistant Console</p>
      <h1 className="mt-2 text-4xl font-semibold tracking-tight md:text-5xl">
        Good {new Date().getHours() < 12 ? 'morning' : 'evening'}, {profileFirstName}.
      </h1>
      <p className="mt-4 max-w-2xl text-base text-white/75">
        Ask for diagnostics, open tools, or jump straight into logs. (Input is UI-only for now.)
      </p>

      <div className="mt-8 w-full max-w-2xl rounded-3xl border border-white/10 bg-white/10 p-3 backdrop-blur-xl">
        <div className="flex items-center gap-3 rounded-2xl bg-black/25 px-4 py-3 ring-1 ring-white/10">
          <Icon name="spark" className="size-5 text-white/80" />
          <input
            className="h-10 flex-1 bg-transparent text-sm text-white placeholder:text-white/50 focus:outline-none"
            placeholder="Ask Jarvis anything..."
          />
          <button
            type="button"
            className="inline-flex h-10 items-center gap-2 rounded-full bg-[var(--cp-primary)] px-4 text-sm font-semibold text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
          >
            Send
          </button>
        </div>
        <div className="mt-3 flex flex-wrap justify-center gap-2 px-2 pb-1">
          <button
            type="button"
            onClick={() => openWindow('logs')}
            className="rounded-full border border-white/10 bg-white/10 px-4 py-2 text-xs font-semibold text-white/90 transition hover:bg-white/15"
          >
            Check system logs
          </button>
          <button
            type="button"
            onClick={() => openWindow('monitor')}
            className="rounded-full border border-white/10 bg-white/10 px-4 py-2 text-xs font-semibold text-white/90 transition hover:bg-white/15"
          >
            System monitor
          </button>
          <button
            type="button"
            onClick={() => openWindow('storage')}
            className="rounded-full border border-white/10 bg-white/10 px-4 py-2 text-xs font-semibold text-white/90 transition hover:bg-white/15"
          >
            Optimize storage
          </button>
          <button
            type="button"
            onClick={onNavigateSettings}
            className="rounded-full border border-white/10 bg-white/10 px-4 py-2 text-xs font-semibold text-white/90 transition hover:bg-white/15"
          >
            Network diagnostics
          </button>
        </div>
      </div>
    </section>
  )
})
JarvisView.displayName = 'JarvisView'

type DesktopViewProps = {
  desktopApps: { id: WindowId; label: string; icon: IconName; tile: string }[]
  prefersReducedMotion: boolean
  openWindow: (id: WindowId) => void
  statusState: SystemStatusResponse['state']
  cpuPercent: number
  memoryPercent: number
  diskPercent: number
  appsError: string | null
}

const DesktopView = memo((props: DesktopViewProps) => {
  const {
    desktopApps,
    prefersReducedMotion,
    openWindow,
    statusState,
    cpuPercent,
    memoryPercent,
    diskPercent,
    appsError,
  } = props

  return (
    <section className="relative min-h-screen px-5 pb-28 pt-28 sm:px-6 md:px-8">
      <div className="mx-auto grid max-w-7xl grid-cols-1 gap-10 md:grid-cols-[1fr_320px]">
        <div className="grid grid-cols-3 content-start justify-items-center gap-x-5 gap-y-4 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 xl:grid-cols-10">
          {desktopApps.map((app, index) => (
            <button
              key={app.id}
              type="button"
              onClick={() => openWindow(app.id)}
              className={`group flex w-[86px] flex-col items-center gap-2 rounded-xl p-2 transition-colors hover:bg-white/10 focus-visible:bg-white/15 focus-visible:outline-none ${
                index === 0 ? 'md:col-start-2 lg:col-start-3 xl:col-start-4' : ''
              }`}
            >
              <div
                className={`flex h-16 w-16 items-center justify-center rounded-2xl text-white shadow-lg ${app.tile} ${
                  prefersReducedMotion ? '' : 'group-hover:scale-105'
                } transition-transform duration-200`}
              >
                <Icon name={app.icon} className="size-7" />
              </div>
              <span
                className="w-full text-center text-xs font-medium text-white drop-shadow-md"
                style={{
                  display: '-webkit-box',
                  WebkitBoxOrient: 'vertical',
                  WebkitLineClamp: 2,
                  overflow: 'hidden',
                }}
              >
                {app.label}
              </span>
            </button>
          ))}
        </div>

        <aside className="hidden md:block">
          <div className="sticky top-24 space-y-4">
            <ClockCard />

            <div className="rounded-2xl border border-white/10 bg-white/10 p-4 text-white shadow-xl backdrop-blur-md">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-sm font-medium opacity-80">System Health</span>
                <div
                  className={`size-2 rounded-full ${
                    statusState === 'online'
                      ? 'bg-green-400'
                      : statusState === 'warning'
                        ? 'bg-amber-300'
                        : 'bg-rose-300'
                  } ${prefersReducedMotion ? '' : 'animate-pulse'}`}
                  aria-hidden
                />
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-white/10">
                <div
                  className={`h-full ${
                    statusState === 'online'
                      ? 'bg-green-400'
                      : statusState === 'warning'
                        ? 'bg-amber-300'
                        : 'bg-rose-300'
                  }`}
                  style={{ width: `${clamp(100 - Math.round((cpuPercent + memoryPercent + diskPercent) / 3), 0, 100)}%` }}
                />
              </div>
              <div className="mt-2 flex justify-between text-xs opacity-60">
                <span>CPU: {cpuPercent}%</span>
                <span>RAM: {memoryPercent}%</span>
              </div>
              {appsError ? (
                <p className="mt-2 text-[11px] text-amber-100/90">Apps unavailable: {appsError}</p>
              ) : null}
            </div>
          </div>
        </aside>
      </div>
    </section>
  )
})
DesktopView.displayName = 'DesktopView'

type WindowData = {
  metrics: SystemMetrics
  status: SystemStatusResponse
  layout: RootLayoutData
  apps: DappCard[]
  appsError: string | null
  logPeek: SystemLogEntry[] | null
  logPeekError: string | null
  cpuPercent: number
  memoryPercent: number
  diskPercent: number
  rxRate: number
  txRate: number
  navigateTo: (to: string) => void
}

const ClockCard = memo(() => {
  const [clock, setClock] = useState(() => {
    const date = new Date()
    return {
      time: date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
      day: date.toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' }),
    }
  })

  useEffect(() => {
    const tick = () => {
      const date = new Date()
      setClock({
        time: date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
        day: date.toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' }),
      })
    }

    const intervalId = window.setInterval(tick, 1000)
    return () => window.clearInterval(intervalId)
  }, [])

  return (
    <div className="rounded-2xl border border-white/10 bg-white/10 p-4 text-white shadow-xl backdrop-blur-md">
      <div className="text-3xl font-light">{clock.time}</div>
      <div className="text-sm opacity-70">{clock.day}</div>
    </div>
  )
})
ClockCard.displayName = 'ClockCard'

type WindowLayerProps = {
  windows: DesktopWindow[]
  windowData: WindowData
  bringToFront: (id: WindowId) => void
  startWindowDrag: (
    id: WindowId,
    originX: number,
    originY: number,
    width: number,
    height: number,
    event: PointerEvent<HTMLDivElement>,
  ) => void
  startWindowResize: (
    id: WindowId,
    edge: ResizeEdge,
    originX: number,
    originY: number,
    originWidth: number,
    originHeight: number,
    event: PointerEvent<HTMLDivElement>,
  ) => void
  onTitlePointerMove: (event: PointerEvent<HTMLDivElement>) => void
  onTitlePointerUp: (event: PointerEvent<HTMLDivElement>) => void
  onResizePointerMove: (event: PointerEvent<HTMLDivElement>) => void
  onResizePointerUp: (event: PointerEvent<HTMLDivElement>) => void
  closeWindow: (id: WindowId) => void
  toggleMinimize: (id: WindowId) => void
}

const getWindowFullPath = (id: WindowId) => {
  if (id === 'monitor') return '/monitor'
  if (id === 'storage') return '/storage'
  if (id === 'logs') return '/system-logs'
  if (id === 'apps') return '/dapps'
  if (id === 'settings') return '/settings'
  return '/users'
}

const WindowLayer = memo((props: WindowLayerProps) => {
  const {
    windows,
    windowData,
    bringToFront,
    startWindowDrag,
    startWindowResize,
    onTitlePointerMove,
    onTitlePointerUp,
    onResizePointerMove,
    onResizePointerUp,
    closeWindow,
    toggleMinimize,
  } = props

  return (
    <div className="pointer-events-none fixed inset-0 z-40">
      {windows
        .filter((win) => !win.minimized)
        .sort((a, b) => a.z - b.z)
        .map((win) => (
          <WindowFrame
            key={win.id}
            win={win}
            windowData={windowData}
            bringToFront={bringToFront}
            startWindowDrag={startWindowDrag}
            startWindowResize={startWindowResize}
            onTitlePointerMove={onTitlePointerMove}
            onTitlePointerUp={onTitlePointerUp}
            onResizePointerMove={onResizePointerMove}
            onResizePointerUp={onResizePointerUp}
            closeWindow={closeWindow}
            toggleMinimize={toggleMinimize}
          />
        ))}
    </div>
  )
})
WindowLayer.displayName = 'WindowLayer'

type WindowFrameProps = {
  win: DesktopWindow
  windowData: WindowData
  bringToFront: (id: WindowId) => void
  startWindowDrag: (
    id: WindowId,
    originX: number,
    originY: number,
    width: number,
    height: number,
    event: PointerEvent<HTMLDivElement>,
  ) => void
  startWindowResize: (
    id: WindowId,
    edge: ResizeEdge,
    originX: number,
    originY: number,
    originWidth: number,
    originHeight: number,
    event: PointerEvent<HTMLDivElement>,
  ) => void
  onTitlePointerMove: (event: PointerEvent<HTMLDivElement>) => void
  onTitlePointerUp: (event: PointerEvent<HTMLDivElement>) => void
  onResizePointerMove: (event: PointerEvent<HTMLDivElement>) => void
  onResizePointerUp: (event: PointerEvent<HTMLDivElement>) => void
  closeWindow: (id: WindowId) => void
  toggleMinimize: (id: WindowId) => void
}

const WindowFrame = memo((props: WindowFrameProps) => {
  const {
    win,
    windowData,
    bringToFront,
    startWindowDrag,
    startWindowResize,
    onTitlePointerMove,
    onTitlePointerUp,
    onResizePointerMove,
    onResizePointerUp,
    closeWindow,
    toggleMinimize,
  } = props

  const resizeHandles: { edge: ResizeEdge; className: string }[] = [
    { edge: 'top', className: 'left-3 right-3 top-0 h-2 cursor-n-resize' },
    { edge: 'right', className: 'bottom-3 right-0 top-3 w-2 cursor-e-resize' },
    { edge: 'bottom', className: 'bottom-0 left-3 right-3 h-2 cursor-s-resize' },
    { edge: 'left', className: 'bottom-3 left-0 top-3 w-2 cursor-w-resize' },
    { edge: 'top-left', className: 'left-0 top-0 size-3 cursor-nw-resize' },
    { edge: 'top-right', className: 'right-0 top-0 size-3 cursor-ne-resize' },
    { edge: 'bottom-left', className: 'bottom-0 left-0 size-3 cursor-sw-resize' },
    { edge: 'bottom-right', className: 'bottom-0 right-0 size-3 cursor-se-resize' },
  ]

  return (
    <div
      className="pointer-events-auto fixed"
      style={{
        left: 0,
        top: 0,
        transform: `translate3d(${win.x}px, ${win.y}px, 0)`,
        willChange: 'transform',
        width: win.width,
        height: win.height,
        zIndex: win.z,
      }}
      onPointerDown={() => bringToFront(win.id)}
    >
      <div className="flex h-full flex-col overflow-hidden rounded-3xl border border-white/10 bg-white/85 text-[var(--cp-ink)] shadow-2xl shadow-black/40 backdrop-blur">
        <div
          className="flex items-center justify-between gap-3 border-b border-[rgba(215,225,223,0.65)] bg-white/70 px-4 py-3"
          onPointerDown={(event) => startWindowDrag(win.id, win.x, win.y, win.width, win.height, event)}
          onPointerMove={onTitlePointerMove}
          onPointerUp={onTitlePointerUp}
          onPointerCancel={onTitlePointerUp}
          style={{ touchAction: 'none' }}
        >
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2">
              <button
                type="button"
                data-window-control="true"
                className="size-3 rounded-full bg-rose-500/90 ring-1 ring-black/10"
                aria-label={`Close ${win.title}`}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                  event.stopPropagation()
                  closeWindow(win.id)
                }}
              />
              <button
                type="button"
                data-window-control="true"
                className="size-3 rounded-full bg-amber-400/90 ring-1 ring-black/10"
                aria-label={`Minimize ${win.title}`}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                  event.stopPropagation()
                  toggleMinimize(win.id)
                }}
              />
              <span className="size-3 rounded-full bg-emerald-400/80 ring-1 ring-black/10" />
            </div>
            <div className="flex items-center gap-2">
              <Icon name={win.icon} className="size-4 text-[var(--cp-muted)]" />
              <p className="text-sm font-semibold tracking-tight text-[var(--cp-ink)]">{win.title}</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Link
              data-window-control="true"
              to={getWindowFullPath(win.id)}
              className="rounded-full border border-[var(--cp-border)] bg-white px-3 py-1 text-xs font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
              onPointerDown={(event) => event.stopPropagation()}
            >
              Open full
            </Link>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-auto bg-[var(--cp-surface-muted)] p-4">
          <WindowBody id={win.id} data={windowData} />
        </div>
      </div>

      {resizeHandles.map((handle) => (
        <div
          key={handle.edge}
          className={`absolute ${handle.className}`}
          onPointerDown={(event) =>
            startWindowResize(win.id, handle.edge, win.x, win.y, win.width, win.height, event)
          }
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
          onPointerCancel={onResizePointerUp}
          style={{ touchAction: 'none' }}
        />
      ))}
    </div>
  )
})
WindowFrame.displayName = 'WindowFrame'

type WindowBodyProps = {
  id: WindowId
  data: WindowData
}

const WindowBody = memo((props: WindowBodyProps) => {
  const { id, data } = props
  const {
    metrics,
    status,
    layout,
    apps,
    logPeek,
    logPeekError,
    cpuPercent,
    memoryPercent,
    diskPercent,
    rxRate,
    txRate,
    navigateTo,
  } = data

  if (id === 'monitor') {
    const resourceTimeline = (
      metrics.resourceTimeline?.length
        ? metrics.resourceTimeline
        : [{ time: 'now', cpu: cpuPercent, memory: memoryPercent }]
    ).slice(-8)
    const networkTimeline = (
      metrics.networkTimeline?.length
        ? metrics.networkTimeline
        : [{ time: 'now', rx: rxRate, tx: txRate }]
    ).slice(-8)

    return (
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">CPU</p>
          <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">{cpuPercent}%</p>
          <div className="mt-3 h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
            <div
              className="h-full rounded-full bg-[var(--cp-primary)]"
              style={{ width: `${clamp(cpuPercent, 0, 100)}%` }}
            />
          </div>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Memory</p>
          <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">{memoryPercent}%</p>
          <div className="mt-3 h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
            <div
              className="h-full rounded-full bg-[var(--cp-accent)]"
              style={{ width: `${clamp(memoryPercent, 0, 100)}%` }}
            />
          </div>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Storage</p>
          <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">{diskPercent}%</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">
            {metrics.disk?.usedGb?.toFixed(0) ?? '-'} / {metrics.disk?.totalGb?.toFixed(0) ?? '-'} GB
          </p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Network</p>
          <p className="mt-2 text-lg font-semibold text-[var(--cp-ink)]">{formatRate(rxRate)}</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">Up {formatRate(txRate)}</p>
        </div>
        <div className="sm:col-span-2 rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="mb-3 flex items-center justify-between">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">CPU / Memory trend</p>
            <span className="text-[11px] text-[var(--cp-muted)]">Last {resourceTimeline.length} points</span>
          </div>
          <ResourceTrendChart timeline={resourceTimeline} height={180} />
        </div>
        <div className="sm:col-span-2 rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="mb-3 flex items-center justify-between">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Network throughput trend</p>
            <span className="text-[11px] text-[var(--cp-muted)]">MB/s</span>
          </div>
          <NetworkTrendChart timeline={networkTimeline} height={180} />
        </div>
        <div className="sm:col-span-2 rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="flex items-center justify-between">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">System status</p>
            <span
              className={`rounded-full px-3 py-1 text-[11px] font-semibold uppercase tracking-wide ${
                status.state === 'online'
                  ? 'bg-emerald-100 text-emerald-700'
                  : status.state === 'warning'
                    ? 'bg-amber-100 text-amber-700'
                    : 'bg-rose-100 text-rose-700'
              }`}
            >
              {status.state}
            </span>
          </div>
          {status.warnings?.length ? (
            <div className="mt-3 space-y-2">
              {status.warnings.slice(0, 3).map((warning) => (
                <div
                  key={`${warning.label}-${warning.message}`}
                  className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs"
                >
                  <p className="font-semibold text-[var(--cp-ink)]">{warning.label}</p>
                  <p className="text-[var(--cp-muted)]">{warning.message}</p>
                </div>
              ))}
            </div>
          ) : (
            <p className="mt-2 text-xs text-[var(--cp-muted)]">No active warnings.</p>
          )}
        </div>
      </div>
    )
  }

  if (id === 'logs') {
    return (
      <div className="space-y-3">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm">
          <p className="font-semibold text-[var(--cp-ink)]">Live preview</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">Showing a compact tail across core services.</p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white">
          {logPeekError ? (
            <div className="px-4 py-3 text-xs text-amber-800" style={{ background: '#fffbeb' }}>
              Using mock data: {logPeekError}
            </div>
          ) : null}
          {logPeek?.length ? (
            <div className="max-h-[360px] overflow-auto p-2">
              {logPeek.slice(0, 12).map((entry) => (
                <div
                  key={`${entry.file}-${entry.timestamp}-${entry.message}`}
                  className="rounded-xl px-3 py-2 text-xs"
                  style={{ borderBottom: '1px solid rgba(215, 225, 223, 0.35)' }}
                >
                  <p className="font-mono text-[11px] text-[var(--cp-muted)]">
                    {entry.timestamp} - {entry.service} - {entry.file}
                  </p>
                  <p className="mt-1 font-mono text-[13px] text-[var(--cp-ink)]">{entry.message || entry.raw}</p>
                </div>
              ))}
            </div>
          ) : (
            <div className="px-4 py-6 text-sm text-[var(--cp-muted)]">No log entries yet.</div>
          )}
        </div>
      </div>
    )
  }

  if (id === 'apps') {
    return (
      <div className="space-y-3">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm">
          <p className="font-semibold text-[var(--cp-ink)]">Installed services</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">Quick glance at apps known to the system config.</p>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {(apps.length ? apps : []).slice(0, 6).map((app) => (
            <div key={app.name} className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-3">
                  <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-surface-muted)] text-[var(--cp-primary-strong)]">
                    <Icon name={app.icon} className="size-4" />
                  </span>
                  <div className="min-w-0">
                    <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{app.name}</p>
                    <p className="text-xs text-[var(--cp-muted)]">{app.category}</p>
                  </div>
                </div>
                <span className="rounded-full bg-emerald-100 px-2.5 py-1 text-[11px] font-semibold text-emerald-700">
                  {app.status}
                </span>
              </div>
              <p className="mt-2 text-xs text-[var(--cp-muted)]">v{app.version}</p>
            </div>
          ))}
        </div>
        {!apps.length ? (
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-6 text-sm text-[var(--cp-muted)]">
            No apps available yet.
          </div>
        ) : null}
      </div>
    )
  }

  if (id === 'storage') {
    return (
      <div className="space-y-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="flex items-center justify-between">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Storage usage</p>
            <span className="rounded-full bg-[var(--cp-surface-muted)] px-3 py-1 text-xs font-semibold text-[var(--cp-ink)]">
              {metrics.disk?.usedGb?.toFixed(0) ?? '-'} / {metrics.disk?.totalGb?.toFixed(0) ?? '-'} GB
            </span>
          </div>
          <div className="mt-3 h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
            <div
              className="h-full rounded-full bg-[var(--cp-primary)]"
              style={{ width: `${clamp(diskPercent, 0, 100)}%` }}
            />
          </div>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Disks</p>
          <div className="mt-3 space-y-3">
            {(metrics.disk?.disks ?? []).slice(0, 5).map((disk) => {
              const usagePercent = Math.round(
                disk.usagePercent ?? (disk.totalGb ? (disk.usedGb / disk.totalGb) * 100 : 0),
              )
              return (
                <div
                  key={`${disk.mount}-${disk.label}`}
                  className="rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{disk.label}</p>
                      <p className="truncate text-xs text-[var(--cp-muted)]">{disk.mount}</p>
                    </div>
                    <span className="text-xs font-semibold text-[var(--cp-ink)]">{usagePercent}%</span>
                  </div>
                  <div className="mt-2 h-2 overflow-hidden rounded-full bg-white">
                    <div
                      className="h-full rounded-full bg-[var(--cp-primary)]"
                      style={{ width: `${clamp(usagePercent, 0, 100)}%` }}
                    />
                  </div>
                  <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--cp-muted)]">
                    <span>
                      {disk.usedGb.toFixed(1)} / {disk.totalGb.toFixed(1)} GB
                    </span>
                    <span>{disk.fs ?? 'unknown'}</span>
                  </div>
                </div>
              )
            })}
            {!(metrics.disk?.disks ?? []).length ? (
              <p className="text-sm text-[var(--cp-muted)]">No disk details available.</p>
            ) : null}
          </div>
        </div>
      </div>
    )
  }

  if (id === 'settings') {
    return (
      <div className="space-y-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">System details</p>
          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <div className="rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">CPU</p>
              <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{metrics.cpu?.model ?? 'Unknown CPU'}</p>
            </div>
            <div className="rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Network</p>
              <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">Down {formatRate(rxRate)}</p>
              <p className="text-xs text-[var(--cp-muted)]">Up {formatRate(txRate)}</p>
            </div>
          </div>
          <div className="mt-3 rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Identity</p>
            <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{layout.profile.email}</p>
            <p className="text-xs text-[var(--cp-muted)]">Policy owner: {layout.profile.name}</p>
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Settings modules</p>
            <button
              type="button"
              onClick={() => navigateTo('/settings')}
              className="rounded-full bg-[var(--cp-primary)] px-3 py-1.5 text-[11px] font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
            >
              Open full settings
            </button>
          </div>
          <div className="mt-3 space-y-2">
            {SETTINGS_MODULE_PREVIEW.map((module) => (
              <div
                key={module.name}
                className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
              >
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <p className="text-sm font-semibold text-[var(--cp-ink)]">{module.name}</p>
                    <p className="text-xs text-[var(--cp-muted)]">{module.detail}</p>
                  </div>
                  <span
                    className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide ${
                      module.state === 'ready'
                        ? 'bg-emerald-100 text-emerald-700'
                        : module.state === 'review'
                          ? 'bg-amber-100 text-amber-700'
                          : 'bg-slate-100 text-slate-700'
                    }`}
                  >
                    {module.state}
                  </span>
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">Policy baseline</p>
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            {[
              'MFA required for Owner/Admin roles',
              'Session timeout at 12h idle window',
              'Critical alerts routed to admin channel',
              'Nightly backup validation at 04:00',
            ].map((policy) => (
              <div
                key={policy}
                className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs text-[var(--cp-ink)]"
              >
                {policy}
              </div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Team and access overview</p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">Role assignments and invite queue snapshot.</p>
          </div>
          <button
            type="button"
            onClick={() => navigateTo('/users')}
            className="rounded-full bg-[var(--cp-primary)] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
          >
            Open user management
          </button>
        </div>

        <div className="mt-4 grid gap-2 sm:grid-cols-3">
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Members</p>
            <p className="mt-1 text-xl font-semibold text-[var(--cp-ink)]">{USER_TEAM_PREVIEW.length}</p>
          </div>
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Pending invites</p>
            <p className="mt-1 text-xl font-semibold text-[var(--cp-ink)]">
              {USER_TEAM_PREVIEW.filter((member) => member.status === 'pending').length}
            </p>
          </div>
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Admin coverage</p>
            <p className="mt-1 text-xl font-semibold text-[var(--cp-ink)]">
              {USER_TEAM_PREVIEW.filter((member) => member.role === 'Owner' || member.role === 'Admin').length}
            </p>
          </div>
        </div>

        <div className="mt-3 space-y-2">
          {USER_TEAM_PREVIEW.map((member) => (
            <div
              key={member.name}
              className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
            >
              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-sm font-semibold text-[var(--cp-ink)]">{member.name}</p>
                  <p className="text-xs text-[var(--cp-muted)]">
                    {member.role} - {member.group}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <span className="rounded-full border border-[var(--cp-border)] bg-white px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-ink)]">
                    {member.access}
                  </span>
                  <span
                    className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                      member.status === 'active'
                        ? 'bg-emerald-100 text-emerald-700'
                        : member.status === 'pending'
                          ? 'bg-amber-100 text-amber-700'
                          : 'bg-rose-100 text-rose-700'
                    }`}
                  >
                    {member.status}
                  </span>
                </div>
              </div>
            </div>
          ))}
        </div>

        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => navigateTo('/settings')}
            className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-2 text-xs font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
          >
            Review policies
          </button>
        </div>
      </div>
    </div>
  )
})
WindowBody.displayName = 'WindowBody'

export default DesktopHomePage
