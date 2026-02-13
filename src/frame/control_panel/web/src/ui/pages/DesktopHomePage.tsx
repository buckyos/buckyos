import type { MouseEvent, PointerEvent } from 'react'
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import {
  fetchAppsList,
  fetchContainerOverview,
  fetchGatewayFile,
  fetchGatewayOverview,
  fetchLayout,
  fetchNetworkOverview,
  fetchSystemMetrics,
  fetchSystemOverview,
  fetchSystemStatus,
  fetchZoneOverview,
  mockDappStoreData,
  mockLayoutData,
  mockSystemMetrics,
  mockSystemStatus,
  querySystemLogs,
} from '@/api'
import ContainerOverviewPanel from '../components/ContainerOverviewPanel'
import NetworkOverviewPanel from '../components/NetworkOverviewPanel'
import { NetworkTrendChart, ResourceTrendChart } from '../components/MonitorTrendCharts'
import StorageDiskStatusPanel from '../components/StorageDiskStatusPanel'
import StorageHealthSignalsPanel from '../components/StorageHealthSignalsPanel'
import SystemConfigTreeViewer from '../components/SystemConfigTreeViewer'
import UserPatternAvatar from '../components/UserPatternAvatar'
import usePrefersReducedMotion from '../charts/usePrefersReducedMotion'
import Icon from '../icons'

type DesktopMode = 'desktop' | 'jarvis'

type WindowId = 'monitor' | 'network' | 'containers' | 'storage' | 'logs' | 'apps' | 'settings' | 'users'

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
  maximized: boolean
  restoreRect?: {
    x: number
    y: number
    width: number
    height: number
  }
}

type AccessModePill = {
  label: string
  tone: string
  dot: string
  description: string
  host: string
}

type SettingsMenuKey =
  | 'general'
  | 'zone-manager'
  | 'sys-manager'
  | 'gateway-manager'
  | 'storage'
  | 'permissions'
  | 'software-update'

type UserGroupKey = 'admin' | 'family' | 'guests'

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

const formatUptime = (seconds: number) => {
  const safeSeconds = Math.max(0, Math.floor(seconds))
  const days = Math.floor(safeSeconds / 86400)
  const hours = Math.floor((safeSeconds % 86400) / 3600)
  const minutes = Math.floor((safeSeconds % 3600) / 60)
  const parts: string[] = []
  if (days) parts.push(`${days}d`)
  if (hours || days) parts.push(`${hours}h`)
  parts.push(`${minutes}m`)
  return parts.join(' ')
}

const WINDOW_MARGIN = 24
const WINDOW_TOP_MARGIN = 80
const MIN_WINDOW_WIDTH = 420
const MIN_WINDOW_HEIGHT = 280
const DESKTOP_HEADER_HEIGHT = 56
const DESKTOP_DOCK_RESERVED_HEIGHT = 92
const MAXIMIZED_SIDE_MARGIN = 10

const getMaximizedRect = () => {
  const viewportWidth = typeof window === 'undefined' ? 1200 : window.innerWidth
  const viewportHeight = typeof window === 'undefined' ? 800 : window.innerHeight
  const x = MAXIMIZED_SIDE_MARGIN
  const y = DESKTOP_HEADER_HEIGHT
  const width = Math.max(320, viewportWidth - MAXIMIZED_SIDE_MARGIN * 2)
  const height = Math.max(220, viewportHeight - y - DESKTOP_DOCK_RESERVED_HEIGHT)
  return { x, y, width, height }
}

const USER_WINDOW_GROUPS: { id: UserGroupKey; label: string; description: string }[] = [
  { id: 'admin', label: 'admin', description: 'Owner and system operators' },
  { id: 'family', label: 'family', description: 'Trusted home collaborators' },
  { id: 'guests', label: 'guests', description: 'Limited temporary access' },
]

const SETTINGS_WINDOW_MENU: { id: SettingsMenuKey; label: string; description: string }[] = [
  { id: 'general', label: 'General', description: 'Identity and basic preferences' },
  { id: 'zone-manager', label: 'Zone Manager', description: 'Zone naming and topology' },
  { id: 'sys-manager', label: 'Sys Manager', description: 'Runtime and service health' },
  { id: 'gateway-manager', label: 'Gateway Manager', description: 'Ingress and access mode' },
  { id: 'storage', label: 'Storage', description: 'Capacity and disk health' },
  { id: 'permissions', label: 'Permissions', description: 'Role and policy baseline' },
  { id: 'software-update', label: 'Software Update', description: 'Version and release channel' },
]

const SETTINGS_POLICY_BASELINE = [
  'MFA required for Owner/Admin roles',
  'Session timeout at 12h idle window',
  'Critical alerts routed to admin channel',
  'Nightly backup validation at 04:00',
]

