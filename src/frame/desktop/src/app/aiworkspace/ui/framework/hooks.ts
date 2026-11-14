/* The framework in React (UI 框架 PRD §3.3, §4.6): the context actions are asked in, and the tool zones as the
 * configuration layers resolve them. */

import { useMemo, useSyncExternalStore } from 'react'
import type { Json } from '../../api/types'
import { useOutlineVersion, useStore, useWorkspaceUi } from '../../state/hooks'
import { useUserPrefs } from '../../state/prefs'
import { useSurfaceToolbar, useWorkspaceSettings } from '../../state/settings'
import type { CanvasCommands } from '../shell/MainMenu'
import { useCanvasMode, useShell } from '../shell/shellContext'
import { actionRegistry, type ActionContext } from './actions'
import { useZonesRevealed } from './appMode'
import { builtinToolbar, mergeToolbar, parseToolbarConfig, withZonesShown, type ResolvedToolbar } from './config'
import { personalToolbar } from './personal'
import './builtin'

export function useActionContext(canvas: CanvasCommands | null): ActionContext {
  const store = useStore()
  const shell = useShell()
  const mode = useCanvasMode()
  const ui = useWorkspaceUi()
  const layout = useToolbarLayout()
  useSyncExternalStore(actionRegistry.subscribe, actionRegistry.snapshot)
  useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  useOutlineVersion()
  return { store, shell, canvas, mode, openEntity: ui.openEntity, layout }
}

/** The tool zones of this window (§4.6): built-in defaults, the Workspace's shared settings, the current canvas's
 * `config.toolbar` (`surfaceToolbar` when the caller has read it already), the person's own layout across Workspaces.
 * "临时显示工具区" of App mode shows the zones the shared layers hid, in this window only. */
export function useToolbarLayout(surfaceToolbar?: Json): ResolvedToolbar {
  const shell = useShell()
  const settings = useWorkspaceSettings()
  const surface = useSurfaceToolbar(surfaceToolbar === undefined ? shell.activeSurface?.entity_id ?? null : null)
  const prefs = useUserPrefs(shell.client.transport)
  const revealed = useZonesRevealed()
  const registry = useSyncExternalStore(actionRegistry.subscribe, actionRegistry.snapshot)
  const workspaceLayer = settings.value.toolbar
  const surfaceLayer = surfaceToolbar === undefined ? surface.value : surfaceToolbar
  const personal = personalToolbar(prefs)
  const personalKey = JSON.stringify(personal)
  return useMemo(() => {
    const shared = (value: Json | null | undefined) => (revealed ? withZonesShown(parseToolbarConfig(value)) : parseToolbarConfig(value))
    return mergeToolbar([
      { layer: 'builtin', config: builtinToolbar() },
      { layer: 'workspace', config: shared(workspaceLayer) },
      { layer: 'surface', config: shared(surfaceLayer) },
      { layer: 'personal', config: JSON.parse(personalKey) as ReturnType<typeof personalToolbar> },
    ])
    // the registry's version stands for the built-in layer's actions
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceLayer, surfaceLayer, personalKey, revealed, registry])
}
