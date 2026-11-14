/* Icons of actions and tool items by name (UI 框架 PRD §3.3, §4.6): a configuration names an icon with a string
 * (`"icon": "calendar-plus"`), so actions declare theirs the same way. A short fixed table, not the whole icon set. */

import {
  CalendarPlus, Code, Database, Ellipsis, FlaskConical, Link2, MessageSquarePlus, Play, Plus, Presentation, Signpost, Sparkles, Star,
  Users, WandSparkles, Zap, type LucideIcon,
} from 'lucide-react'

export const ACTION_ICONS: Readonly<Record<string, LucideIcon>> = {
  play: Play,
  presentation: Presentation,
  guide: Signpost,
  macro: Zap,
  wand: WandSparkles,
  comment: MessageSquarePlus,
  share: Link2,
  users: Users,
  data: Database,
  more: Ellipsis,
  plus: Plus,
  code: Code,
  star: Star,
  sparkles: Sparkles,
  flask: FlaskConical,
  'calendar-plus': CalendarPlus,
}

/** An icon name, or the icon itself. */
export type IconRef = string | LucideIcon

export function iconOf(icon: IconRef | undefined): LucideIcon | null {
  if (!icon) return null
  return typeof icon === 'string' ? ACTION_ICONS[icon] ?? null : icon
}
