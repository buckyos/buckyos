import {
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
  getDesktopWindowPositionBounds,
  getDesktopWindowWorkspaceBounds,
} from './geometry'
import type {
  DesktopWindowDataModel,
  DesktopWindowLayerDataModel,
  ResizeDirection,
} from './types'

export function DesktopWindowLayer({
  activityLog,
  deadZone,
  layoutState,
  locale,
  onClose,
  onGeometryChange,
  onFocus,
  onMaximize,
  onMinimize,
  onSaveSettings,
  runtimeContainer,
  safeArea = { top: 0, bottom: 0, left: 0, right: 0 },
  themeMode,
  topInset,
  uiModel,
  windowAppearance,
  workspaceSize,
}: {
  activityLog: string[]
  deadZone: LayoutState['deadZone']
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
  safeArea?: { top: number; bottom: number; left: number; right: number }
  themeMode: ThemeMode
  topInset: number
  uiModel: DesktopWindowLayerDataModel
  windowAppearance: WindowAppearancePreferences
  workspaceSize: { width: number; height: number }
}) {
  const windows = uiModel.windows
  const topZIndex = uiModel.topWindow?.zIndex ?? 0
  const dragState = useRef<{
    id: string
    offsetX: number
    offsetY: number
  } | null>(null)
  const resizeState = useRef<{
    id: string
    direction: ResizeDirection
    minWidth: number
    minHeight: number
    startWidth: number
    startHeight: number
    startX: number
    startY: number
    startWindowX: number
    startWindowY: number
  } | null>(null)
  const windowsRef = useRef(windows)
  const layerRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    windowsRef.current = windows
  }, [windows])

  const handlePointerDown =
    (windowItem: DesktopWindowDataModel) =>
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (windowItem.state !== 'windowed') {
        onFocus(windowItem.id)
        return
      }

      const layerRect = layerRef.current?.getBoundingClientRect()
      if (!layerRect) {
        return
      }

      dragState.current = {
        id: windowItem.id,
        offsetX: event.clientX - layerRect.left - windowItem.x,
        offsetY: event.clientY - layerRect.top - windowItem.y,
      }
      onFocus(windowItem.id)
      event.preventDefault()
    }

  const handleResizePointerDown =
    (windowItem: DesktopWindowDataModel, direction: ResizeDirection) =>
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (windowItem.state !== 'windowed') {
        return
      }

      resizeState.current = {
        id: windowItem.id,
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
      onFocus(windowItem.id)
      event.preventDefault()
      event.stopPropagation()
    }

  useEffect(() => {
    const handleMove = (event: PointerEvent) => {
      const layerRect = layerRef.current?.getBoundingClientRect()
      if (!layerRect) {
        return
      }

      const activeResize = resizeState.current
      if (activeResize) {
        const workspaceBounds = getDesktopWindowWorkspaceBounds({
          deadZone,
          safeArea,
          topInset,
          viewportSize: workspaceSize,
        })
        const minLeft = workspaceBounds.minX
        const maxRight = workspaceBounds.maxRight
        const maxBottom = workspaceBounds.maxBottom
        const maxWidth = workspaceBounds.maxWidth
        const maxHeight = workspaceBounds.maxHeight
        const minWidth = Math.min(activeResize.minWidth, maxWidth)
        const minHeight = Math.min(activeResize.minHeight, maxHeight)
        const deltaX = event.clientX - activeResize.startX
        const deltaY = event.clientY - activeResize.startY
        let nextX = activeResize.startWindowX
        let nextWidth = activeResize.startWidth
        let nextHeight = activeResize.startHeight

        if (
          activeResize.direction === 'right' ||
          activeResize.direction === 'bottom-right'
        ) {
          nextWidth = Math.min(
            Math.max(minWidth, activeResize.startWidth + deltaX),
            Math.max(minWidth, maxRight - activeResize.startWindowX),
          )
        }

        if (
          activeResize.direction === 'left' ||
          activeResize.direction === 'bottom-left'
        ) {
          nextX = Math.min(
            Math.max(minLeft, activeResize.startWindowX + deltaX),
            activeResize.startWindowX + activeResize.startWidth - minWidth,
          )
          nextWidth =
            activeResize.startWidth + (activeResize.startWindowX - nextX)
        }

        if (
          activeResize.direction === 'bottom' ||
          activeResize.direction === 'bottom-left' ||
          activeResize.direction === 'bottom-right'
        ) {
          nextHeight = Math.min(
            Math.max(minHeight, activeResize.startHeight + deltaY),
            Math.max(minHeight, maxBottom - activeResize.startWindowY),
          )
        }

        onGeometryChange(activeResize.id, {
          ...(activeResize.direction === 'left' ||
          activeResize.direction === 'bottom-left'
            ? {
                x: nextX,
                y: activeResize.startWindowY,
              }
            : undefined),
          width: nextWidth,
          height: nextHeight,
        })
        return
      }

      const activeDrag = dragState.current
      if (activeDrag) {
        const draggingWindow = windowsRef.current.find(
          (windowItem) => windowItem.id === activeDrag.id,
        )
        const measured = draggingWindow ?? { width: 540, height: 380 }
        const workspaceBounds = getDesktopWindowWorkspaceBounds({
          deadZone,
          safeArea,
          topInset,
          viewportSize: workspaceSize,
        })
        const positionBounds = getDesktopWindowPositionBounds(
          workspaceBounds,
          measured,
        )

        const nextX = Math.min(
          Math.max(
            positionBounds.minX,
            event.clientX - layerRect.left - activeDrag.offsetX,
          ),
          positionBounds.maxX,
        )
        const nextY = Math.min(
          Math.max(
            positionBounds.minY,
            event.clientY - layerRect.top - activeDrag.offsetY,
          ),
          positionBounds.maxY,
        )
        onGeometryChange(activeDrag.id, { x: nextX, y: nextY })
      }
    }

    const handleUp = () => {
      dragState.current = null
      resizeState.current = null
    }

    window.addEventListener('pointermove', handleMove)
    window.addEventListener('pointerup', handleUp)
    window.addEventListener('pointercancel', handleUp)

    return () => {
      window.removeEventListener('pointermove', handleMove)
      window.removeEventListener('pointerup', handleUp)
      window.removeEventListener('pointercancel', handleUp)
    }
  }, [
    deadZone,
    onFocus,
    onGeometryChange,
    safeArea,
    topInset,
    workspaceSize,
  ])

  return (
    <div ref={layerRef} className="pointer-events-none absolute inset-0 z-30">
      {windows.map((windowItem) => {
        const isMaximized = windowItem.state === 'maximized'
        const isFront = windowItem.zIndex === topZIndex

        return (
          <DesktopWindowContainer
            key={windowItem.id}
            isFront={isFront}
            onClose={() => onClose(windowItem.id)}
            onDragPointerDown={handlePointerDown(windowItem)}
            onFocus={() => onFocus(windowItem.id)}
            onMaximize={() => onMaximize(windowItem.id)}
            onMinimize={() => onMinimize(windowItem.id)}
            onResizePointerDown={(direction) =>
              handleResizePointerDown(windowItem, direction)
            }
            style={{
              zIndex: windowItem.zIndex,
              left: isMaximized ? safeArea.left + deadZone.left + 12 : windowItem.x,
              top: isMaximized ? topInset + 12 : windowItem.y,
              width: isMaximized
                ? workspaceSize.width -
                  safeArea.left -
                  deadZone.left -
                  safeArea.right -
                  deadZone.right -
                  24
                : windowItem.width,
              height: isMaximized
                ? workspaceSize.height - topInset - safeArea.bottom - deadZone.bottom - 24
                : windowItem.height,
              transform: isFront ? 'translateY(0)' : 'translateY(4px)',
            }}
            themeMode={themeMode}
            uiModel={windowItem}
            windowAppearance={windowAppearance}
          >
            <AppContentRenderer
              activityLog={activityLog}
              app={windowItem.app}
              layoutState={layoutState}
              locale={locale}
              onSaveSettings={onSaveSettings}
              runtimeContainer={runtimeContainer}
              themeMode={themeMode}
              windowAppearance={windowAppearance}
            />
          </DesktopWindowContainer>
        )
      })}
    </div>
  )
}
