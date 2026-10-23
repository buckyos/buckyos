import type { ndm } from 'buckyos'
import type { LocalUpload } from './transfers'

export function selectedUploads(files: readonly File[]): LocalUpload[] {
  const folders = new Map<string, File[]>()
  const result: LocalUpload[] = []
  for (const file of files) {
    if (!file.webkitRelativePath) { result.push({ kind: 'file', file }); continue }
    const root = file.webkitRelativePath.split('/')[0]
    if (!folders.has(root)) {
      const files: File[] = []
      folders.set(root, files)
      result.push({ kind: 'folder', files })
    }
    folders.get(root)!.push(file)
  }
  return result
}

export function describeUpload(input: LocalUpload): { name: string; sizeBytes: number; mimeType?: string } {
  if (input.kind === 'file') return { name: input.file.name, sizeBytes: input.file.size, mimeType: input.file.type || undefined }
  if (input.kind === 'folder') return { name: input.files[0].webkitRelativePath.split('/')[0], sizeBytes: input.files.reduce((sum, file) => sum + file.size, 0) }
  const size = (node: ndm.ImportTreeEntry): number => node.kind === 'file' ? node.file.size : node.children.reduce((sum, child) => sum + size(child), 0)
  return { name: input.tree.name, sizeBytes: size(input.tree) }
}

async function readEntry(entry: FileSystemEntry): Promise<ndm.ImportTreeEntry> {
  if (entry.isFile) return { kind: 'file', file: await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject)) }
  if (!entry.isDirectory) throw new Error(`Unsupported dropped entry: ${entry.name}`)
  const reader = (entry as FileSystemDirectoryEntry).createReader()
  const children: ndm.ImportTreeEntry[] = []
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject))
    if (batch.length === 0) break
    for (const child of batch) children.push(await readEntry(child))
  }
  return { kind: 'dir', name: entry.name, children }
}

export async function droppedUploads(data: DataTransfer): Promise<LocalUpload[]> {
  const items = [...data.items].filter(item => item.kind === 'file')
  const entries = items.map(item => ({ entry: item.webkitGetAsEntry?.(), file: item.getAsFile() }))
  if (entries.length === 0) return selectedUploads([...data.files])
  const result: LocalUpload[] = []
  for (const item of entries) {
    if (item.entry) {
      const node = await readEntry(item.entry)
      result.push(node.kind === 'file' ? node : { kind: 'tree', tree: node })
    } else if (item.file) result.push({ kind: 'file', file: item.file })
    else throw new Error('This browser could not enumerate a dropped entry; choose the folder again')
  }
  return result
}
