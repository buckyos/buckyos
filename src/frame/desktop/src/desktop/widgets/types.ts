import type { ComponentType } from 'react'
import type { WidgetItem } from '../../models/ui'

export interface DesktopWidgetProps {
  item: WidgetItem
  /** Saves part of the widget's own settings into its `.widget` file. */
  onUpdateConfig: (itemId: string, patch: Record<string, unknown>) => void
}

export type DesktopWidgetComponent = ComponentType<DesktopWidgetProps>
