/* ── The signed-in user's Desktop folder (control-panel `/api/desktop`, `items.*`) ──
 *
 * Like Windows, the desktop shows a folder in the user's home (`/home/{user}/Desktop`, also seen in Files), so
 * every device shows the same items and only keeps where they sit. Shortcuts (`*.lnk`) and widgets
 * (`*.widget`) are small JSON files; any other entry is a document or folder. See desktop_folder.rs.
 * The mock runtime keeps the folder in this browser.
 */

import { isMockRuntime } from '../runtime'
import { DESKTOP_API, sendDesktopApi } from './desktop_state'

export interface DesktopFolderEntry {
  name: string
  kind: 'file' | 'dir'
  size: number
  /** Seconds since the epoch. */
  mtime: number
  /** Content of a `.lnk` / `.widget` file. */
  text?: string
}

export interface DesktopFolderListing {
  /** The folder as Files shows it, e.g. `/home/alice/Desktop`. */
  path: string
  /** True when this call created the folder (and wrote the seed). */
  created: boolean
  items: DesktopFolderEntry[]
}

export interface DesktopSeedItem {
  name: string
  text: string
}

/** Lists the folder; a missing folder is created with `seed` in it. */
export async function listDesktopFolder(seed: DesktopSeedItem[]): Promise<DesktopFolderListing> {
  if (isMockRuntime()) {
    const { listMockDesktopFolder } = await import('./desktop_items_mock')
    return listMockDesktopFolder(seed)
  }
  return sendDesktopApi<DesktopFolderListing>(DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'items.list', seed }),
  })
}

/** Writes a shortcut or widget file; with `create` an existing file is kept and the call fails. */
export async function writeDesktopItem(name: string, text: string, options: { create?: boolean } = {}): Promise<void> {
  if (isMockRuntime()) {
    const { writeMockDesktopItem } = await import('./desktop_items_mock')
    writeMockDesktopItem(name, text, options.create ?? false)
    return
  }
  await sendDesktopApi(DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'items.write', name, text, create: options.create ?? false }),
    keepalive: true,
  })
}

/** Removes a shortcut or widget file. */
export async function deleteDesktopItem(name: string): Promise<void> {
  if (isMockRuntime()) {
    const { deleteMockDesktopItem } = await import('./desktop_items_mock')
    deleteMockDesktopItem(name)
    return
  }
  await sendDesktopApi(DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'items.delete', name }),
  })
}
