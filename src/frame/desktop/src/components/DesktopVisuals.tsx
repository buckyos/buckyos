import clsx from 'clsx'
import {
  BookOpen,
  Bot,
  Boxes,
  BrainCircuit,
  ClipboardList,
  Clock3,
  FolderOpen,
  FlaskConical,
  Waypoints,
  LayoutGrid,
  MessageSquare,
  Network,
  ScanEye,
  Share2,
  Settings,
  SlidersHorizontal,
  Sparkles,
  UserRoundPlus,
  StickyNote,
  NotebookTabs,
  Store,
  Users,
  Workflow as WorkflowIcon,
  Wrench,
} from 'lucide-react'
import type { AppDefinition } from '../models/ui'
import { panelToneClasses } from './DesktopVisualTokens'
import { ObjectImage } from '../presentation/ObjectIcon'

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
  homestation: Waypoints,
  systest: FlaskConical,
  'task-center': ClipboardList,
  workflow: WorkflowIcon,
  aiworkspace: NotebookTabs,
  preview: ScanEye,
  'users-agents': Users,
  'agent-setup': UserRoundPlus,
  'agent-guide': Sparkles,
  'my-network': Network,
  'my-content': Share2,
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
  iconUrl,
  fill = false,
  className,
  style,
}: {
  iconKey: string
  /** Image shown instead of the built-in icon; the icon stays as the fallback when it fails to load. */
  iconUrl?: string
  /** The image covers the whole icon tile instead of the glyph area. */
  fill?: boolean
  className?: string
  style?: React.CSSProperties
}) {
  const Icon = iconMap[iconKey as keyof typeof iconMap] ?? LayoutGrid
  const size = fill ? 'var(--icon-size)' : 'calc(var(--icon-size) * 0.5)'
  return (
    <ObjectImage
      icons={[iconUrl]}
      className={clsx('relative z-10 shrink-0 object-cover', fill ? '' : 'rounded-full', className)}
      style={{ width: size, height: size, ...style }}
      fallback={
        <Icon
          className={clsx('relative z-10', className)}
          style={{ width: 'calc(var(--icon-size) * 0.5)', height: 'calc(var(--icon-size) * 0.5)', ...style }}
        />
      }
    />
  )
}
