import { FileBrowserView } from './FileBrowserView'
import { filesOpenRequest } from './launch'
import type { AppContentLoaderProps } from '../types'

export function FileBrowserAppPanel(props: AppContentLoaderProps) {
  return <FileBrowserView key={props.windowId} windowId={props.windowId} openRequest={filesOpenRequest(props.launch)} />
}
