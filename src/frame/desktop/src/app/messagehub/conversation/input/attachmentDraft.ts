import type { VoiceNote } from '../../protocol/msgobj'

export const MAX_ATTACHMENT_BYTES = 64 * 1024 * 1024

export interface AttachmentDirectory {
  kind: 'dir'
  name: string
  children: (AttachmentDirectory | { kind: 'file'; file: File })[]
}

export interface ComposerAttachmentInput {
  directory?: AttachmentDirectory
  file: File
  relativePath?: string
  /** A recorded voice note: sent alone it goes out as one (`MsgObject.voice`), next to other files as an audio file. */
  voice?: VoiceNote
}

export interface ComposerAttachmentItem extends ComposerAttachmentInput {
  id: string
  kind: 'image' | 'file' | 'dir'
  previewUrl?: string
}

interface FileSystemEntryLike {
  isFile: boolean
  isDirectory: boolean
  fullPath?: string
}

interface FileSystemFileEntryLike extends FileSystemEntryLike {
  file: (
    successCallback: (file: File) => void,
    errorCallback?: (error: DOMException) => void,
  ) => void
}

interface FileSystemDirectoryEntryLike extends FileSystemEntryLike {
  createReader: () => FileSystemDirectoryReaderLike
}

interface FileSystemDirectoryReaderLike {
  readEntries: (
    successCallback: (entries: FileSystemEntryLike[]) => void,
    errorCallback?: (error: DOMException) => void,
  ) => void
}

type DataTransferItemWithEntry = DataTransferItem & {
  webkitGetAsEntry?: () => FileSystemEntryLike | null
}

export function isTransferWithFiles(dataTransfer: DataTransfer | null): boolean {
  if (!dataTransfer) {
    return false
  }

  if (Array.from(dataTransfer.types).includes('Files')) {
    return true
  }

  return Array.from(dataTransfer.items).some((item) => item.kind === 'file')
}

export function filesFromInputList(
  files: FileList | null,
): ComposerAttachmentInput[] {
  if (!files) {
    return []
  }

  const selected = Array.from(files)
  const roots = new Map<string, AttachmentDirectory>()
  const loose: ComposerAttachmentInput[] = []
  for (const file of selected) {
    const parts = file.webkitRelativePath.split('/').filter(Boolean)
    if (parts.length < 2) { loose.push({ file }); continue }
    let directory = roots.get(parts[0])
    if (!directory) { directory = { kind: 'dir', name: parts[0], children: [] }; roots.set(parts[0], directory) }
    for (const name of parts.slice(1, -1)) {
      let child = directory.children.find(item => item.kind === 'dir' && item.name === name) as AttachmentDirectory | undefined
      if (!child) { child = { kind: 'dir', name, children: [] }; directory.children.push(child) }
      directory = child
    }
    directory.children.push({ kind: 'file', file })
  }
  return [...loose, ...Array.from(roots.values(), directoryAttachment)]
}

export async function extractTransferFiles(
  dataTransfer: DataTransfer,
): Promise<ComposerAttachmentInput[]> {
  const transferItems = Array.from(dataTransfer.items)

  if (transferItems.length === 0) {
    return filesFromInputList(dataTransfer.files)
  }

  const items = await Promise.all(
    transferItems.map(async (item) => {
      if (item.kind !== 'file') {
        return []
      }

      const entry = (item as DataTransferItemWithEntry).webkitGetAsEntry?.()

      if (entry) {
        return [await collectEntry(entry)]
      }

      const file = item.getAsFile()
      return file ? [{ file }] : []
    }),
  )

  return items.flat()
}

export function createAttachmentItem(
  input: ComposerAttachmentInput,
): ComposerAttachmentItem {
  const previewUrl = input.file.type.startsWith('image/')
    ? URL.createObjectURL(input.file)
    : undefined

  return {
    id: createAttachmentId(input),
    file: input.file,
    kind: input.directory ? 'dir' : input.file.type.startsWith('image/') ? 'image' : 'file',
    directory: input.directory,
    previewUrl,
    relativePath: input.relativePath,
    ...(input.voice ? { voice: input.voice } : {}),
  }
}

export function getAttachmentKey(
  input: ComposerAttachmentInput,
): string {
  const fingerprint = (entry: AttachmentDirectory | { kind: 'file'; file: File }): unknown => entry.kind === 'file' ? [entry.file.name, entry.file.size, entry.file.lastModified] : [entry.name, entry.children.map(fingerprint)]
  return input.directory ? JSON.stringify(fingerprint(input.directory)) : [normalizeAttachmentPath(input.relativePath || input.file.name), input.file.size, input.file.lastModified].join(':')
}

export function revokeAttachmentItem(item: ComposerAttachmentItem) {
  if (item.previewUrl) {
    URL.revokeObjectURL(item.previewUrl)
  }
}

export function formatAttachmentSize(size: number): string {
  if (size < 1024) {
    return `${size} B`
  }

  if (size < 1024 * 1024) {
    return `${Math.round(size / 102.4) / 10} KB`
  }

  if (size < 1024 * 1024 * 1024) {
    return `${Math.round(size / (1024 * 102.4)) / 10} MB`
  }

  return `${Math.round(size / (1024 * 1024 * 102.4)) / 10} GB`
}

function createAttachmentId({
  file,
  relativePath,
}: ComposerAttachmentInput): string {
  return [
    relativePath || file.name,
    file.size,
    file.type,
    file.lastModified,
    Math.random().toString(36).slice(2, 8),
  ].join(':')
}

export function directoryAttachment(directory: AttachmentDirectory): ComposerAttachmentInput {
  return { file: new File([], directory.name, { type: 'application/x-directory', lastModified: 0 }), directory }
}

export function attachmentSize(input: ComposerAttachmentInput): number {
  const size = (entry: AttachmentDirectory | { kind: 'file'; file: File }): number => entry.kind === 'file' ? entry.file.size : entry.children.reduce((total, child) => total + size(child), 0)
  return input.directory ? size(input.directory) : input.file.size
}

async function collectEntry(entry: FileSystemEntryLike): Promise<ComposerAttachmentInput> {
  const path = (entry.fullPath ?? '').replace(/^\/+/, '')
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntryLike).file(resolve, error => reject(new Error(`${path}: ${error.message}`))))
    return { file }
  }
  if (!entry.isDirectory) throw new Error(`${path}: unsupported file system entry`)
  const reader = (entry as FileSystemDirectoryEntryLike).createReader()
  const children: AttachmentDirectory['children'] = []
  for (;;) {
    const entries = await new Promise<FileSystemEntryLike[]>((resolve, reject) => reader.readEntries(resolve, error => reject(new Error(`${path}: ${error.message}`))))
    if (entries.length === 0) break
    for (const child of entries) {
      const input = await collectEntry(child)
      children.push(input.directory ?? { kind: 'file', file: input.file })
    }
  }
  return directoryAttachment({ kind: 'dir', name: path.split('/').pop() || 'folder', children })
}

function normalizeAttachmentPath(path: string): string {
  return path.trim().replaceAll('\\', '/')
}

export function oversizedAttachmentNames(input: ComposerAttachmentInput): string[] {
  const visit = (entry: AttachmentDirectory | { kind: 'file'; file: File }, path: string): string[] => entry.kind === 'file'
    ? entry.file.size > MAX_ATTACHMENT_BYTES ? [`${path}${entry.file.name}`] : []
    : entry.children.flatMap(child => visit(child, `${path}${entry.name}/`))
  return input.directory ? visit(input.directory, '') : input.file.size > MAX_ATTACHMENT_BYTES ? [input.relativePath || input.file.name] : []
}
