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
  Globe,
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
  'agent-browser': Globe,
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
  monochrome = false,
  className,
  style,
}: {
  iconKey: string
  /** Image shown instead of the built-in icon; the icon stays as the fallback when it fails to load. */
  iconUrl?: string
  /** The image covers the whole icon tile instead of the glyph area. */
  fill?: boolean
  monochrome?: boolean
  className?: string
  style?: React.CSSProperties
}) {
  const Icon = iconMap[iconKey as keyof typeof iconMap] ?? LayoutGrid
  const workspaceIcon = iconKey === 'aiworkspace'
  if (workspaceIcon && monochrome) {
    const size = fill ? '70%' : '100%'
    return (
      <img
        src="/aiworkspace/icon-white.svg"
        alt=""
        draggable={false}
        className={clsx('relative z-10 shrink-0 object-contain', className)}
        style={{
          width: size,
          height: size,
          filter: fill ? undefined : 'drop-shadow(0 0 0.5px #17191cee) drop-shadow(0 1px 0.6px #12151b99)',
          ...style,
        }}
      />
    )
  }

  const size = fill ? 'var(--icon-size)' : 'calc(var(--icon-size) * 0.5)'
  return (
    <ObjectImage
      icons={workspaceIcon ? [iconUrl, '/aiworkspace/icon.svg'] : [iconUrl]}
      className={clsx('relative z-10 shrink-0', workspaceIcon ? 'object-contain' : 'object-cover', !workspaceIcon && !fill && 'rounded-full', className)}
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
