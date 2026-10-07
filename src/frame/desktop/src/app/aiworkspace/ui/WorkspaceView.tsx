/* One open workspace: the store in context, the shell on top (phase two §5). Block definitions are
 * registered once per app lifetime; extensions register the same way. */

import { StoreContext } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { registerDefaultBlocks } from './blocks/registerAll'
import { WorkspaceShell, type ShellProps } from './shell/WorkspaceShell'

/** What the shell can ask of the app panel: prepare the offline replica, reopen the workspace in the mode that is possible now. */
export interface OfflineActions {
  prepare(workspaceId: string, discardLocal: boolean): Promise<void>
  reopen(workspaceId: string): Promise<void>
  describe(failure: unknown): string
}

export function WorkspaceView({ store, ...shell }: { store: WorkspaceStore } & ShellProps) {
  registerDefaultBlocks()
  return (
    <StoreContext.Provider value={store}>
      <WorkspaceShell {...shell} />
    </StoreContext.Provider>
  )
}