const DesktopHomePage = () => {
  const navigate = useNavigate()
  const navigateTo = useCallback((to: string) => navigate(to), [navigate])
  const prefersReducedMotion = usePrefersReducedMotion()

  const [mode, setMode] = useState<DesktopMode>('desktop')
  const [layout, setLayout] = useState<RootLayoutData>(mockLayoutData)
  const [layoutError, setLayoutError] = useState<string | null>(null)
  const [overview, setOverview] = useState<SystemOverview | null>(null)
  const [overviewError, setOverviewError] = useState<string | null>(null)
  const [metrics, setMetrics] = useState<SystemMetrics>(mockSystemMetrics)
  const [status, setStatus] = useState<SystemStatusResponse>(mockSystemStatus)
  const [apps, setApps] = useState<DappCard[]>([])
  const [appsError, setAppsError] = useState<string | null>(null)
  const [networkOverview, setNetworkOverview] = useState<NetworkOverview | null>(null)
  const [networkError, setNetworkError] = useState<string | null>(null)
  const [containerOverview, setContainerOverview] = useState<ContainerOverview | null>(null)
  const [containerError, setContainerError] = useState<string | null>(null)
  const [zoneOverview, setZoneOverview] = useState<ZoneOverview | null>(null)
  const [zoneError, setZoneError] = useState<string | null>(null)
  const [gatewayOverview, setGatewayOverview] = useState<GatewayOverview | null>(null)
  const [gatewayError, setGatewayError] = useState<string | null>(null)
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
        network: { title: 'Network Monitor', icon: 'network' as const, width: 980, height: 700 },
        containers: { title: 'Container Manager', icon: 'container' as const, width: 980, height: 700 },
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
    const loadOverview = async () => {
      const { data, error } = await fetchSystemOverview()
      if (cancelled) return
      setOverview(data)
      if (error) {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : 'System overview request failed'
        setOverviewError(message)
      } else {
        setOverviewError(null)
      }
    }

    loadOverview()
    const intervalId = window.setInterval(loadOverview, 30000)
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
    let cancelled = false
    const loadNetwork = async () => {
      const { data, error } = await fetchNetworkOverview()
      if (cancelled) return
      setNetworkOverview(data)
      if (error) {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : 'Network request failed'
        setNetworkError(message)
      } else {
        setNetworkError(null)
      }
    }

    loadNetwork()
    const intervalId = window.setInterval(loadNetwork, 4000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadContainers = async () => {
      const { data, error } = await fetchContainerOverview()
      if (cancelled) return
      setContainerOverview(data)
      if (error) {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : 'Container request failed'
        setContainerError(message)
      } else {
        setContainerError(null)
      }
    }

    loadContainers()
    const intervalId = window.setInterval(loadContainers, 7000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadZone = async () => {
      const { data, error } = await fetchZoneOverview()
      if (cancelled) return
      setZoneOverview(data)
      if (error) {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : 'Zone config request failed'
        setZoneError(message)
      } else {
        setZoneError(null)
      }
    }

    loadZone()
    const intervalId = window.setInterval(loadZone, 30000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const loadGateway = async () => {
      const { data, error } = await fetchGatewayOverview()
      if (cancelled) return
      setGatewayOverview(data)
      if (error) {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : 'Gateway config request failed'
        setGatewayError(message)
      } else {
        setGatewayError(null)
      }
    }

    loadGateway()
    const intervalId = window.setInterval(loadGateway, 30000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
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
        maximized: false,
      }
      return [...prev, next]
    })
  }, [windowSpec])

  const closeWindow = useCallback((id: WindowId) => {
    setWindows((prev) => {
      const closing = prev.find((win) => win.id === id)
      if (closing) {
        const restoreRect = closing.maximized ? closing.restoreRect : undefined
        windowMemoryRef.current[id] = {
          x: restoreRect?.x ?? closing.x,
          y: restoreRect?.y ?? closing.y,
          width: restoreRect?.width ?? closing.width,
          height: restoreRect?.height ?? closing.height,
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

  const toggleMaximize = useCallback((id: WindowId) => {
    setWindows((prev) =>
      prev.map((win) => {
        if (win.id !== id) {
          return win
        }

        if (win.maximized) {
          const restore = win.restoreRect
          if (!restore) {
            return { ...win, maximized: false }
          }

          const viewportWidth = typeof window === 'undefined' ? 1200 : window.innerWidth
          const viewportHeight = typeof window === 'undefined' ? 800 : window.innerHeight
          const restoredWidth = clamp(
            restore.width,
            MIN_WINDOW_WIDTH,
            Math.max(MIN_WINDOW_WIDTH, viewportWidth - WINDOW_MARGIN * 2),
          )
          const restoredHeight = clamp(
            restore.height,
            MIN_WINDOW_HEIGHT,
            Math.max(MIN_WINDOW_HEIGHT, viewportHeight - WINDOW_TOP_MARGIN - WINDOW_MARGIN),
          )
          const restoredX = clamp(
            restore.x,
            WINDOW_MARGIN,
            Math.max(WINDOW_MARGIN, viewportWidth - restoredWidth - WINDOW_MARGIN),
          )
          const restoredY = clamp(
            restore.y,
            WINDOW_TOP_MARGIN,
            Math.max(WINDOW_TOP_MARGIN, viewportHeight - restoredHeight - WINDOW_MARGIN),
          )

          return {
            ...win,
            x: restoredX,
            y: restoredY,
            width: restoredWidth,
            height: restoredHeight,
            maximized: false,
            restoreRect: undefined,
          }
        }

        const maximizedRect = getMaximizedRect()
        return {
          ...win,
          ...maximizedRect,
          maximized: true,
          restoreRect: {
            x: win.x,
            y: win.y,
            width: win.width,
            height: win.height,
          },
        }
      }),
    )
  }, [])

  const handleTitleDoubleClick = useCallback(
    (id: WindowId, event: MouseEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement | null
      const isControl = Boolean(
        target?.closest('[data-window-control="true"],button,a,input,textarea,select'),
      )
      if (isControl) {
        return
      }
      event.stopPropagation()
      toggleMaximize(id)
    },
    [toggleMaximize],
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

  const accessModePill = useMemo<AccessModePill>(() => {
    if (typeof window === 'undefined') {
      return {
        label: 'Direct mode',
        tone: 'bg-white/15 text-white ring-white/20',
        dot: 'bg-white/80',
        description: 'Direct access to local gateway endpoint.',
        host: 'unknown',
      }
    }

    const hostname = window.location.hostname.toLowerCase()
    const host = window.location.host
    const isIPv4 = /^(?:\d{1,3}\.){3}\d{1,3}$/.test(hostname)
    const isIPv6 = hostname.includes(':')

    if (isIPv4 || isIPv6) {
      return {
        label: 'Direct mode',
        tone: 'bg-sky-500/20 text-sky-100 ring-sky-300/40',
        dot: 'bg-sky-300',
        description:
          'Access through IP/local entry. Requests go straight to this node gateway with lower latency. Best for LAN and local debugging.',
        host,
      }
    }

    if (hostname.includes('web3.buckyos.ai')) {
      return {
        label: 'SN mode',
        tone: 'bg-emerald-500/20 text-emerald-100 ring-emerald-300/40',
        dot: 'bg-emerald-300',
        description:
          'Access through web3 SN domain. Traffic goes via SN/DDNS mapping and tunnel route, suitable for remote access with public TLS.',
        host,
      }
    }

    return {
      label: 'Direct mode',
      tone: 'bg-white/15 text-white ring-white/20',
      dot: 'bg-white/80',
      description:
        'Access through non-SN hostname. Requests are handled as direct gateway entry on this node.',
      host,
    }
  }, [])

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
          id: 'network' as const,
          label: 'Network',
          icon: 'network' as const,
          tile: 'bg-indigo-500',
        },
        {
          id: 'containers' as const,
          label: 'Containers',
          icon: 'container' as const,
          tile: 'bg-cyan-600',
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
      overview,
      overviewError,
      layout,
      apps,
      appsError,
      networkOverview,
      networkError,
      containerOverview,
      containerError,
      zoneOverview,
      zoneError,
      gatewayOverview,
      gatewayError,
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
      networkError,
      networkOverview,
      containerError,
      containerOverview,
      overview,
      overviewError,
      zoneError,
      zoneOverview,
      gatewayError,
      gatewayOverview,
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
  const onOpenNetworkWindow = useCallback(() => openWindow('network'), [openWindow])
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
        profileName={layout.profile.name}
        profileEmail={layout.profile.email}
        systemPill={systemPill}
        accessModePill={accessModePill}
        prefersReducedMotion={prefersReducedMotion}
        rxRate={rxRate}
        txRate={txRate}
        onNotificationsClick={onNotificationsClick}
        onNetworkClick={onOpenNetworkWindow}
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
        onTitleDoubleClick={handleTitleDoubleClick}
        closeWindow={closeWindow}
        toggleMinimize={toggleMinimize}
        toggleMaximize={toggleMaximize}
      />
    </div>
  )
}

type DesktopHeaderProps = {
  layoutError: string | null
  profileName: string
  profileEmail: string
  systemPill: { label: string; tone: string; dot: string }
  accessModePill: AccessModePill
  prefersReducedMotion: boolean
  rxRate: number
  txRate: number
  onNotificationsClick: () => void
  onNetworkClick: () => void
}

const DesktopHeader = memo((props: DesktopHeaderProps) => {
  const {
    layoutError,
    profileName,
    profileEmail,
    systemPill,
    accessModePill,
    prefersReducedMotion,
    rxRate,
    txRate,
    onNotificationsClick,
    onNetworkClick,
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
            <p className="truncate text-xs text-white/70">Control Desktop</p>
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
          <div
            className={`group relative hidden items-center gap-2 rounded-full px-3 py-1 text-xs font-semibold ring-1 lg:inline-flex ${accessModePill.tone}`}
            title={accessModePill.description}
          >
            <span className={`inline-flex size-2 rounded-full ${accessModePill.dot}`} aria-hidden />
            {accessModePill.label}
            <div className="pointer-events-none absolute left-1/2 top-[calc(100%+8px)] z-20 w-80 -translate-x-1/2 rounded-xl border border-white/20 bg-slate-900/90 px-3 py-2 text-left text-[11px] font-normal text-white/90 opacity-0 shadow-2xl transition-opacity duration-150 group-hover:opacity-100">
              <p>{accessModePill.description}</p>
              <p className="mt-1 text-white/65">Current host: {accessModePill.host}</p>
            </div>
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

          <button
            type="button"
            onClick={onNetworkClick}
            className="hidden items-center gap-2 rounded-xl bg-white/10 px-2.5 py-1.5 ring-1 ring-white/15 transition hover:bg-white/15 md:flex"
          >
            <Icon name="network" className="size-4 text-white/80" />
            <p className="text-[11px] font-semibold text-white">
              Down {formatRate(rxRate)}
              <span className="mx-2 text-white/40" aria-hidden>
                |
              </span>
              Up {formatRate(txRate)}
            </p>
          </button>

          <div className="flex items-center gap-3">
            <UserPatternAvatar name={profileName} className="size-8" />
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
        <div className="grid grid-cols-3 content-start justify-items-center gap-x-16 gap-y-5 sm:grid-cols-4 md:ml-[100px] md:w-fit md:grid-cols-5 md:justify-items-start md:gap-x-16 md:gap-y-6 lg:ml-[120px]">
          {desktopApps.map((app) => (
            <button
              key={app.id}
              type="button"
              onClick={() => openWindow(app.id)}
              className="group flex w-[86px] flex-col items-center gap-2 rounded-xl p-2 transition-colors hover:bg-white/10 focus-visible:bg-white/15 focus-visible:outline-none"
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
  overview: SystemOverview | null
  overviewError: string | null
  layout: RootLayoutData
  apps: DappCard[]
  appsError: string | null
  networkOverview: NetworkOverview | null
  networkError: string | null
  containerOverview: ContainerOverview | null
  containerError: string | null
  zoneOverview: ZoneOverview | null
  zoneError: string | null
  gatewayOverview: GatewayOverview | null
  gatewayError: string | null
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
  onTitleDoubleClick: (id: WindowId, event: MouseEvent<HTMLDivElement>) => void
  closeWindow: (id: WindowId) => void
  toggleMinimize: (id: WindowId) => void
  toggleMaximize: (id: WindowId) => void
}

const getWindowFullPath = (id: WindowId) => {
  if (id === 'monitor') return '/monitor'
  if (id === 'network') return '/network'
  if (id === 'containers') return '/containers'
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
    onTitleDoubleClick,
    closeWindow,
    toggleMinimize,
    toggleMaximize,
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
            onTitleDoubleClick={onTitleDoubleClick}
            closeWindow={closeWindow}
            toggleMinimize={toggleMinimize}
            toggleMaximize={toggleMaximize}
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
  onTitleDoubleClick: (id: WindowId, event: MouseEvent<HTMLDivElement>) => void
  closeWindow: (id: WindowId) => void
  toggleMinimize: (id: WindowId) => void
  toggleMaximize: (id: WindowId) => void
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
    onTitleDoubleClick,
    closeWindow,
    toggleMinimize,
    toggleMaximize,
  } = props
  const panelRef = useRef<HTMLDivElement | null>(null)

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

  useEffect(() => {
    const panel = panelRef.current
    if (!panel || typeof window === 'undefined') {
      return
    }

    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      return
    }

    const animation = panel.animate(
      [
        {
          opacity: 0,
          transform: 'translateY(10px) scale(0.985)',
          filter: 'saturate(0.92)',
        },
        {
          opacity: 1,
          transform: 'translateY(0) scale(1)',
          filter: 'saturate(1)',
        },
      ],
      {
        duration: 260,
        easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
        fill: 'both',
      },
    )

    return () => {
      animation.cancel()
    }
  }, [])

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
      <div
        ref={panelRef}
        className="flex h-full flex-col overflow-hidden rounded-3xl border border-white/10 bg-white/85 text-[var(--cp-ink)] shadow-2xl shadow-black/40 backdrop-blur"
      >
        <div
          className="flex items-center justify-between gap-3 border-b border-[rgba(215,225,223,0.65)] bg-white/70 px-4 py-3"
          onPointerDown={(event) => {
            if (win.maximized) {
              return
            }
            startWindowDrag(win.id, win.x, win.y, win.width, win.height, event)
          }}
          onPointerMove={onTitlePointerMove}
          onPointerUp={onTitlePointerUp}
          onPointerCancel={onTitlePointerUp}
          onDoubleClick={(event) => onTitleDoubleClick(win.id, event)}
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
              <button
                type="button"
                data-window-control="true"
                className="size-3 rounded-full bg-emerald-400/85 ring-1 ring-black/10"
                aria-label={`${win.maximized ? 'Restore' : 'Maximize'} ${win.title}`}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                  event.stopPropagation()
                  toggleMaximize(win.id)
                }}
              />
            </div>
            <div className="flex items-center gap-2">
              <Icon name={win.icon} className="size-4 text-[var(--cp-muted)]" />
              <p className="text-sm font-semibold tracking-tight text-[var(--cp-ink)]">{win.title}</p>
            </div>
          </div>
          {win.id === 'logs' ? (
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
          ) : null}
        </div>

        <div className="min-h-0 flex-1 overflow-auto bg-[var(--cp-surface-muted)] p-4">
          <WindowBody id={win.id} data={windowData} />
        </div>
      </div>

      {!win.maximized
        ? resizeHandles.map((handle) => (
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
          ))
        : null}
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
    overview,
    overviewError,
    layout,
    apps,
    networkOverview,
    networkError,
    containerOverview,
    containerError,
    zoneOverview,
    zoneError,
    gatewayOverview,
    gatewayError,
    logPeek,
    logPeekError,
    cpuPercent,
    memoryPercent,
    diskPercent,
    rxRate,
    txRate,
  } = data
  const [settingsMenu, setSettingsMenu] = useState<SettingsMenuKey>('general')
  const [expandedGatewayFile, setExpandedGatewayFile] = useState<string | null>(null)
  const [gatewayFileCache, setGatewayFileCache] = useState<Record<string, GatewayFileContent>>({})
  const [gatewayFileLoadingName, setGatewayFileLoadingName] = useState<string | null>(null)
  const [gatewayFileErrors, setGatewayFileErrors] = useState<Record<string, string>>({})
  const [selectedUserGroup, setSelectedUserGroup] = useState<UserGroupKey>('admin')
  const [selectedContactId, setSelectedContactId] = useState('self')

  const userContacts = useMemo(
    () => [
      {
        id: 'self',
        name: layout.profile.name || 'Current user',
        email: layout.profile.email || '-',
        status: 'online',
        role: 'Owner',
        memberships: {
          admin: 'Owner',
          family: 'Not in group',
          guests: 'Not in group',
        } satisfies Record<UserGroupKey, string>,
      },
    ],
    [layout.profile.email, layout.profile.name],
  )

  const selectedContact = userContacts.find((contact) => contact.id === selectedContactId) ?? userContacts[0]
  const selectedGroup = USER_WINDOW_GROUPS.find((group) => group.id === selectedUserGroup) ?? USER_WINDOW_GROUPS[0]
  const selectedGroupMembership = selectedContact.memberships[selectedUserGroup]

  const toggleGatewayFile = useCallback((name: string) => {
    setExpandedGatewayFile((prev) => (prev === name ? null : name))
  }, [])

  useEffect(() => {
    if (!expandedGatewayFile || gatewayFileCache[expandedGatewayFile]) {
      return
    }

    let cancelled = false

    const load = async () => {
      setGatewayFileLoadingName(expandedGatewayFile)
      setGatewayFileErrors((prev) => {
        const next = { ...prev }
        delete next[expandedGatewayFile]
        return next
      })

      const { data, error } = await fetchGatewayFile(expandedGatewayFile)
      if (cancelled) {
        return
      }

      if (data) {
        setGatewayFileCache((prev) => ({
          ...prev,
          [expandedGatewayFile]: data,
        }))
      } else {
        const message =
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : `Failed to load ${expandedGatewayFile}`
        setGatewayFileErrors((prev) => ({
          ...prev,
          [expandedGatewayFile]: message,
        }))
      }

      setGatewayFileLoadingName((current) => (current === expandedGatewayFile ? null : current))
    }

    load()

    return () => {
      cancelled = true
    }
  }, [expandedGatewayFile, gatewayFileCache])

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

  if (id === 'network') {
    return (
      <div className="space-y-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">Network monitor</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">
            Backend-thread timeline with per-interface throughput, errors, and drops.
          </p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <NetworkOverviewPanel
            overview={networkOverview}
            errorMessage={networkError}
            compact
          />
        </div>
      </div>
    )
  }

  if (id === 'containers') {
    return (
      <div className="space-y-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">Docker runtime overview</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">
            Manage container lifecycle and inspect runtime health for this node.
          </p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <ContainerOverviewPanel
            overview={containerOverview}
            errorMessage={containerError}
            compact
          />
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
    const totalGb = metrics.disk?.totalGb ?? 0
    const usedGb = metrics.disk?.usedGb ?? 0
    const freeGb = Math.max(0, totalGb - usedGb)

    return (
      <div className="space-y-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div>
            <div>
              <p className="text-sm font-semibold text-[var(--cp-ink)]">Storage center preview</p>
              <p className="text-xs text-[var(--cp-muted)]">Unified disk status with system telemetry.</p>
            </div>
          </div>
          <div className="mt-3 grid gap-2 sm:grid-cols-3">
            <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Total</p>
              <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{totalGb.toFixed(totalGb >= 100 ? 0 : 1)} GB</p>
            </div>
            <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Used</p>
              <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{usedGb.toFixed(usedGb >= 100 ? 0 : 1)} GB</p>
            </div>
            <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Free</p>
              <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{freeGb.toFixed(freeGb >= 100 ? 0 : 1)} GB</p>
            </div>
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <StorageDiskStatusPanel disk={metrics.disk} compact maxItems={5} />
        </div>

        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Health signals</p>
          <div className="mt-3">
            <StorageHealthSignalsPanel
              warnings={status.warnings}
              disks={metrics.disk?.disks}
              compact
            />
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">File manager handoff</p>
          <p className="mt-2 text-sm text-[var(--cp-muted)]">
            File browsing and operations are handled by a dedicated app. Use full storage view for
            integration entry.
          </p>
          <div className="mt-3 rounded-xl border border-dashed border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs text-[var(--cp-muted)]">
            Planned: app deep-link and compact file manager quick preview.
          </div>
        </div>
      </div>
    )
  }

  if (id === 'settings') {
    const selectedMenu = SETTINGS_WINDOW_MENU.find((menu) => menu.id === settingsMenu) ?? SETTINGS_WINDOW_MENU[0]
    const currentHost = typeof window === 'undefined' ? 'unknown' : window.location.host
    const storageUsed = metrics.disk?.usedGb ?? 0
    const storageTotal = metrics.disk?.totalGb ?? 0
    const snUrlDisplay = (zoneOverview?.sn.url ?? '').replace(/^https?:\/\//, '') || '-'
    const uptimeLabel = formatUptime(overview?.uptime_seconds ?? metrics.uptimeSeconds ?? 0)

    const contentByMenu: Record<SettingsMenuKey, React.ReactNode> = {
      general: (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Identity and system release</p>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Current user</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{layout.profile.name}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Device</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{layout.profile.email}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">System version</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{overview?.version ?? 'Beta1'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">System</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{overview?.os ?? 'Linux'} · {overview?.model ?? '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2 sm:col-span-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Uptime</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{uptimeLabel}</p>
              </div>
            </div>
            {overviewError ? (
              <div className="mt-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
                System overview fallback is active: {overviewError}
              </div>
            ) : null}
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Hardware and runtime details</p>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">CPU model</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{metrics.cpu?.model ?? 'Unknown CPU'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">CPU cores</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{metrics.cpu?.cores ?? '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Memory</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {metrics.memory?.usedGb?.toFixed(1) ?? '-'} / {metrics.memory?.totalGb?.toFixed(1) ?? '-'} GB ({memoryPercent}%)
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Swap</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {metrics.swap?.usedGb?.toFixed(1) ?? '-'} / {metrics.swap?.totalGb?.toFixed(1) ?? '-'} GB ({Math.round(metrics.swap?.usagePercent ?? 0)}%)
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Disk</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {metrics.disk?.usedGb?.toFixed(1) ?? '-'} / {metrics.disk?.totalGb?.toFixed(1) ?? '-'} GB ({diskPercent}%)
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Network throughput</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">Down {formatRate(rxRate)} · Up {formatRate(txRate)}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Process count</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{metrics.processCount ?? '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Load average</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {metrics.loadAverage
                    ? `${metrics.loadAverage.one.toFixed(2)} / ${metrics.loadAverage.five.toFixed(2)} / ${metrics.loadAverage.fifteen.toFixed(2)}`
                    : '-'}
                </p>
              </div>
            </div>
          </div>
        </div>
      ),
      'zone-manager': (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Zone identity (/opt/buckyos/etc)</p>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {zoneOverview?.zone.name || layout.profile.name}
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone domain</p>
                <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">
                  {zoneOverview?.zone.domain || '-'}
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone DID</p>
                <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.zone.did || '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Owner DID</p>
                <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.zone.ownerDid || '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">User name</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.zone.userName || '-'}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone IAT</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {zoneOverview?.zone.zoneIat ? String(zoneOverview.zone.zoneIat) : '-'}
                </p>
              </div>
            </div>

            {zoneError ? (
              <div className="mt-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
                Zone config fallback is active: {zoneError}
              </div>
            ) : null}
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">Device profile</p>
              <div className="mt-3 space-y-2">
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Device</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.device.name || '-'}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Device DID</p>
                  <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.device.did || '-'}</p>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                    <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Type</p>
                    <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.device.type || '-'}</p>
                  </div>
                  <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                    <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Net ID</p>
                    <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.device.netId || '-'}</p>
                  </div>
                </div>
              </div>
            </div>

            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">SN profile</p>
              <div className="mt-3 space-y-2">
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">SN username</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.sn.username || '-'}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">SN URL</p>
                  <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{snUrlDisplay}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Config dir</p>
                  <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.etcDir || '/opt/buckyos/etc'}</p>
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Zone-related files</p>
            <div className="mt-3 space-y-2">
              {(zoneOverview?.files ?? []).map((file) => (
                <div
                  key={file.path}
                  className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                >
                  <p className="text-xs font-semibold text-[var(--cp-ink)]">{file.name}</p>
                  <p className="mt-1 break-all text-[11px] text-[var(--cp-muted)]">{file.path}</p>
                  <p className="text-[11px] text-[var(--cp-muted)]">
                    {file.exists ? `size ${file.sizeBytes} bytes` : 'missing file'}
                    {file.modifiedAt ? ` · updated ${file.modifiedAt}` : ''}
                  </p>
                </div>
              ))}
              {!(zoneOverview?.files ?? []).length ? (
                <p className="text-xs text-[var(--cp-muted)]">No zone config files discovered.</p>
              ) : null}
            </div>
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm text-[var(--cp-muted)]">
            {(zoneOverview?.notes ?? []).length ? (
              (zoneOverview?.notes ?? []).map((note, index) => (
                <p key={`${index}-${note}`} className={index > 0 ? 'mt-2' : ''}>
                  {note}
                </p>
              ))
            ) : (
              <p>Zone manager shows identity and topology values sourced from /opt/buckyos/etc.</p>
            )}
          </div>
        </div>
      ),
      'sys-manager': (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">System Config Tree</p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">Depth-limited view (4 levels)</p>
            <div className="mt-3">
              <SystemConfigTreeViewer defaultKey="" depth={4} compact />
            </div>
          </div>
        </div>
      ),
      'gateway-manager': (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Gateway config overview</p>
            <div className="mt-3 grid gap-3 sm:grid-cols-3">
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Mode</p>
                <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                  {gatewayOverview?.mode === 'sn' ? 'SN mode' : 'Direct mode'}
                </p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Host</p>
                <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{currentHost}</p>
              </div>
              <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Config dir</p>
                <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">
                  {gatewayOverview?.etcDir ?? '/opt/buckyos/etc'}
                </p>
              </div>
            </div>

            {gatewayError ? (
              <div className="mt-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
                Gateway config fallback is active: {gatewayError}
              </div>
            ) : null}
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Gateway files</p>
            <div className="mt-3 space-y-2">
              {(gatewayOverview?.files ?? []).map((file) => (
                <div
                  key={file.path}
                  className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <p className="text-xs font-semibold text-[var(--cp-ink)]">{file.name}</p>
                    <button
                      type="button"
                      disabled={!file.exists}
                      onClick={() => toggleGatewayFile(file.name)}
                      className="rounded-full border border-[var(--cp-border)] bg-white px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {expandedGatewayFile === file.name
                        ? 'Hide file'
                        : gatewayFileLoadingName === file.name
                          ? 'Loading...'
                          : 'View file'}
                    </button>
                  </div>
                  <p className="mt-1 break-all text-[11px] text-[var(--cp-muted)]">{file.path}</p>
                  <p className="text-[11px] text-[var(--cp-muted)]">
                    {file.exists ? `size ${file.sizeBytes} bytes` : 'missing file'}
                    {file.modifiedAt ? ` · updated ${file.modifiedAt}` : ''}
                  </p>

                  {expandedGatewayFile === file.name ? (
                    <div className="mt-3 max-h-72 min-w-0 overflow-auto rounded-xl border border-[var(--cp-border)] bg-white p-3">
                      {gatewayFileLoadingName === file.name && !gatewayFileCache[file.name] ? (
                        <p className="text-xs text-[var(--cp-muted)]">Loading file content...</p>
                      ) : gatewayFileErrors[file.name] ? (
                        <p className="text-xs text-amber-800">{gatewayFileErrors[file.name]}</p>
                      ) : gatewayFileCache[file.name] ? (
                        <pre className="max-w-full whitespace-pre-wrap break-all font-mono text-[11px] leading-5 text-[var(--cp-ink)]">
                          {gatewayFileCache[file.name].content}
                        </pre>
                      ) : (
                        <p className="text-xs text-[var(--cp-muted)]">No file content available.</p>
                      )}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">Include chain</p>
              <div className="mt-3 space-y-2">
                {(gatewayOverview?.includes ?? []).map((item) => (
                  <div
                    key={item}
                    className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs text-[var(--cp-ink)]"
                  >
                    {item}
                  </div>
                ))}
                {!(gatewayOverview?.includes ?? []).length ? (
                  <p className="text-xs text-[var(--cp-muted)]">No include chain data.</p>
                ) : null}
              </div>
            </div>

            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">Stack bindings</p>
              <div className="mt-3 space-y-2">
                {(gatewayOverview?.stacks ?? []).map((stack) => (
                  <div
                    key={`${stack.name}-${stack.bind}`}
                    className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-xs font-semibold text-[var(--cp-ink)]">{stack.name}</p>
                      <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-slate-700">
                        {stack.protocol || 'unknown'}
                      </span>
                    </div>
                    <p className="mt-1 text-[11px] text-[var(--cp-muted)]">Bind: {stack.bind || 'N/A'}</p>
                  </div>
                ))}
                {!(gatewayOverview?.stacks ?? []).length ? (
                  <p className="text-xs text-[var(--cp-muted)]">No stack binding data.</p>
                ) : null}
              </div>
            </div>
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">TLS domains</p>
            <div className="mt-3 flex flex-wrap gap-2">
              {(gatewayOverview?.tlsDomains ?? []).map((domain) => (
                <span
                  key={domain}
                  className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-2.5 py-1 text-[11px] text-[var(--cp-ink)]"
                >
                  {domain}
                </span>
              ))}
              {!(gatewayOverview?.tlsDomains ?? []).length ? (
                <p className="text-xs text-[var(--cp-muted)]">No TLS domain config detected.</p>
              ) : null}
            </div>
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Route preview</p>
            <div className="mt-3 max-h-52 overflow-auto rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-3">
              <pre className="whitespace-pre-wrap break-words font-mono text-[11px] text-[var(--cp-ink)]">
                {gatewayOverview?.routePreview || 'No route preview available.'}
              </pre>
            </div>
          </div>

          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm text-[var(--cp-muted)]">
            {(gatewayOverview?.notes ?? []).map((note, index) => (
              <p key={`${index}-${note}`} className={index > 0 ? 'mt-2' : ''}>
                {note}
              </p>
            ))}
          </div>
        </div>
      ),
      storage: (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Storage policy snapshot</p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">
              {storageUsed.toFixed(storageUsed >= 100 ? 0 : 1)} / {storageTotal.toFixed(storageTotal >= 100 ? 0 : 1)} GB in use.
            </p>
            <div className="mt-3">
              <StorageDiskStatusPanel disk={metrics.disk} compact maxItems={4} />
            </div>
          </div>
        </div>
      ),
      permissions: (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">Permission baseline</p>
            <div className="mt-3 grid gap-2 sm:grid-cols-2">
              {SETTINGS_POLICY_BASELINE.map((policy) => (
                <div
                  key={policy}
                  className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs text-[var(--cp-ink)]"
                >
                  {policy}
                </div>
              ))}
            </div>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm text-[var(--cp-muted)]">
            Permission changes should be reviewed with role ownership, app scope, and audit trail.
          </div>
        </div>
      ),
      'software-update': (
        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">System release</p>
            <div className="mt-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4">
              <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Current version</p>
              <p className="mt-1 text-2xl font-semibold text-[var(--cp-ink)]">Beta1</p>
              <p className="mt-1 text-xs text-[var(--cp-muted)]">
                This version represents the whole BuckyOS system release, not individual app versions.
              </p>
            </div>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm text-[var(--cp-muted)]">
            Update strategy and channel policy are managed at system level by scheduler/repo workflow.
          </div>
        </div>
      ),
    }

    return (
      <div className="grid gap-4 md:grid-cols-[220px_1fr]">
        <aside className="rounded-2xl border border-[var(--cp-border)] bg-white p-3">
          <p className="px-2 pt-1 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Settings Menu
          </p>
          <div className="mt-2 space-y-1">
            {SETTINGS_WINDOW_MENU.map((menu) => (
              <button
                key={menu.id}
                type="button"
                onClick={() => setSettingsMenu(menu.id)}
                className={`w-full rounded-xl px-3 py-2 text-left transition ${
                  settingsMenu === menu.id
                    ? 'bg-[var(--cp-primary)] text-white shadow'
                    : 'bg-[var(--cp-surface-muted)] text-[var(--cp-ink)] hover:bg-[var(--cp-primary-soft)]'
                }`}
              >
                <p className="text-sm font-semibold">{menu.label}</p>
                <p className={`text-xs ${settingsMenu === menu.id ? 'text-white/85' : 'text-[var(--cp-muted)]'}`}>
                  {menu.description}
                </p>
              </button>
            ))}
          </div>
        </aside>

        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">{selectedMenu.label}</p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">{selectedMenu.description}</p>
          </div>
          {contentByMenu[settingsMenu]}
        </div>
      </div>
    )
  }

  if (id === 'users') {
    return (
      <div className="grid gap-4 md:grid-cols-[240px_1fr]">
        <aside className="flex flex-col justify-between rounded-2xl border border-[var(--cp-border)] bg-white p-3">
          <div className="space-y-4">
            <section>
              <p className="px-2 pt-1 text-[11px] font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Main menu</p>
              <p className="mt-2 px-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Groups</p>
              <div className="mt-2 space-y-1.5">
                {USER_WINDOW_GROUPS.map((group) => (
                  <button
                    key={group.id}
                    type="button"
                    onClick={() => setSelectedUserGroup(group.id)}
                    className={`w-full rounded-xl px-3 py-2 text-left transition ${
                      selectedUserGroup === group.id
                        ? 'bg-[var(--cp-primary)] text-white shadow'
                        : 'bg-[var(--cp-surface-muted)] text-[var(--cp-ink)] hover:bg-[var(--cp-primary-soft)]'
                    }`}
                  >
                    <p className="text-sm font-semibold capitalize">{group.label}</p>
                    <p className={`text-xs ${selectedUserGroup === group.id ? 'text-white/85' : 'text-[var(--cp-muted)]'}`}>
                      {group.description}
                    </p>
                  </button>
                ))}
              </div>
            </section>

            <section>
              <p className="px-2 text-[11px] font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Quick contacts</p>
              <p className="mt-2 px-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">All contacts</p>
              <div className="mt-2 space-y-1">
                {userContacts.map((contact) => (
                  <button
                    key={contact.id}
                    type="button"
                    onClick={() => setSelectedContactId(contact.id)}
                    className={`flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left transition ${
                      selectedContact.id === contact.id
                        ? 'bg-[var(--cp-primary-soft)] text-[var(--cp-ink)] ring-1 ring-[var(--cp-primary)]/30'
                        : 'bg-[var(--cp-surface-muted)] text-[var(--cp-ink)] hover:bg-[var(--cp-primary-soft)]'
                    }`}
                  >
                    <UserPatternAvatar name={contact.name} className="size-9 border border-[var(--cp-border)]" />
                    <div className="min-w-0">
                      <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{contact.name}</p>
                      <p className="truncate text-xs text-[var(--cp-muted)]">{contact.email}</p>
                    </div>
                  </button>
                ))}
              </div>
            </section>
          </div>

          <div className="group relative mt-4">
            <button
              type="button"
              aria-disabled="true"
              onClick={(event) => event.preventDefault()}
              className="w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
            >
              Invite
            </button>
            <div className="pointer-events-none absolute -top-10 left-1/2 -translate-x-1/2 rounded-lg bg-slate-900 px-2.5 py-1 text-[11px] font-medium text-white opacity-0 shadow-lg transition-opacity duration-150 group-hover:opacity-100">
              Not implemented yet
            </div>
          </div>
        </aside>

        <div className="space-y-4">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
            <p className="text-sm font-semibold text-[var(--cp-ink)]">User information</p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">Selected contact details and group context.</p>
            <div className="mt-4 flex flex-wrap items-center gap-3">
              <UserPatternAvatar name={selectedContact.name} className="size-12 border border-[var(--cp-border)]" />
              <div className="min-w-0">
                <p className="truncate text-lg font-semibold text-[var(--cp-ink)]">{selectedContact.name}</p>
                <p className="truncate text-sm text-[var(--cp-muted)]">{selectedContact.email}</p>
              </div>
              <span className="ml-auto rounded-full bg-emerald-100 px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wide text-emerald-700">
                {selectedContact.status}
              </span>
            </div>
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Account profile</p>
              <div className="mt-3 space-y-2">
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Role</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{selectedContact.role}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Active group</p>
                  <p className="mt-1 text-sm font-semibold capitalize text-[var(--cp-ink)]">{selectedGroup.label}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Membership</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{selectedGroupMembership}</p>
                </div>
              </div>
            </div>

            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Zone identity</p>
              <div className="mt-3 space-y-2">
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">SN username</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.sn.username || '-'}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone user</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.zone.userName || '-'}</p>
                </div>
                <div className="rounded-xl bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Zone DID</p>
                  <p className="mt-1 break-all text-sm font-semibold text-[var(--cp-ink)]">{zoneOverview?.zone.did || '-'}</p>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    )
  }

  return null
})
WindowBody.displayName = 'WindowBody'

export default DesktopHomePage
