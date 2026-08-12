/**
 * Icon mapping for the renderer-agnostic `FileMenuIcon` ids — shared by the
 * desktop popup renderer and the mobile action-sheet renderer.
 */

import {
  ArrowDown,
  ArrowUp,
  Copy,
  CornerUpRight,
  Download,
  ExternalLink,
  Eye,
  FolderInput,
  FolderOpen,
  FolderPlus,
  LayoutGrid,
  Library,
  Link2,
  List,
  PanelRight,
  PencilLine,
  RefreshCw,
  Share2,
  SquareDashedMousePointer,
  Trash2,
  Unlink,
  Upload,
} from 'lucide-react'
import type { FileMenuIcon } from './types'

export const MENU_ICONS: Record<FileMenuIcon, React.ComponentType<{ size?: number | string }>> = {
  open: FolderOpen,
  'open-new-tab': ExternalLink,
  'open-right': PanelRight,
  preview: Eye,
  copy: Copy,
  link: Link2,
  share: Share2,
  download: Download,
  rename: PencilLine,
  move: FolderInput,
  trash: Trash2,
  'new-folder': FolderPlus,
  upload: Upload,
  refresh: RefreshCw,
  'select-all': SquareDashedMousePointer,
  'view-list': List,
  'view-icon': LayoutGrid,
  collection: Library,
  'remove-ref': Unlink,
  jump: CornerUpRight,
  broken: Unlink,
  'move-up': ArrowUp,
  'move-down': ArrowDown,
}
