import type { AppContentLoaderProps } from '../types'
import { FilesView } from './FilesView'

export function FilesAppPanel(props: AppContentLoaderProps) {
  return (
    <FilesView
      key={`${props.locale}:${props.runtimeContainer}:${props.layoutState.formFactor}:${props.layoutState.version}`}
      embedded
      initialPath="/Desktop"
      layoutState={props.layoutState}
      locale={props.locale}
      runtimeContainer={props.runtimeContainer}
      themeMode={props.themeMode}
    />
  )
}
