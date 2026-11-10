import { useMemo } from 'react'
import type { LocalFile, ZoneContent } from 'buckyos/content'
import { useSelectContent } from '../../../../sysdlg'
import { readZoneFiles, readZoneFolder } from '../../../../sysdlg/select/zoneFiles'

/**
 * Files to import, chosen in the system file picker from this device or from the Zone. Zone files are read into
 * `File` objects so both sources go through the same import. Null where there is no Desktop window (a Workspace's
 * own-origin page): the caller keeps its native file inputs there. Resolves with null when the user cancels.
 */
export function useImportPicker(): ((folder: boolean, signal?: AbortSignal) => Promise<File[] | null>) | null {
  const select = useSelectContent()
  return useMemo(() => select ? async (folder, signal) => {
    const result = await select(folder ? { mode: 'folder', allowLocal: true } : { mode: 'open', multiple: true, allowLocal: true }, { id: 'aiworkspace' })
    if (result.action !== 'selected') return null
    const first = result.items[0]
    if (first.origin === 'local') return first.kind === 'folder' ? first.files.map(entry => entry.file) : (result.items as LocalFile[]).map(item => item.file)
    return folder ? readZoneFolder(first, signal) : readZoneFiles(result.items as ZoneContent[], signal)
  } : null, [select])
}
