import { findDesktopAppById } from '../../app/registry'
import type { DesktopAppItem } from '../../app/types'
import type { AppDefinition, WindowRecord } from '../../models/ui'
import type { DesktopWindowDataModel, DesktopWindowLayerDataModel } from './types'

const fallbackWindowSizing = {
  width: 540,
  height: 380,
  minWidth: 420,
  minHeight: 280,
}

export function resolveDesktopWindowSizing(app: AppDefinition) {
  const preferred = app.manifest.desktopWindow
  const minWidth = preferred?.minWidth ?? fallbackWindowSizing.minWidth
  const minHeight = preferred?.minHeight ?? fallbackWindowSizing.minHeight

  return {
    width: Math.max(preferred?.width ?? fallbackWindowSizing.width, minWidth),
    height: Math.max(preferred?.height ?? fallbackWindowSizing.height, minHeight),
    minWidth,
    minHeight,
  }
}

export function createWindowRecord(
  app: AppDefinition,
  index: number,
  geometry?: Partial<Pick<WindowRecord, 'x' | 'y' | 'width' | 'height'>>,
): WindowRecord {
  const sizing = resolveDesktopWindowSizing(app)

  return {
    id: `${app.id}-${Date.now()}`,
    appId: app.id,
    state: app.manifest.defaultMode === 'windowed' ? 'windowed' : 'maximized',
    minimizedOrder: null,
    titleKey: app.labelKey,
    x: geometry?.x ?? 48 + (index % 4) * 36,
    y: geometry?.y ?? 54 + (index % 3) * 32,
    width: geometry?.width ?? sizing.width,
    height: geometry?.height ?? sizing.height,
    zIndex: 10 + index,
  }
}

export function createDesktopWindowLayerDataModel(
  apps: DesktopAppItem[],
  windows: WindowRecord[],
): DesktopWindowLayerDataModel {
  const visibleWindows = windows
    .filter((windowItem) => windowItem.state !== 'minimized')
    .map((windowItem) => {
      const app = findDesktopAppById(apps, windowItem.appId)

      if (!app) {
        return null
      }

      return {
        ...windowItem,
        app,
      }
    })
    .filter((windowItem): windowItem is DesktopWindowDataModel => Boolean(windowItem))
    .sort((left, right) => left.zIndex - right.zIndex)

  return {
    windows: visibleWindows,
    topWindow: visibleWindows[visibleWindows.length - 1],
  }
}
