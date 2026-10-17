/* Mock-runtime stand-in for the Desktop folder (control-panel `/api/desktop` `items.*`): the folder lives in
 * this browser's localStorage, so a reload finds it again. A test can put documents or folders on the desktop
 * by writing the same key before the page loads. Behaviour follows desktop_folder.rs. The mock Files app
 * shows the folder (read-only) at the same path. */

import type { DesktopFolderEntry, DesktopFolderListing, DesktopSeedItem } from './desktop_items'

export const MOCK_DESKTOP_FOLDER_KEY = 'buckyos.mock.desktop-folder.v1'
/** The mock Files app's home is `/home` itself (no user segment), so the folder sits right under it. */
export const MOCK_DESKTOP_PATH = '/home/Desktop'

interface MockFolder {
  entries: Record<string, Omit<DesktopFolderEntry, 'name'>>
}

const OBJECT_NAME = /\.(lnk|widget)$/i

function isObjectName(name: string) {
  return OBJECT_NAME.test(name) && !name.startsWith('.') && !/[/\\]/.test(name) && name.length > 4
}

function readFolder(): MockFolder | null {
  try {
    const raw = window.localStorage.getItem(MOCK_DESKTOP_FOLDER_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as MockFolder
    return parsed && typeof parsed.entries === 'object' ? parsed : null
  } catch {
    return null
  }
}

function writeFolder(folder: MockFolder) {
  window.localStorage.setItem(MOCK_DESKTOP_FOLDER_KEY, JSON.stringify(folder))
}

function fileEntry(text: string) {
  return { kind: 'file' as const, size: new TextEncoder().encode(text).length, mtime: Math.floor(Date.now() / 1000), text }
}

/** The folder's entries as stored, or null before the desktop first listed (created) it. */
export function mockDesktopFolderEntries(): DesktopFolderEntry[] | null {
  const folder = readFolder()
  return folder ? Object.entries(folder.entries).map(([name, entry]) => ({ ...entry, name })) : null
}

export function listMockDesktopFolder(seed: DesktopSeedItem[]): DesktopFolderListing {
  let folder = readFolder()
  const created = folder === null
  if (!folder) {
    folder = { entries: Object.fromEntries(seed.filter((item) => isObjectName(item.name)).map((item) => [item.name, fileEntry(item.text)])) }
    writeFolder(folder)
  }
  const items = Object.entries(folder.entries)
    .filter(([name]) => name && !name.startsWith('.'))
    .map(([name, entry]) => ({ ...entry, name, ...(isObjectName(name) ? {} : { text: undefined }) }))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
  return { path: MOCK_DESKTOP_PATH, created, items }
}

export function writeMockDesktopItem(name: string, text: string, create: boolean) {
  if (!isObjectName(name)) throw new Error(`not a .lnk or .widget name: ${name}`)
  const folder = readFolder() ?? { entries: {} }
  if (create && folder.entries[name]) throw new Error(`exists: ${name}`)
  folder.entries[name] = fileEntry(text)
  writeFolder(folder)
}

export function deleteMockDesktopItem(name: string) {
  if (!isObjectName(name)) throw new Error(`not a .lnk or .widget name: ${name}`)
  const folder = readFolder()
  if (!folder?.entries[name]) return
  delete folder.entries[name]
  writeFolder(folder)
}
