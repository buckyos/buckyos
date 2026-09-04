/* ── Editor-level actions shared by toolbars, panels and blocks ── */

import { createContext, useContext } from 'react'
import type { ContextRef, TableBlockContent } from '../domain/types'
import type { PlacementTool } from '../store/canvas-store'

export interface ConfirmAction {
  label: string
  tone?: 'primary' | 'danger' | 'subtle'
  onClick: () => void
}

export interface EditorActions {
  openImport(file?: File): void
  openFeedback(): void
  openSettings(): void
  openSnapshots(): void
  exportJson(): void
  runWish(wishId: string, opts?: { adapterId?: 'mock' | 'http' }): void
  cancelWish(wishId: string): void
  detachGroup(groupId: string): void
  deleteBlocks(ids: string[]): void
  duplicateBlocks(ids: string[]): void
  copyBlocks(ids: string[]): void
  focusBlock(id: string): void
  fitAll(): void
  zoomBy(factor: number): void
  zoomTo(zoom: number): void
  createBlockAt(type: PlacementTool, point?: { x: number; y: number }): void
  /** open the file picker and put the chosen image into an existing image block */
  pickImageFor(blockId: string): void
  setBlockImage(blockId: string, file: File): void
  /** create one image block per file (dropped / pasted); tables go to the import dialog */
  insertFiles(files: File[], point?: { x: number; y: number }): void
  /** paste the internal clipboard (blocks copied with Ctrl+C / 复制) at a canvas point */
  pasteClipboard(point?: { x: number; y: number }): void
  createTableFromContent(content: TableBlockContent, title: string): void
  createWishFromTable(tableId: string, range?: Extract<ContextRef, { kind: 'tableRange' }>['range']): void
  addStepFromViewport(pathId: string): void
  addStepFromBlocks(pathId: string, ids: string[]): void
  startPresentation(pathId: string, index?: number): void
  stopPresentation(): void
  goToStep(index: number): void
  returnToStep(): void
  confirm(opts: { title: string; body: string; actions: ConfirmAction[] }): void
}

export const EditorActionsContext = createContext<EditorActions | null>(null)

export function useEditorActions(): EditorActions {
  const ctx = useContext(EditorActionsContext)
  if (!ctx) throw new Error('EditorActionsContext missing')
  return ctx
}
