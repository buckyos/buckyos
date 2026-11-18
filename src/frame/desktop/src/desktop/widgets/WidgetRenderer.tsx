import { useI18n } from '../../i18n/provider'
import type { WidgetItem } from '../../models/ui'
import { ClockWidget } from './ClockWidget'
import { NotepadWidget } from './NotepadWidget'
import { SystemInfoWidget } from './SystemInfoWidget'
import type { DesktopWidgetComponent } from './types'

const widgetRegistry: Record<string, DesktopWidgetComponent> = {
  clock: ClockWidget,
  notepad: NotepadWidget,
  'system-info': SystemInfoWidget,
}

function UnsupportedWidget({ item }: { item: WidgetItem }) {
  const { t } = useI18n()

  return (
    <div className="flex h-full items-center justify-center rounded-[15px] bg-[color:var(--cp-surface)] p-4 text-center text-sm text-[color:var(--cp-muted)]">
      {t('common.unsupportedPanel', item.widgetType)}
    </div>
  )
}

export function DesktopWidgetRenderer({
  item,
  onSaveNote,
}: {
  item: WidgetItem
  onSaveNote: (itemId: string, content: string) => void
}) {
  const WidgetComponent = widgetRegistry[item.widgetType]

  if (!WidgetComponent) {
    return <UnsupportedWidget item={item} />
  }

  return <WidgetComponent item={item} onSaveNote={onSaveNote} />
}
