/**
 * The widgets the desktop offers, as data: name, sizes in grid cells, and whether the desktop may hold more than
 * one. The components are in WidgetRenderer.tsx; this file stays free of React so the layout model can use it.
 */

export interface WidgetSize {
  w: number
  h: number
}

export interface WidgetDefinition {
  type: string
  labelKey: string
  /** English name; also the stem of the `.widget` file when the translation is not usable as a file name. */
  label: string
  descriptionKey: string
  description: string
  defaultSize: WidgetSize
  minSize: WidgetSize
  maxSize: WidgetSize
  /** A note is content, so the desktop takes as many as the user adds; the others would show the same thing twice. */
  multiple?: boolean
}

export const widgetCatalog: readonly WidgetDefinition[] = [
  {
    type: 'clock',
    labelKey: 'widgets.clock',
    label: 'Clock',
    descriptionKey: 'desktop.add.widget.clock',
    description: 'The time, weekday and date.',
    defaultSize: { w: 2, h: 1 },
    minSize: { w: 2, h: 1 },
    maxSize: { w: 4, h: 2 },
  },
  {
    type: 'notepad',
    labelKey: 'widgets.notepad',
    label: 'Notepad',
    descriptionKey: 'desktop.add.widget.notepad',
    description: 'A note, saved in your Desktop folder.',
    defaultSize: { w: 2, h: 2 },
    minSize: { w: 2, h: 2 },
    maxSize: { w: 4, h: 4 },
    multiple: true,
  },
  {
    type: 'system-info',
    labelKey: 'widgets.systemInfo',
    label: 'System Info',
    descriptionKey: 'desktop.add.widget.systemInfo',
    description: 'The system version, signed-in account and Zone.',
    defaultSize: { w: 2, h: 3 },
    minSize: { w: 2, h: 2 },
    maxSize: { w: 4, h: 4 },
  },
]

/** The size of a widget type the desktop does not know. */
const unknownWidgetSize: WidgetSize = { w: 2, h: 2 }

export function widgetDefinition(type: string): WidgetDefinition | undefined {
  return widgetCatalog.find((definition) => definition.type === type)
}

export function defaultWidgetSize(type: string): WidgetSize {
  return { ...(widgetDefinition(type)?.defaultSize ?? unknownWidgetSize) }
}

export function isResizableWidget(type: string): boolean {
  const definition = widgetDefinition(type)
  return Boolean(
    definition &&
      (definition.minSize.w !== definition.maxSize.w || definition.minSize.h !== definition.maxSize.h),
  )
}

/** The size limits of a widget: its own, and never more than the grid. A type the desktop does not know has only the grid's. */
export function widgetSizeLimits(type: string, grid?: { cols: number; rows: number }): { min: WidgetSize; max: WidgetSize } {
  const definition = widgetDefinition(type)
  const max = definition ? { ...definition.maxSize } : { w: Infinity, h: Infinity }
  const min = definition ? { ...definition.minSize } : { w: 1, h: 1 }
  if (grid) {
    max.w = Math.max(1, Math.min(max.w, grid.cols))
    max.h = Math.max(1, Math.min(max.h, grid.rows))
  }
  min.w = Math.min(min.w, max.w)
  min.h = Math.min(min.h, max.h)
  return { min, max }
}

/** A size kept within the widget's limits (a saved size from before the limits changed, a size asked for). */
export function clampWidgetSize(type: string, size: WidgetSize, grid?: { cols: number; rows: number }): WidgetSize {
  const { min, max } = widgetSizeLimits(type, grid)
  return {
    w: Math.min(max.w, Math.max(min.w, size.w)),
    h: Math.min(max.h, Math.max(min.h, size.h)),
  }
}
