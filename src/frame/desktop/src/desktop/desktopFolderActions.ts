/* Opening what the Desktop folder holds besides shortcuts and widgets (models/desktopFolder.ts): a document
 * opens with its content handler, a folder in Files. */

import { openContent } from '../app/content/open'
import { filesOpenLaunch } from '../app/filebrowser/launch'
import { normalizeCyfsPath } from '../components/preview/session'
import { desktopUIStore } from '../models/DesktopUIDataModel'
import type { FileIconItem } from '../models/ui'

/** Shows a folder (by default the Desktop folder itself) in Files. */
export function showFolderInFiles(path: string) {
  desktopUIStore.openAppWindow('files', { launch: filesOpenLaunch(path) })
}

export function openDesktopFile(item: FileIconItem) {
  if (item.fileKind === 'folder') {
    showFolderInFiles(item.path)
    return
  }
  const folder = item.path.slice(0, item.path.lastIndexOf('/')) || '/'
  const source = { kind: 'cyfs-path' as const, path: normalizeCyfsPath(item.path) }
  void openContent(
    {
      source,
      // Preview steps through the other documents on the desktop.
      session: { kind: 'container', container: { kind: 'cyfs-path', path: normalizeCyfsPath(folder) }, current: source },
      origin: { app: 'desktop', hostContext: folder },
    },
    { name: item.id, size: item.size },
  )
}
