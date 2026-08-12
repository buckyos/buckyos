import type { ComponentType } from 'react'
import type { WidgetItem } from '../../models/ui'

export interface DesktopWidgetProps {
  item: WidgetItem
  onSaveNote: (itemId: string, content: string) => void
}

export type DesktopWidgetComponent = ComponentType<DesktopWidgetProps>
