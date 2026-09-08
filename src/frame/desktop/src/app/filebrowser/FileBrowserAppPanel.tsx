import { FileBrowserView } from './FileBrowserView'
import type { AppContentLoaderProps } from '../types'

export function FileBrowserAppPanel(props: AppContentLoaderProps) {
  return <FileBrowserView key={props.windowId} windowId={props.windowId} />
}
