/* The main menu (UI 框架 PRD §4.1.2): the complete index of functions. The first level holds the groups only (at most
 * nine rows, so it fits a 480px-high window without scrolling), the second level at most twelve entries. The entries
 * are the registered actions (ui/framework): every entry calls the same action as its button, context menu entry or
 * shortcut, and one that cannot run now is disabled and says why. A new function joins a group's second level by
 * declaring it; a new first-level group needs the PRD changed first. */

import type { Camera } from '../canvas/render/camera'
import type { CatalogEntry } from '../canvas/catalog'
import type { CatalogTab } from '../canvas/InsertCatalog'
import { buildMenu, type ActionContext } from '../framework/actions'
import '../framework/builtin'
import type { MenuItem } from './popover'
import type { SideTab } from './shellContext'

export interface Command { reason: string | null; run: () => void }

/** 评论 (Q14): comment on the selection or pick a target; a free canvas also takes notes (`note`). `active`: a target is
 * being picked (the same command cancels the pick). */
export interface CommentCommand { reason: string | null; active: boolean; run: () => void; note?: Command | null }

/** What the open canvas offers the menus and tool zones (absent in data mode and without a Surface). */
export interface CanvasCommands {
  isFree: boolean
  /** Why inserting is not possible now, or null. */
  insertReason: string | null
  /** The same for notes, which need only `comment` (公开链接便签). */
  noteReason: string | null
  insert: (entry: CatalogEntry) => void
  openCatalog: (tab: CatalogTab, key?: string) => void
  copy: Command
  cut: Command
  paste: Command
  remove: Command
  /** draw.io files (draw.io 导入 TODO §3): import into this canvas or new ones; export this canvas. */
  importDrawio: Command
  /** PDF files (PDF 导入方案 §5): pages on a new canvas with a presentation path. */
  importPdf: Command
  /** Office documents (Office 导入 TODO §6): a docx as a flow page, a pptx as pages on a new canvas. */
  importOffice: Command
  /** Table data (表格数据导入 TODO §5.1): CSV / xlsx / SQLite as a table on this canvas. */
  importTable: Command
  exportDrawio: Command
  /** Free Surfaces only. */
  view: { camera: Camera; fitAll: () => void; fitSelection: () => void; hasSelection: boolean } | null
  canvasTabs: SideTab[]
  comment?: CommentCommand | null
  /** 手写 on a phone's free canvas (UI 框架 PRD §6.3): absent where there is no writing (a flow page, a computer). */
  ink?: Command | null
}

/** Built when the menu opens, in the language of that moment. */
export function buildMainMenu(ctx: ActionContext): MenuItem[] {
  return buildMenu(ctx)
}
