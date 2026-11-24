/* ── An app's window declaration (AppDoc `window`, system-apps.json) ↔ the Shell's WindowManifest ── */

import type { AppWindowDeclaration } from 'buckyos/open-app'
import type { WindowManifest } from '../models/ui'

export function windowDeclarationOf(manifest: WindowManifest): AppWindowDeclaration {
  const { width, height, minWidth, minHeight } = manifest.desktopWindow ?? {}
  return {
    instance: manifest.instance ?? 'single',
    default_mode: manifest.defaultMode === 'windowed' ? 'windowed' : 'maximized',
    ...(width ? { width } : {}),
    ...(height ? { height } : {}),
    ...(minWidth ? { min_width: minWidth } : {}),
    ...(minHeight ? { min_height: minHeight } : {}),
    resizable: manifest.resizable ?? true,
    allow_minimize: manifest.allowMinimize,
    allow_maximize: manifest.allowMaximize,
    mobile_header: manifest.mobileStatusBarMode,
  }
}

type DeclaredManifest = Pick<WindowManifest, 'defaultMode' | 'instance' | 'resizable' | 'allowMinimize' | 'allowMaximize' | 'mobileStatusBarMode' | 'desktopWindow'>

export function windowManifestOf(declaration: AppWindowDeclaration): DeclaredManifest {
  const { width, height, min_width: minWidth, min_height: minHeight } = declaration
  return {
    defaultMode: declaration.default_mode,
    instance: declaration.instance,
    resizable: declaration.resizable,
    allowMinimize: declaration.allow_minimize,
    allowMaximize: declaration.allow_maximize,
    mobileStatusBarMode: declaration.mobile_header,
    ...(width || height || minWidth || minHeight ? { desktopWindow: { ...(width ? { width } : {}), ...(height ? { height } : {}), ...(minWidth ? { minWidth } : {}), ...(minHeight ? { minHeight } : {}) } } : {}),
  }
}
