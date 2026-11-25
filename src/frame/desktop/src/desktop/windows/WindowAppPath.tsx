import { useCallback, type ReactNode } from 'react'
import { AppPathContext } from '../../app/pathContext'
import { desktopUIStore } from '../../models/DesktopUIDataModel'

/** Keeps the page an app reports with its window. A new launch asks the app again. */
export function WindowAppPath({ windowId, launchId, children }: { windowId?: string; launchId?: string; children: ReactNode }) {
  const report = useCallback((path: string) => {
    if (windowId) desktopUIStore.updateWindow(windowId, { path })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [windowId, launchId])
  return <AppPathContext.Provider value={report}>{children}</AppPathContext.Provider>
}
