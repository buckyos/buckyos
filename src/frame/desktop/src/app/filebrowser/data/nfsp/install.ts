/**
 * Wires the NFSP-backed data sources into the File Browser registries —
 * the real-backend counterpart of the mock registrations (UI_DATAMODEL.md
 * §8.1). Components keep consuming FolderReader/FileItemList and the
 * data-layer registries; no NFSP wire type crosses this boundary.
 */

import type { FileItem } from '../FolderReader'
import { registerPreviewEnricher } from '../usePreview'
import { ensureSession, nfspClient } from './client'
import { registerNfspCollectionReader } from './collections'
import { registerNfspCollectionDirectory } from './collectionDirectory'
import { registerNfspFolderOps } from './folderOps'
import { applyMetaRecords } from './mapping'
import { registerNfspReaders } from './readers'
import { registerNfspSearchProvider } from './search'
import { registerNfspSidebarSources } from './sidebar'
import { registerNfspTransferExecutor } from './transfers'

/** get_meta enrichment for the preview panel (§8.2 meta mapping). */
async function enrichPreviewItem(item: FileItem): Promise<FileItem> {
  await ensureSession()
  const meta = await nfspClient()
    .getMeta(item.entry.path.startsWith('/') ? item.entry.path : { uri: item.entry.path })
    .catch(() => null)
  if (!meta || meta.records.length === 0) return item
  return { ...item, entry: applyMetaRecords(item.entry, meta.records) }
}

export function installNfspFileBrowserData() {
  registerNfspReaders()
  registerNfspCollectionReader()
  registerNfspCollectionDirectory()
  registerNfspSidebarSources()
  registerNfspSearchProvider()
  registerNfspTransferExecutor()
  registerNfspFolderOps()
  registerPreviewEnricher(enrichPreviewItem)
  // Establish the session (and auto-watch) eagerly so the first listings and
  // watch invalidation don't wait for a lazy hello.
  void ensureSession().catch(() => {
    // Surfaced per-source by the readers when the user actually navigates.
  })
}
