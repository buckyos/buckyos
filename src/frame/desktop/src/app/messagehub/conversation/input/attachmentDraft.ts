export interface ComposerAttachmentInput {
  file: File
  relativePath?: string
}

export interface ComposerAttachmentItem extends ComposerAttachmentInput {
  id: string
  kind: 'image' | 'file'
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

  return Array.from(files).map((file) => ({
    file,
    relativePath: file.webkitRelativePath || undefined,
  }))
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
        return collectEntryFiles(entry)
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
    kind: input.file.type.startsWith('image/') ? 'image' : 'file',
    previewUrl,
    relativePath: input.relativePath,
  }
}

export function getAttachmentPathKey(
  input: ComposerAttachmentInput,
): string {
  return normalizeAttachmentPath(input.relativePath || input.file.name)
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

async function collectEntryFiles(
  entry: FileSystemEntryLike,
): Promise<ComposerAttachmentInput[]> {
  if (entry.isFile) {
    const file = await readFileEntry(entry as FileSystemFileEntryLike)

    if (!file) {
      return []
    }

    const fullPath = sanitizeEntryPath(entry.fullPath)

    return [{
      file,
      relativePath: fullPath.includes('/') ? fullPath : undefined,
    }]
  }

  if (!entry.isDirectory) {
    return []
  }

  const reader = (entry as FileSystemDirectoryEntryLike).createReader()
  const entries = await readDirectoryEntries(reader)
  const nestedFiles = await Promise.all(entries.map((child) => collectEntryFiles(child)))

  return nestedFiles.flat()
}

function readFileEntry(
  entry: FileSystemFileEntryLike,
): Promise<File | null> {
  return new Promise((resolve) => {
    entry.file(
      (file) => resolve(file),
      () => resolve(null),
    )
  })
}

async function readDirectoryEntries(
  reader: FileSystemDirectoryReaderLike,
): Promise<FileSystemEntryLike[]> {
  const entries: FileSystemEntryLike[] = []

  while (true) {
    const batch = await new Promise<FileSystemEntryLike[]>((resolve) => {
      reader.readEntries(
        (nextEntries) => resolve(nextEntries),
        () => resolve([]),
      )
    })

    if (batch.length === 0) {
      return entries
    }

    entries.push(...batch)
  }
}

function sanitizeEntryPath(path?: string): string {
  return (path ?? '').replace(/^\/+/, '')
}

function normalizeAttachmentPath(path: string): string {
  return path.trim().replaceAll('\\', '/')
}
