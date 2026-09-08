import type { FileMenuContext } from './types'
import { folderOps } from '../data/folderOps'

export interface CommandState {
  state: 'available' | 'denied' | 'unsupported' | 'loading'
  reason?: string
}
const available: CommandState = { state: 'available' }
export function commandState(command: string, ctx: FileMenuContext): CommandState {
  if (ctx.capabilities.availability === 'loading' && ['cut','rename','move-to','move-other','delete','paste','upload','new-folder','remove-from-collection'].includes(command)) return { state: 'loading', reason: 'Permissions are loading' }
  const unsupported = (reason: string): CommandState => ({ state: 'unsupported', reason })
  if (['share', 'settings', 'new-file', 'camera'].includes(command)) return unsupported('This service does not support this action')
  if (command === 'copy' && !folderOps().supportsCopy) return unsupported('File content copying is not supported. Use Add to Collection for references.')
  if (command === 'paste') {
    if (!ctx.clipboard) return unsupported('The clipboard is empty')
    if (ctx.capabilities.acceptsReferences && ctx.clipboard.mode !== 'copy') return unsupported('Cut moves files. Use Add existing files to add references.')
    if (!ctx.capabilities.acceptsReferences && (!ctx.capabilities.acceptsContent || ctx.searching)) return unsupported('Choose a writable destination folder')
    if (!ctx.capabilities.acceptsReferences && ctx.clipboard.mode === 'copy' && !folderOps().supportsCopy) return unsupported('File content copying is not supported')
  }
  if (ctx.busy && ['cut', 'paste', 'rename', 'move-to', 'move-other', 'delete', 'remove-from-collection', 'download'].includes(command)) return { state: 'loading', reason: 'Wait for the current operation to finish' }
  if (['rename', 'cut', 'move-to', 'move-other', 'delete'].includes(command)) {
    if (ctx.searching) return unsupported('Open the original location to change this file')
    if (!ctx.capabilities.acceptsContent || (command === 'delete' && ctx.capabilities.removal !== 'destroy')) return { state: 'denied', reason: 'This location is read-only' }
    if (ctx.entries.length === 0) return unsupported('Select a file first')
    if (command === 'rename' && ctx.entries.length !== 1) return unsupported('Select one file to rename')
  }
  if (['upload', 'new-folder'].includes(command) && (!ctx.capabilities.acceptsContent || ctx.searching)) return unsupported('Choose a writable folder')
  if (command === 'add-existing' && !ctx.capabilities.acceptsReferences) return unsupported('This location does not accept references')
  if (['remove-from-collection', 'remove-broken'].includes(command) && (ctx.capabilities.removal !== 'remove-ref' || ctx.searching)) return unsupported('Select references in a writable collection')
  const op = command === 'cut' || command === 'move-to' || command === 'move-other' ? 'move' : command
  if (['rename', 'move', 'delete', 'download', 'share', 'copy'].includes(op)) {
    for (const entry of ctx.entries) {
      const value = entry.operations?.[op as keyof NonNullable<typeof entry.operations>]
      if (value && value !== 'available') return { state: value, reason: value === 'loading' ? 'Permissions are loading' : value === 'denied' ? 'You do not have permission for this action' : 'This item does not support this action' }
    }
  }
  if (ctx.items.some((item) => item.ref?.broken || item.entry.link?.broken) && ['open', 'preview-new-window', 'download', 'cut', 'rename', 'move-to'].includes(command)) return unsupported('The original file is unavailable')
  return available
}
