import type { LayoutState } from '../../models/ui'

export const desktopWindowViewportPadding = {
  top: 8,
  right: 12,
  bottom: 8,
  left: 12,
} as const

export const desktopWindowTitleBarHeight = 32
export const desktopWindowMinVisibleTitleBarWidth = 120

export function getDesktopWindowWorkspaceBounds({
  deadZone,
  safeArea,
  topInset,
  viewportSize,
}: {
  deadZone: LayoutState['deadZone']
  safeArea: { top: number; bottom: number; left: number; right: number }
  topInset: number
  viewportSize: { width: number; height: number }
}) {
  const minX =
    safeArea.left + deadZone.left + desktopWindowViewportPadding.left
  const minY = topInset + desktopWindowViewportPadding.top
  const maxRight =
    viewportSize.width -
    safeArea.right -
    deadZone.right -
    desktopWindowViewportPadding.right
  const maxBottom =
    viewportSize.height -
    safeArea.bottom -
    deadZone.bottom -
    desktopWindowViewportPadding.bottom

  return {
    minX,
    minY,
    maxRight,
    maxBottom,
    maxWidth: Math.max(240, maxRight - minX),
    maxHeight: Math.max(160, maxBottom - minY),
  }
}

export function getDesktopWindowPositionBounds(
  workspaceBounds: ReturnType<typeof getDesktopWindowWorkspaceBounds>,
  size: { width: number; height: number },
) {
  const visibleTitleBarWidth = Math.min(
    size.width,
    desktopWindowMinVisibleTitleBarWidth,
  )

  return {
    minX: workspaceBounds.minX - size.width + visibleTitleBarWidth,
    maxX: workspaceBounds.maxRight - visibleTitleBarWidth,
    minY: workspaceBounds.minY,
    maxY: Math.max(
      workspaceBounds.minY,
      workspaceBounds.maxBottom - desktopWindowTitleBarHeight,
    ),
  }
}
