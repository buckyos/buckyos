import clsx from 'clsx'
import {
  BookOpen,
  Bot,
  Boxes,
  BrainCircuit,
  ClipboardList,
  Clock3,
  FolderOpen,
  Home,
  LayoutGrid,
  MessageSquare,
  Settings,
  SlidersHorizontal,
  StickyNote,
  Store,
  Users,
  Workflow as WorkflowIcon,
  Wrench,
} from 'lucide-react'
import type { AppDefinition } from '../models/ui'
import { panelToneClasses } from './DesktopVisualTokens'

const iconMap = {
  'ai-center': BrainCircuit,
  'app-service': Boxes,
  settings: Settings,
  files: FolderOpen,
  studio: Wrench,
  market: Store,
  diagnostics: LayoutGrid,
  demos: SlidersHorizontal,
  docs: BookOpen,
  codeassistant: Bot,
  messagehub: MessageSquare,
  homestation: Home,
  'task-center': ClipboardList,
  workflow: WorkflowIcon,
  'users-agents': Users,
  clock: Clock3,
  notepad: StickyNote,
}

export function TierBadge({ tier }: { tier: AppDefinition['tier'] }) {
  const tone: keyof typeof panelToneClasses =
    tier === 'system' ? 'accent' : tier === 'sdk' ? 'success' : 'warning'

  return (
    <span
      className={clsx(
        'inline-flex rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-[0.18em]',
        panelToneClasses[tone],
      )}
    >
      {tier}
    </span>
  )
}

export function AppIcon({
  iconKey,
  className,
  style,
}: {
  iconKey: string
  className?: string
  style?: React.CSSProperties
}) {
  const Icon = iconMap[iconKey as keyof typeof iconMap] ?? LayoutGrid
  return (
    <Icon
      className={clsx('relative z-10', className)}
      style={{ width: 'calc(var(--icon-size) * 0.5)', height: 'calc(var(--icon-size) * 0.5)', ...style }}
    />
  )
}
