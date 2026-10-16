/* eslint-disable react-refresh/only-export-components -- a preset table with its renderer */
/* Preset canvas icons (UI improvement §5.1): the document stores only the stable id; this table is
 * the client's preset list. An id the client does not know shows the default of the Surface's layout. */

import { createElement } from 'react'
import { BookOpen, Briefcase, Calendar, ChartColumn, Clapperboard, Code, Compass, FileText, Flag, Globe, Heart, Kanban, LayoutDashboard, Lightbulb, ListTodo, Map as MapIcon, NotebookPen, Palette, Rocket, Sparkles, Star, Target, type LucideIcon } from 'lucide-react'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'

/** A preset whose name is read in the current language. */
const preset = (id: string, key: string, Icon: LucideIcon) => ({ id, get label() { return aiwsT(key) }, Icon })

export const SURFACE_ICONS: { id: string; label: string; Icon: LucideIcon }[] = [
  preset('board', 'aiworkspace.canvas.icon.board', LayoutDashboard),
  preset('page', 'aiworkspace.canvas.icon.page', FileText),
  preset('chart', 'aiworkspace.canvas.icon.chart', ChartColumn),
  preset('kanban', 'aiworkspace.canvas.icon.kanban', Kanban),
  preset('todo', 'aiworkspace.canvas.icon.todo', ListTodo),
  preset('notes', 'aiworkspace.canvas.icon.notes', NotebookPen),
  preset('book', 'aiworkspace.canvas.icon.book', BookOpen),
  preset('idea', 'aiworkspace.canvas.icon.idea', Lightbulb),
  { id: 'ai', label: 'AI', Icon: Sparkles },
  preset('rocket', 'aiworkspace.canvas.icon.rocket', Rocket),
  preset('target', 'aiworkspace.canvas.icon.target', Target),
  preset('flag', 'aiworkspace.canvas.icon.flag', Flag),
  preset('calendar', 'aiworkspace.canvas.icon.calendar', Calendar),
  preset('map', 'aiworkspace.canvas.icon.map', MapIcon),
  preset('compass', 'aiworkspace.canvas.icon.compass', Compass),
  preset('globe', 'aiworkspace.canvas.icon.globe', Globe),
  preset('work', 'aiworkspace.canvas.icon.work', Briefcase),
  preset('code', 'aiworkspace.canvas.icon.code', Code),
  preset('film', 'aiworkspace.canvas.icon.film', Clapperboard),
  preset('design', 'aiworkspace.canvas.icon.design', Palette),
  preset('star', 'aiworkspace.canvas.icon.star', Star),
  preset('heart', 'aiworkspace.canvas.icon.heart', Heart),
]

const BY_ID = new Map(SURFACE_ICONS.map((icon) => [icon.id, icon]))

export function surfaceIcon(surface: Pick<EntityEnvelope, 'icon' | 'layout'> | undefined): LucideIcon {
  const preset = surface?.icon ? BY_ID.get(surface.icon) : undefined
  if (preset) return preset.Icon
  return surface?.layout?.mode === 'flow' ? FileText : LayoutDashboard
}

export function SurfaceIcon({ surface, size = 18 }: { surface: Pick<EntityEnvelope, 'icon' | 'layout'> | undefined; size?: number }) {
  return createElement(surfaceIcon(surface), { size, 'aria-hidden': true })
}
