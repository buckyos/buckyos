import {
  memo,
  useCallback,
  useEffect,
  useRef,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import type {
  LayoutState,
  SystemPreferencesInput,
  ThemeMode,
  WindowAppearancePreferences,
} from '../../models/ui'
import { AppContentRenderer } from '../../app/registry'
import { DesktopWindowContainer } from './DesktopWindowContainer'
import {
  desktopWindowTransform,
  getDesktopWindowPositionBounds,
  getDesktopWindowWorkspaceBounds,
} from './geometry'
import type {
  DesktopWindowDataModel,
  DesktopWindowLayerDataModel,
  ResizeDirection,
} from './types'

type SafeArea = { top: number; bottom: number; left: number; right: number }
type Size = { width: number; height: number }
type WindowGeometry = { x: number; y: number; width: number; height: number }

const zeroSafeArea: SafeArea = { top: 0, bottom: 0, left: 0, right: 0 }

// ---------------------------------------------------------------------------
// Pointer sessions (drag / resize)
//
// While a window is being dragged or resized nothing goes through React:
// pointermove only records the latest pointer position, one animation frame
// resolves it into a clamped geometry and writes that straight to the window
// element (transform / width / height). The store is updated exactly once,
// on pointerup, with the final geometry. This keeps the hot path at one
// style write per frame instead of a full desktop re-render per event.
// ---------------------------------------------------------------------------

interface BoundsInput {
  safeArea: SafeArea
  topInset: number
  workspaceSize: Size
}

interface SessionBase {
  id: string
  node: HTMLElement
  pointerId: number
  /** Latest pointer position (client coordinates). */
  clientX: number
  clientY: number
  /** Whether any pointermove was seen — a plain click commits nothing. */
  moved: boolean
}

interface DragSession extends SessionBase {
  kind: 'drag'
  offsetX: number
  offsetY: number
  width: number
  height: number
  layerLeft: number
  layerTop: number
}

interface ResizeSession extends SessionBase {
  kind: 'resize'
  direction: ResizeDirection
  minWidth: number
  minHeight: number
  startWidth: number
  startHeight: number
  startX: number
  startY: number
  startWindowX: number
  startWindowY: number
}

type PointerSession = DragSession | ResizeSession

function resolveDragGeometry(session: DragSession, bounds: BoundsInput): WindowGeometry {
  const workspaceBounds = getDesktopWindowWorkspaceBounds({
    safeArea: bounds.safeArea,
    topInset: bounds.topInset,
    viewportSize: bounds.workspaceSize,
  })
  const positionBounds = getDesktopWindowPositionBounds(workspaceBounds, {
    width: session.width,
    height: session.height,
  })
  const x = Math.min(
    Math.max(positionBounds.minX, session.clientX - session.layerLeft - session.offsetX),
    positionBounds.maxX,
  )
  const y = Math.min(
    Math.max(positionBounds.minY, session.clientY - session.layerTop - session.offsetY),
    positionBounds.maxY,
  )
  return { x, y, width: session.width, height: session.height }
}

function resolveResizeGeometry(session: ResizeSession, bounds: BoundsInput): WindowGeometry {
  const workspaceBounds = getDesktopWindowWorkspaceBounds({
    safeArea: bounds.safeArea,
    topInset: bounds.topInset,
    viewportSize: bounds.workspaceSize,
  })
  const minLeft = workspaceBounds.minX
  const maxRight = workspaceBounds.maxRight
  const maxBottom = workspaceBounds.maxBottom
  const minWidth = Math.min(session.minWidth, workspaceBounds.maxWidth)
  const minHeight = Math.min(session.minHeight, workspaceBounds.maxHeight)
  const deltaX = session.clientX - session.startX
  const deltaY = session.clientY - session.startY
  const { direction } = session
  let x = session.startWindowX
  let width = session.startWidth
  let height = session.startHeight

  if (direction === 'right' || direction === 'bottom-right') {
    width = Math.min(
      Math.max(minWidth, session.startWidth + deltaX),
      Math.max(minWidth, maxRight - session.startWindowX),
    )
  }

  if (direction === 'left' || direction === 'bottom-left') {
    x = Math.min(
      Math.max(minLeft, session.startWindowX + deltaX),
      session.startWindowX + session.startWidth - minWidth,
    )
    width = session.startWidth + (session.startWindowX - x)
  }

  if (direction === 'bottom' || direction === 'bottom-left' || direction === 'bottom-right') {
    height = Math.min(
      Math.max(minHeight, session.startHeight + deltaY),
      Math.max(minHeight, maxBottom - session.startWindowY),
    )
  }

  return { x, y: session.startWindowY, width, height }
}

function resolveSessionGeometry(session: PointerSession, bounds: BoundsInput) {
  return session.kind === 'drag'
    ? resolveDragGeometry(session, bounds)
    : resolveResizeGeometry(session, bounds)
}

function applyGeometryToNode(node: HTMLElement, geometry: WindowGeometry) {
  node.style.transform = desktopWindowTransform(geometry.x, geometry.y)
  node.style.width = `${geometry.width}px`
  node.style.height = `${geometry.height}px`
}

function capturePointer(target: EventTarget | null, pointerId: number) {
  if (target instanceof Element) {
    // Keeps pointer events flowing to us even when the pointer crosses an
    // iframe hosted by another window, or leaves the browser viewport.
    try {
      target.setPointerCapture(pointerId)
    } catch {
      // Pointer already released — nothing to capture.
    }
  }
}

// ---------------------------------------------------------------------------
// Window slot
// ---------------------------------------------------------------------------

interface WindowSlotProps {
  activityLog: string[]
  isFront: boolean
  layoutState: LayoutState
  locale: string
  onClose: (windowId: string) => void
  onDragPointerDown: (
    windowItem: DesktopWindowDataModel,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => void
  onFocus: (windowId: string) => void
  onMaximize: (windowId: string) => void
  onMinimize: (windowId: string) => void
  onRegisterNode: (windowId: string, node: HTMLDivElement | null) => void
  onResizePointerDown: (
    windowItem: DesktopWindowDataModel,
    direction: ResizeDirection,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => void
  onSaveSettings: (values: SystemPreferencesInput) => void
  runtimeContainer: string
  safeArea: SafeArea
  themeMode: ThemeMode
  topInset: number
  windowAppearance: WindowAppearancePreferences
  windowItem: DesktopWindowDataModel
  workspaceSize: Size
}

/**
 * One window. Memoised so that dragging/resizing window A (which updates the
 * store on every pointermove) does not re-render window B and its app panel:
 * `windowItem` keeps its identity while the record is unchanged (see
 * `createDesktopWindowLayerDataModel`), and every callback prop is stable.
 */
const DesktopWindowSlot = memo(function DesktopWindowSlot({
  activityLog,
  isFront,
  layoutState,
  locale,
  onClose,
  onDragPointerDown,
  onFocus,
  onMaximize,
  onMinimize,
  onRegisterNode,
  onResizePointerDown,
  onSaveSettings,
  runtimeContainer,
  safeArea,
  themeMode,
  topInset,
  windowAppearance,
  windowItem,
  workspaceSize,
}: WindowSlotProps) {
  const isMaximized = windowItem.state === 'maximized'
  const windowId = windowItem.id
  const registerNode = useCallback(
    (node: HTMLDivElement | null) => onRegisterNode(windowId, node),
    [onRegisterNode, windowId],
  )

  return (
    <DesktopWindowContainer
      ref={registerNode}
      isFront={isFront}
      onClose={() => onClose(windowId)}
      onDragPointerDown={(event) => onDragPointerDown(windowItem, event)}
      onFocus={() => onFocus(windowId)}
      onMaximize={() => onMaximize(windowId)}
      onMinimize={() => onMinimize(windowId)}
      onResizePointerDown={(direction) => (event) =>
        onResizePointerDown(windowItem, direction, event)}
      style={{
        zIndex: windowItem.zIndex,
        transform: desktopWindowTransform(
          isMaximized ? safeArea.left : windowItem.x,
          isMaximized ? topInset : windowItem.y,
        ),
        width: isMaximized
          ? workspaceSize.width - safeArea.left - safeArea.right
          : windowItem.width,
        height: isMaximized
          ? workspaceSize.height - topInset - safeArea.bottom
          : windowItem.height,
      }}
      themeMode={themeMode}
      uiModel={windowItem}
      windowAppearance={windowAppearance}
    >
      <AppContentRenderer
        activityLog={activityLog}
        app={windowItem.app}
        windowId={windowId}
        launch={windowItem.launch}
        layoutState={layoutState}
        locale={locale}
        onSaveSettings={onSaveSettings}
        runtimeContainer={runtimeContainer}
        themeMode={themeMode}
        windowAppearance={windowAppearance}
      />
    </DesktopWindowContainer>
  )
})

// ---------------------------------------------------------------------------
// Window layer
// ---------------------------------------------------------------------------

export function DesktopWindowLayer({
  activityLog,
  layoutState,
  locale,
  onClose,
  onGeometryChange,
  onFocus,
  onMaximize,
  onMinimize,
  onSaveSettings,
  runtimeContainer,
  safeArea = zeroSafeArea,
  themeMode,
  topInset,
  uiModel,
  windowAppearance,
  workspaceSize,
}: {
  activityLog: string[]
  layoutState: LayoutState
  locale: string
  onClose: (windowId: string) => void
  onGeometryChange: (
    windowId: string,
    geometry: Partial<Pick<DesktopWindowDataModel, 'x' | 'y' | 'width' | 'height'>>,
  ) => void
  onFocus: (windowId: string) => void
  onMaximize: (windowId: string) => void
  onMinimize: (windowId: string) => void
  onSaveSettings: (values: SystemPreferencesInput) => void
  runtimeContainer: string
  safeArea?: SafeArea
  themeMode: ThemeMode
  topInset: number
  uiModel: DesktopWindowLayerDataModel
  windowAppearance: WindowAppearancePreferences
  workspaceSize: Size
}) {
  const windows = uiModel.windows
  const topZIndex = uiModel.topWindow?.zIndex ?? 0
  const layerRef = useRef<HTMLDivElement | null>(null)
  const nodesRef = useRef(new Map<string, HTMLDivElement>())
  const sessionRef = useRef<PointerSession | null>(null)
  const frameRef = useRef<number | null>(null)

  // The move/up listeners are attached once; they read the latest bounds and
  // commit callback through refs instead of re-subscribing on every change.
  const boundsRef = useRef<BoundsInput>({ safeArea, topInset, workspaceSize })
  const onGeometryChangeRef = useRef(onGeometryChange)
  useEffect(() => {
    boundsRef.current = { safeArea, topInset, workspaceSize }
  }, [safeArea, topInset, workspaceSize])
  useEffect(() => {
    onGeometryChangeRef.current = onGeometryChange
  }, [onGeometryChange])

  const registerNode = useCallback((windowId: string, node: HTMLDivElement | null) => {
    if (node) {
      nodesRef.current.set(windowId, node)
    } else {
      nodesRef.current.delete(windowId)
    }
  }, [])

  const handlePointerDown = useCallback(
    (windowItem: DesktopWindowDataModel, event: ReactPointerEvent<HTMLDivElement>) => {
      if (windowItem.state !== 'windowed') {
        onFocus(windowItem.id)
        return
      }

      const node = nodesRef.current.get(windowItem.id)
      const layerRect = layerRef.current?.getBoundingClientRect()
      if (!node || !layerRect || sessionRef.current) {
        return
      }

      sessionRef.current = {
        kind: 'drag',
        id: windowItem.id,
        node,
        pointerId: event.pointerId,
        clientX: event.clientX,
        clientY: event.clientY,
        moved: false,
        offsetX: event.clientX - layerRect.left - windowItem.x,
        offsetY: event.clientY - layerRect.top - windowItem.y,
        width: windowItem.width,
        height: windowItem.height,
        layerLeft: layerRect.left,
        layerTop: layerRect.top,
      }
      node.dataset.dragging = 'true'
      capturePointer(event.currentTarget, event.pointerId)
      onFocus(windowItem.id)
      event.preventDefault()
    },
    [onFocus],
  )

  const handleResizePointerDown = useCallback(
    (
      windowItem: DesktopWindowDataModel,
      direction: ResizeDirection,
      event: ReactPointerEvent<HTMLDivElement>,
    ) => {
      if (windowItem.state !== 'windowed') {
        return
      }

      const node = nodesRef.current.get(windowItem.id)
      if (!node || sessionRef.current) {
        return
      }

      sessionRef.current = {
        kind: 'resize',
        id: windowItem.id,
        node,
        pointerId: event.pointerId,
        clientX: event.clientX,
        clientY: event.clientY,
        moved: false,
        direction,
        minWidth: windowItem.app.manifest.desktopWindow?.minWidth ?? 420,
        minHeight: windowItem.app.manifest.desktopWindow?.minHeight ?? 280,
        startWidth: windowItem.width,
        startHeight: windowItem.height,
        startX: event.clientX,
        startY: event.clientY,
        startWindowX: windowItem.x,
        startWindowY: windowItem.y,
      }
      node.dataset.dragging = 'true'
      capturePointer(event.currentTarget, event.pointerId)
      onFocus(windowItem.id)
      event.preventDefault()
      event.stopPropagation()
    },
    [onFocus],
  )

  useEffect(() => {
    const flushFrame = () => {
      frameRef.current = null
      const session = sessionRef.current
      if (!session) return
      applyGeometryToNode(session.node, resolveSessionGeometry(session, boundsRef.current))
    }

    const handleMove = (event: PointerEvent) => {
      const session = sessionRef.current
      if (!session || event.pointerId !== session.pointerId) return
      session.clientX = event.clientX
      session.clientY = event.clientY
      session.moved = true
      if (frameRef.current === null) {
        frameRef.current = window.requestAnimationFrame(flushFrame)
      }
    }

    const finishSession = () => {
      const session = sessionRef.current
      if (!session) return
      sessionRef.current = null
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
      delete session.node.dataset.dragging
      if (!session.moved) return

      const geometry = resolveSessionGeometry(session, boundsRef.current)
      applyGeometryToNode(session.node, geometry)
      onGeometryChangeRef.current(session.id, geometry)
    }

    window.addEventListener('pointermove', handleMove)
    window.addEventListener('pointerup', finishSession)
    window.addEventListener('pointercancel', finishSession)

    return () => {
      window.removeEventListener('pointermove', handleMove)
      window.removeEventListener('pointerup', finishSession)
      window.removeEventListener('pointercancel', finishSession)
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
    }
  }, [])

  return (
    <div ref={layerRef} className="pointer-events-none absolute inset-0 z-30">
      {windows.map((windowItem) => (
        <DesktopWindowSlot
          key={windowItem.id}
          activityLog={activityLog}
          isFront={windowItem.zIndex === topZIndex}
          layoutState={layoutState}
          locale={locale}
          onClose={onClose}
          onDragPointerDown={handlePointerDown}
          onFocus={onFocus}
          onMaximize={onMaximize}
          onMinimize={onMinimize}
          onRegisterNode={registerNode}
          onResizePointerDown={handleResizePointerDown}
          onSaveSettings={onSaveSettings}
          runtimeContainer={runtimeContainer}
          safeArea={safeArea}
          themeMode={themeMode}
          topInset={topInset}
          windowAppearance={windowAppearance}
          windowItem={windowItem}
          workspaceSize={workspaceSize}
        />
      ))}
    </div>
  )
}
