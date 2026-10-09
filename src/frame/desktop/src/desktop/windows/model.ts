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

/**
 * Per-record memo so an unchanged `WindowRecord` maps to the same
 * `DesktopWindowDataModel` object across recomputations. The window layer
 * relies on this identity to skip re-rendering windows that did not move
 * while a sibling is being dragged.
 */
const windowModelCache = new WeakMap<WindowRecord, DesktopWindowDataModel>()

/**
 * The layer model keeps `windows` in the store's (creation) order and never
 * drops or reorders entries while a window is open:
 *
 * - Minimized windows stay in the list; the window layer hides them instead
 *   of unmounting them.
 * - Stacking is expressed only through each window's `zIndex` style, not
 *   through render order. Reordering keyed siblings makes React move the DOM
 *   nodes, and a moved `<iframe>` reloads its page.
 *
 * Both would otherwise make an embedded web app lose its state and restart
 * every time the user minimizes/restores it or merely focuses another window.
 * `topWindow` is the visible window with the highest `zIndex`.
 */
export function createDesktopWindowLayerDataModel(
  apps: DesktopAppItem[],
  windows: WindowRecord[],
): DesktopWindowLayerDataModel {
  const layerWindows = windows
    .map((windowItem) => {
      const app = findDesktopAppById(apps, windowItem.appId)

      if (!app) {
        return null
      }

      const cached = windowModelCache.get(windowItem)
      if (cached && cached.app === app) {
        return cached
      }

      const model: DesktopWindowDataModel = {
        ...windowItem,
        app,
      }
      windowModelCache.set(windowItem, model)
      return model
    })
    .filter((windowItem): windowItem is DesktopWindowDataModel => Boolean(windowItem))

  let topWindow: DesktopWindowDataModel | undefined
  for (const windowItem of layerWindows) {
    if (windowItem.state === 'minimized') continue
    if (!topWindow || windowItem.zIndex >= topWindow.zIndex) topWindow = windowItem
  }

  return { windows: layerWindows, topWindow }
}
