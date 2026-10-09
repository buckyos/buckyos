/* eslint-disable react-refresh/only-export-components -- a preset table with its renderer */
/* Preset canvas icons (UI improvement §5.1): the document stores only the stable id; this table is
 * the client's preset list. An id the client does not know shows the default of the Surface's layout. */

import { createElement } from 'react'
import { BookOpen, Briefcase, Calendar, ChartColumn, Clapperboard, Code, Compass, FileText, Flag, Globe, Heart, Kanban, LayoutDashboard, Lightbulb, ListTodo, Map as MapIcon, NotebookPen, Palette, Rocket, Sparkles, Star, Target, type LucideIcon } from 'lucide-react'
import type { EntityEnvelope } from '../../api/types'

export const SURFACE_ICONS: { id: string; label: string; Icon: LucideIcon }[] = [
  { id: 'board', label: '白板', Icon: LayoutDashboard },
  { id: 'page', label: '页面', Icon: FileText },
  { id: 'chart', label: '图表', Icon: ChartColumn },
  { id: 'kanban', label: '看板', Icon: Kanban },
  { id: 'todo', label: '清单', Icon: ListTodo },
  { id: 'notes', label: '笔记', Icon: NotebookPen },
  { id: 'book', label: '资料', Icon: BookOpen },
  { id: 'idea', label: '想法', Icon: Lightbulb },
  { id: 'ai', label: 'AI', Icon: Sparkles },
  { id: 'rocket', label: '项目', Icon: Rocket },
  { id: 'target', label: '目标', Icon: Target },
  { id: 'flag', label: '里程碑', Icon: Flag },
  { id: 'calendar', label: '日程', Icon: Calendar },
  { id: 'map', label: '地图', Icon: MapIcon },
  { id: 'compass', label: '探索', Icon: Compass },
  { id: 'globe', label: '市场', Icon: Globe },
  { id: 'work', label: '工作', Icon: Briefcase },
  { id: 'code', label: '代码', Icon: Code },
  { id: 'film', label: '影像', Icon: Clapperboard },
  { id: 'design', label: '设计', Icon: Palette },
  { id: 'star', label: '收藏', Icon: Star },
  { id: 'heart', label: '生活', Icon: Heart },
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
