/* What the chrome of one open workspace shares (UI improvement §3, §12.1): the top-level view, the
 * active Surface, the right panel, dialogs, the personal layout preferences, the window size class and
 * whether this is a phone (§16). Layout preferences live in the user work state under the `ui:` prefix;
 * none of this is a document write. */

import { createContext, useContext, useSyncExternalStore } from 'react'
import type { AiwsClient } from '../../api/client'
import type { EntityEnvelope } from '../../api/types'
import { workspaceUrl } from '../../links'
import { useUserState } from '../../state/hooks'
import { CANVAS_MODES, type CanvasMode } from '../blocks/registry'
import type { OfflineActions } from '../WorkspaceView'

export type TopMode = 'sources' | 'canvas'

/** The right panel shows one kind of content at a time (§3.2). */
export type SideTab = 'inspector' | 'relations' | 'annotations' | 'collab' | 'edits' | 'wish'
export const SIDE_TAB_LABEL: Record<SideTab, string> = { inspector: '属性', relations: '引用与依赖', annotations: '批注', collab: '协作', edits: '修改状态', wish: '许愿格' }
export const CANVAS_SIDE_TABS: SideTab[] = ['inspector', 'relations', 'annotations', 'collab', 'edits']
export const SOURCES_SIDE_TABS: SideTab[] = ['collab', 'edits']

export type DialogRequest =
  | { kind: 'new'; tab: 'canvas' | 'workspace' | 'template' }
  | { kind: 'export' }
  | { kind: 'import' }
  | { kind: 'help' }
  | { kind: 'mock' }

/** Personal layout preferences (user work state, `ui:*`); "restore default layout" clears them. */
export interface LayoutPrefs { objectToolbar: boolean; presenterToolbar: boolean; grid: boolean }
export const PREF_KEYS = { objectToolbar: 'ui:object-toolbar', presenterToolbar: 'ui:presenter-toolbar', grid: 'ui:grid' } as const
export const LAYOUT_KEYS = ['ui:object-toolbar', 'ui:presenter-toolbar', 'ui:grid', 'ui:side', 'ui:pinned-defs'] as const

/** Size class of the application container (not the browser): §11 responsive rules. */
export type SizeClass = 'wide' | 'medium' | 'narrow'

// ---- phone (§16): touch is the primary pointer and the screen is small. Tablets and touch laptops are not phones.

const COARSE_POINTER = '(pointer: coarse)'
/** The short side of a phone screen is below this (CSS px); tablets start around 740. */
const PHONE_SHORT_SIDE = 600

function phoneNow(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false
  return window.matchMedia(COARSE_POINTER).matches && Math.min(window.screen.width, window.screen.height) < PHONE_SHORT_SIDE
}

function subscribePhone(listener: () => void): () => void {
  const query = window.matchMedia(COARSE_POINTER)
  query.addEventListener('change', listener)
  window.addEventListener('resize', listener)
  return () => { query.removeEventListener('change', listener); window.removeEventListener('resize', listener) }
}

export function usePhone(): boolean {
  return useSyncExternalStore(subscribePhone, phoneNow, () => false)
}

export interface ShellApi {
  /** Workspace-level calls outside the open session (create, fork, import, export). */
  client: AiwsClient
  /** Leave the workspace (after the leave check) and show the list. */
  close: () => void
  /** Leave this workspace and open another one (new workspace, template result, fork). */
  openWorkspace: (workspaceId: string) => void
  offline: OfflineActions
  topMode: TopMode
  setTopMode: (mode: TopMode) => void
  surfaces: EntityEnvelope[]
  activeSurface: EntityEnvelope | null
  selectSurface: (surfaceId: string) => void
  side: SideTab | null
  setSide: (tab: SideTab | null) => void
  openDialog: (request: DialogRequest) => void
  prefs: LayoutPrefs
  setPref: (key: keyof LayoutPrefs, value: boolean) => void
  resetLayout: () => void
  size: SizeClass
  /** A phone (§16): the canvas only, in view mode, with one toolbar; nothing about it is written to the shared work state. */
  phone: boolean
  /** The Mock processing panel and other developer tools are shown (dev server or dev override). */
  devTools: boolean
  /** Who this window acts as; a dev-override identity has no login session to end. */
  identity: { principal: string | null; dev: boolean }
  /** Log out through the Desktop's sign-in flow (after the workspace's leave check). */
  logout: () => void
  home: (() => void) | null
  /** Offline actions (prepare, take over, reopen) run one at a time; their failure stays visible. */
  runOffline: (label: string, work: () => Promise<void>) => void
  offlineBusy: string | null
  offlineError: string | null
  clearOfflineError: () => void
}

export const ShellContext = createContext<ShellApi | null>(null)

/** The canvas sub-mode (user work state `canvas:mode`); a phone always views (§16) and leaves the stored mode alone. */
export function useCanvasMode(): CanvasMode {
  const phone = useContext(ShellContext)?.phone ?? false
  const state = useUserState<CanvasMode>('canvas:mode')
  if (phone) return 'view'
  return state && CANVAS_MODES.includes(state) ? state : 'edit'
}

export const ZOOM_PRESETS = [25, 50, 75, 100, 150, 200]

/** An access link (§8.3): it names the workspace and Surface, carries no credential and grants nothing. */
export function shareLink(workspaceId: string, surfaceId: string | null): string {
  return workspaceUrl(workspaceId, { surfaceId })
}

export function useShell(): ShellApi {
  const shell = useContext(ShellContext)
  if (!shell) throw new Error('aiworkspace: no workspace shell in context')
  return shell
}
