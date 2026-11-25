import { FileBrowserView } from './FileBrowserView'
import { forgetBrowserSessions } from './data/useBrowserPane'
import { filesOpenRequest } from './launch'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import type { AppContentLoaderProps } from '../types'

let openWindows = desktopUIStore.getSnapshot().runtime.windows
desktopUIStore.subscribe(() => {
  const { windows } = desktopUIStore.getSnapshot().runtime
  if (windows === openWindows) return
  openWindows = windows
  forgetBrowserSessions((windowId) => windows.some((w) => w.id === windowId))
})

export function FileBrowserAppPanel(props: AppContentLoaderProps) {
  return <FileBrowserView key={props.windowId} windowId={props.windowId} openRequest={filesOpenRequest(props.launch)} />
}
