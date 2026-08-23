import { createHash } from 'node:crypto'
import { dirname, relative, resolve, sep } from 'node:path'
import { ToolError, UsageError } from '../core/errors.ts'

const BLOCK_SIZE = 512
const ZIP_LOCAL_SIGNATURE = 0x04034b50
const ZIP_CENTRAL_SIGNATURE = 0x02014b50
const ZIP_EOCD_SIGNATURE = 0x06054b50
const ZIP64_EOCD_SIGNATURE = 0x06064b50
const ZIP64_LOCATOR_SIGNATURE = 0x07064b50
const MAX_CENTRAL_DIRECTORY = 64 * 1024 * 1024
export const MAX_PIKG_ENTRIES = 4096

const CRC_TABLE = new Uint32Array(256)
for (let index = 0; index < CRC_TABLE.length; index++) {
  let value = index
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (value & 1 ? 0xedb88320 : 0)
  CRC_TABLE[index] = value >>> 0
}

export interface FileDigest {
  size: number
  sha256: string
  crc32: number
}

export interface ZipEntry {
  name: string
  compressedSize: number
  size: number
  crc32: number
  compression: number
  flags: number
  localOffset: number
  dataOffset: number
  externalAttributes: number
  isDirectory: boolean
}

export interface OpenZip {
  path: string
  size: number
  entries: ZipEntry[]
  byName: Map<string, ZipEntry>
}

interface TarSource {
  archivePath: string
  physicalPath: string
  kind: 'file' | 'directory'
  size: number
  executable: boolean
  identity: string
  contentDigest?: string
}

interface ZipWriteSource {
  name: string
  bytes?: Uint8Array
  path?: string
}

export async function digestFile(path: string): Promise<FileDigest> {
  const file = await Deno.open(path, { read: true })
  const sha = createHash('sha256')
  let crc = 0xffffffff
  let size = 0
  try {
    const buffer = new Uint8Array(256 * 1024)
    while (true) {
      const read = await file.read(buffer)
      if (read === null) break
      const chunk = buffer.subarray(0, read)
      sha.update(chunk)
      crc = updateCrc32(crc, chunk)
      size += read
      if (!Number.isSafeInteger(size)) throw new UsageError('FILE_TOO_LARGE', 'file is too large')
    }
  } finally {
    file.close()
  }
  return { size, sha256: sha.digest('hex'), crc32: (crc ^ 0xffffffff) >>> 0 }
}

export function sha256Bytes(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex')
}

export async function createDeterministicTarGz(
  sourceDir: string,
  destination: string,
): Promise<void> {
  const root = await Deno.realPath(sourceDir)
  const before = await collectTarSources(root)
  const tarPath = `${destination}.tar-${crypto.randomUUID()}`
  const tar = await Deno.open(tarPath, { createNew: true, write: true, mode: 0o600 })
  try {
    for (const source of before) {
      const header = tarHeader(source)
      await writeAll(tar, header)
      if (source.kind === 'file') {
        await copyFileToWriter(source.physicalPath, tar)
        const padding = (BLOCK_SIZE - source.size % BLOCK_SIZE) % BLOCK_SIZE
        if (padding) await writeAll(tar, new Uint8Array(padding))
      }
    }
    await writeAll(tar, new Uint8Array(BLOCK_SIZE * 2))
    await tar.sync()
  } finally {
    tar.close()
  }

  try {
    const after = await collectTarSources(root)
    if (JSON.stringify(before.map(sourceIdentity)) !== JSON.stringify(after.map(sourceIdentity))) {
      throw new ToolError('SOURCE_CHANGED', 'subpackage source changed while it was archived')
    }
    await gzipFile(tarPath, destination)
  } finally {
    await removeIfExists(tarPath)
  }
}

export async function gzipFile(source: string, destination: string): Promise<void> {
  const input = await Deno.open(source, { read: true })
  const output = await Deno.open(destination, { createNew: true, write: true, mode: 0o600 })
  try {
    await input.readable.pipeThrough(new CompressionStream('gzip')).pipeTo(output.writable)
  } finally {
    try {
      input.close()
    } catch {
      // Streams close their backing resources.
    }
    try {
      output.close()
    } catch {
      // Streams close their backing resources.
    }
  }
}

export async function writeStoredZip(
  destination: string,
  sources: ZipWriteSource[],
): Promise<void> {
  if (sources.length > MAX_PIKG_ENTRIES) {
    throw new UsageError('TOO_MANY_ENTRIES', 'PIKG contains too many entries')
  }
  const names = new Set<string>()
  const prepared = [] as Array<
    ZipWriteSource & { nameBytes: Uint8Array; digest: FileDigest; offset: number; zip64: boolean }
  >
  for (const source of sources) {
    validateZipEntryName(source.name)
    if (names.has(source.name)) throw invalidZip('structure', `duplicate entry: ${source.name}`)
    names.add(source.name)
    if ((source.bytes === undefined) === (source.path === undefined)) {
      throw new Error(`zip source ${source.name} must provide exactly one body`)
    }
    const digest = source.bytes
      ? {
        size: source.bytes.byteLength,
        sha256: sha256Bytes(source.bytes),
        crc32: crc32(source.bytes),
      }
      : await digestFile(source.path!)
    prepared.push({
      ...source,
      nameBytes: new TextEncoder().encode(source.name),
      digest,
      offset: 0,
      zip64: digest.size > 0xffffffff,
    })
  }

  await Deno.mkdir(dirname(destination), { recursive: true })
  const output = await Deno.open(destination, { createNew: true, write: true, mode: 0o600 })
  let offset = 0
  const centralRecords: Uint8Array[] = []
  try {
    for (const source of prepared) {
      source.offset = offset
      source.zip64 ||= offset > 0xffffffff
      const localExtra = source.zip64 ? zip64Extra([source.digest.size, source.digest.size]) : EMPTY
      const local = new Uint8Array(30 + source.nameBytes.length + localExtra.length)
      const view = new DataView(local.buffer)
      view.setUint32(0, ZIP_LOCAL_SIGNATURE, true)
      view.setUint16(4, source.zip64 ? 45 : 20, true)
      view.setUint16(6, 0x0800, true)
      view.setUint16(8, 0, true)
      view.setUint32(14, source.digest.crc32, true)
      view.setUint32(18, source.zip64 ? 0xffffffff : source.digest.size, true)
      view.setUint32(22, source.zip64 ? 0xffffffff : source.digest.size, true)
      view.setUint16(26, source.nameBytes.length, true)
      view.setUint16(28, localExtra.length, true)
      local.set(source.nameBytes, 30)
      local.set(localExtra, 30 + source.nameBytes.length)
      await writeAll(output, local)
      offset += local.length
      if (source.bytes) {
        await writeAll(output, source.bytes)
      } else {
        await copyFileToWriter(source.path!, output)
      }
      offset += source.digest.size

      const centralExtra = source.zip64
        ? zip64Extra([source.digest.size, source.digest.size, source.offset])
        : EMPTY
      const central = new Uint8Array(46 + source.nameBytes.length + centralExtra.length)
      const centralView = new DataView(central.buffer)
      centralView.setUint32(0, ZIP_CENTRAL_SIGNATURE, true)
      centralView.setUint16(4, (3 << 8) | 45, true)
      centralView.setUint16(6, source.zip64 ? 45 : 20, true)
      centralView.setUint16(8, 0x0800, true)
      centralView.setUint16(10, 0, true)
      centralView.setUint32(16, source.digest.crc32, true)
      centralView.setUint32(20, source.zip64 ? 0xffffffff : source.digest.size, true)
      centralView.setUint32(24, source.zip64 ? 0xffffffff : source.digest.size, true)
      centralView.setUint16(28, source.nameBytes.length, true)
      centralView.setUint16(30, centralExtra.length, true)
      centralView.setUint32(38, 0o100644 << 16, true)
      centralView.setUint32(42, source.zip64 ? 0xffffffff : source.offset, true)
      central.set(source.nameBytes, 46)
      central.set(centralExtra, 46 + source.nameBytes.length)
      centralRecords.push(central)
    }

    const centralOffset = offset
    for (const record of centralRecords) {
      await writeAll(output, record)
      offset += record.length
    }
    const centralSize = offset - centralOffset
    const needsZip64 = prepared.some((source) => source.zip64) || centralOffset > 0xffffffff ||
      centralSize > 0xffffffff
    if (needsZip64) {
      const zip64Offset = offset
      const eocd64 = new Uint8Array(56)
      const view64 = new DataView(eocd64.buffer)
      view64.setUint32(0, ZIP64_EOCD_SIGNATURE, true)
      setUint64(view64, 4, 44)
      view64.setUint16(12, (3 << 8) | 45, true)
      view64.setUint16(14, 45, true)
      setUint64(view64, 24, prepared.length)
      setUint64(view64, 32, prepared.length)
      setUint64(view64, 40, centralSize)
      setUint64(view64, 48, centralOffset)
      await writeAll(output, eocd64)
      offset += eocd64.length
      const locator = new Uint8Array(20)
      const locatorView = new DataView(locator.buffer)
      locatorView.setUint32(0, ZIP64_LOCATOR_SIGNATURE, true)
      setUint64(locatorView, 8, zip64Offset)
      locatorView.setUint32(16, 1, true)
      await writeAll(output, locator)
      offset += locator.length
    }
    const eocd = new Uint8Array(22)
    const view = new DataView(eocd.buffer)
    view.setUint32(0, ZIP_EOCD_SIGNATURE, true)
    view.setUint16(8, needsZip64 ? 0xffff : prepared.length, true)
    view.setUint16(10, needsZip64 ? 0xffff : prepared.length, true)
    view.setUint32(12, needsZip64 ? 0xffffffff : centralSize, true)
    view.setUint32(16, needsZip64 ? 0xffffffff : centralOffset, true)
    await writeAll(output, eocd)
    await output.sync()
  } finally {
    output.close()
  }
}

export async function openZip(path: string): Promise<OpenZip> {
  const stat = await Deno.stat(path)
  if (!stat.isFile || stat.size < 22) throw invalidZip('container', 'PIKG is not a ZIP file')
  const file = await Deno.open(path, { read: true })
  try {
    const magic = await readAt(file, 0, 4)
    if (
      new DataView(magic.buffer, magic.byteOffset, magic.byteLength).getUint32(0, true) !==
        ZIP_LOCAL_SIGNATURE
    ) {
      throw invalidZip('container', 'PIKG magic mismatch')
    }
    const tailLength = Math.min(stat.size, 22 + 65535)
    const tail = await readAt(file, stat.size - tailLength, tailLength)
    const eocdPosition = findLastSignature(tail, ZIP_EOCD_SIGNATURE)
    if (eocdPosition < 0 || eocdPosition + 22 > tail.length) {
      throw invalidZip('container', 'ZIP end-of-central-directory record is missing')
    }
    const eocd = new DataView(
      tail.buffer,
      tail.byteOffset + eocdPosition,
      tail.length - eocdPosition,
    )
    const commentLength = eocd.getUint16(20, true)
    if (eocdPosition + 22 + commentLength !== tail.length) {
      throw invalidZip('container', 'ZIP has trailing data or a truncated comment')
    }
    if (eocd.getUint16(4, true) !== 0 || eocd.getUint16(6, true) !== 0) {
      throw invalidZip('container', 'multi-disk ZIP files are not supported')
    }
    let count = eocd.getUint16(10, true)
    if (eocd.getUint16(8, true) !== count) {
      throw invalidZip('container', 'ZIP entry counts disagree')
    }
    let centralSize = eocd.getUint32(12, true)
    let centralOffset = eocd.getUint32(16, true)
    if (count === 0xffff || centralSize === 0xffffffff || centralOffset === 0xffffffff) {
      const eocdAbsolute = stat.size - tailLength + eocdPosition
      if (eocdAbsolute < 20) throw invalidZip('container', 'ZIP64 locator is missing')
      const locatorBytes = await readAt(file, eocdAbsolute - 20, 20)
      const locator = new DataView(locatorBytes.buffer, locatorBytes.byteOffset, 20)
      if (locator.getUint32(0, true) !== ZIP64_LOCATOR_SIGNATURE) {
        throw invalidZip('container', 'ZIP64 locator signature mismatch')
      }
      if (locator.getUint32(4, true) !== 0 || locator.getUint32(16, true) !== 1) {
        throw invalidZip('container', 'multi-disk ZIP64 files are not supported')
      }
      const recordOffset = uint64(locator, 8)
      const recordBytes = await readAt(file, recordOffset, 56)
      const record = new DataView(recordBytes.buffer, recordBytes.byteOffset, 56)
      if (record.getUint32(0, true) !== ZIP64_EOCD_SIGNATURE) {
        throw invalidZip('container', 'ZIP64 end record signature mismatch')
      }
      if (record.getUint32(16, true) !== 0 || record.getUint32(20, true) !== 0) {
        throw invalidZip('container', 'multi-disk ZIP64 files are not supported')
      }
      if (uint64(record, 24) !== uint64(record, 32)) {
        throw invalidZip('container', 'ZIP64 entry counts disagree')
      }
      count = uint64(record, 32)
      centralSize = uint64(record, 40)
      centralOffset = uint64(record, 48)
    }
    if (count > MAX_PIKG_ENTRIES) throw invalidZip('structure', 'PIKG has too many entries')
    if (centralSize > MAX_CENTRAL_DIRECTORY) {
      throw invalidZip('structure', 'PIKG central directory exceeds the size limit')
    }
    if (centralOffset + centralSize > stat.size) {
      throw invalidZip('container', 'ZIP central directory is out of bounds')
    }
    const central = await readAt(file, centralOffset, centralSize)
    const decoder = new TextDecoder('utf-8', { fatal: true })
    const entries: ZipEntry[] = []
    const byName = new Map<string, ZipEntry>()
    const fileNames = new Set<string>()
    const directories = new Set<string>()
    let position = 0
    for (let index = 0; index < count; index++) {
      if (position + 46 > central.length || u32(central, position) !== ZIP_CENTRAL_SIGNATURE) {
        throw invalidZip('structure', `central directory entry #${index} is truncated`)
      }
      const flags = u16(central, position + 8)
      const compression = u16(central, position + 10)
      const crc = u32(central, position + 16)
      let compressedSize = u32(central, position + 20)
      let size = u32(central, position + 24)
      const nameLength = u16(central, position + 28)
      const extraLength = u16(central, position + 30)
      const commentLength = u16(central, position + 32)
      const externalAttributes = u32(central, position + 38)
      let localOffset = u32(central, position + 42)
      const end = position + 46 + nameLength + extraLength + commentLength
      if (end > central.length) throw invalidZip('structure', 'central directory is truncated')
      let name: string
      try {
        name = decoder.decode(central.subarray(position + 46, position + 46 + nameLength))
      } catch {
        throw invalidZip('structure', `entry #${index} name is not UTF-8`)
      }
      validateZipEntryName(name)
      if (byName.has(name)) throw invalidZip('structure', `duplicate entry: ${name}`)
      if (flags & 0x0001) throw invalidZip('structure', `encrypted entry is not allowed: ${name}`)
      if (![0, 8].includes(compression)) {
        throw invalidZip('structure', `unsupported compression method for ${name}`)
      }
      const extra = central.subarray(
        position + 46 + nameLength,
        position + 46 + nameLength + extraLength,
      )
      if (size === 0xffffffff || compressedSize === 0xffffffff || localOffset === 0xffffffff) {
        const values = parseZip64Extra(extra)
        let valueIndex = 0
        if (size === 0xffffffff) size = requiredZip64(values, valueIndex++, name)
        if (compressedSize === 0xffffffff) {
          compressedSize = requiredZip64(values, valueIndex++, name)
        }
        if (localOffset === 0xffffffff) localOffset = requiredZip64(values, valueIndex++, name)
      }
      const mode = (externalAttributes >>> 16) & 0xffff
      if ((mode & 0xf000) === 0xa000) {
        throw invalidZip('structure', `symlink entry is not allowed: ${name}`)
      }
      const isDirectory = name.endsWith('/')
      const normalized = name.replace(/\/$/, '')
      if (isDirectory) directories.add(normalized)
      else fileNames.add(name)
      const entry: ZipEntry = {
        name,
        compressedSize,
        size,
        crc32: crc,
        compression,
        flags,
        localOffset,
        dataOffset: 0,
        externalAttributes,
        isDirectory,
      }
      entries.push(entry)
      byName.set(name, entry)
      position = end
    }
    if (position !== central.length) {
      throw invalidZip('structure', 'central directory size mismatch')
    }
    for (const fileName of fileNames) {
      const parts = fileName.split('/')
      let prefix = ''
      for (const part of parts.slice(0, -1)) {
        prefix = prefix ? `${prefix}/${part}` : part
        directories.add(prefix)
      }
    }
    const conflict = [...fileNames].find((name) => directories.has(name))
    if (conflict) throw invalidZip('structure', `path is both file and directory: ${conflict}`)

    const intervals: Array<[number, number, string]> = []
    for (const entry of entries) {
      const local = await readAt(file, entry.localOffset, 30)
      if (u32(local, 0) !== ZIP_LOCAL_SIGNATURE) {
        throw invalidZip('structure', `local header is missing for ${entry.name}`)
      }
      const localFlags = u16(local, 6)
      const localCompression = u16(local, 8)
      const localNameLength = u16(local, 26)
      const localExtraLength = u16(local, 28)
      if (localFlags !== entry.flags || localCompression !== entry.compression) {
        throw invalidZip('structure', `local header disagrees for ${entry.name}`)
      }
      const localName = await readAt(file, entry.localOffset + 30, localNameLength)
      let decodedLocalName: string
      try {
        decodedLocalName = decoder.decode(localName)
      } catch {
        throw invalidZip('structure', `local entry name is not UTF-8: ${entry.name}`)
      }
      if (decodedLocalName !== entry.name) {
        throw invalidZip('structure', `local entry name disagrees for ${entry.name}`)
      }
      entry.dataOffset = entry.localOffset + 30 + localNameLength + localExtraLength
      const end = entry.dataOffset + entry.compressedSize
      if (end > centralOffset) {
        throw invalidZip('structure', `entry data is out of bounds: ${entry.name}`)
      }
      intervals.push([entry.localOffset, end, entry.name])
    }
    intervals.sort((left, right) => left[0] - right[0])
    for (let index = 1; index < intervals.length; index++) {
      if (intervals[index][0] < intervals[index - 1][1]) {
        throw invalidZip('structure', `overlapping ZIP entries: ${intervals[index][2]}`)
      }
    }
    return { path: resolve(path), size: stat.size, entries, byName }
  } finally {
    file.close()
  }
}

export async function readZipEntry(
  archive: OpenZip,
  entry: ZipEntry,
  limit: number,
): Promise<Uint8Array> {
  if (entry.size > limit) throw invalidZip('limits', `entry exceeds size limit: ${entry.name}`)
  const chunks: Uint8Array[] = []
  let size = 0
  const verification = createEntryVerification(entry)
  try {
    for await (const chunk of zipEntryStream(archive.path, entry)) {
      size += chunk.byteLength
      if (size > limit || size > entry.size) {
        throw invalidZip('limits', `entry expands beyond its declared size: ${entry.name}`)
      }
      verification.update(chunk)
      chunks.push(chunk.slice())
    }
  } catch (error) {
    if (error instanceof ToolError) throw error
    throw invalidZip('compression', `failed to read entry ${entry.name}`)
  }
  verification.finish(size)
  const result = new Uint8Array(size)
  let offset = 0
  for (const chunk of chunks) {
    result.set(chunk, offset)
    offset += chunk.length
  }
  return result
}

export async function verifyZipEntry(
  archive: OpenZip,
  entry: ZipEntry,
  expectedSha256?: string,
): Promise<FileDigest> {
  const verification = createEntryVerification(entry)
  try {
    for await (const chunk of zipEntryStream(archive.path, entry)) verification.update(chunk)
  } catch (error) {
    if (error instanceof ToolError) throw error
    throw invalidZip('compression', `failed to read entry ${entry.name}`)
  }
  const result = verification.finish()
  if (expectedSha256 && result.sha256 !== expectedSha256.toLowerCase()) {
    throw invalidZip('content', `content digest mismatch: ${entry.name}`, entry.name)
  }
  return result
}

function createEntryVerification(entry: ZipEntry): {
  update(chunk: Uint8Array): void
  finish(actualSize?: number): FileDigest
} {
  const sha = createHash('sha256')
  let crc = 0xffffffff
  let size = 0
  return {
    update(chunk) {
      size += chunk.length
      if (size > entry.size) {
        throw invalidZip('limits', `entry is larger than declared: ${entry.name}`)
      }
      sha.update(chunk)
      crc = updateCrc32(crc, chunk)
    },
    finish(actualSize = size) {
      if (actualSize !== entry.size || size !== entry.size) {
        throw invalidZip('content', `entry size mismatch: ${entry.name}`, entry.name)
      }
      const actualCrc = (crc ^ 0xffffffff) >>> 0
      if (actualCrc !== entry.crc32) {
        throw invalidZip('content', `entry CRC mismatch: ${entry.name}`, entry.name)
      }
      return { size, sha256: sha.digest('hex'), crc32: actualCrc }
    },
  }
}

function zipEntryStream(path: string, entry: ZipEntry): ReadableStream<Uint8Array> {
  let file: Deno.FsFile | undefined
  let remaining = entry.compressedSize
  const compressed = new ReadableStream<Uint8Array>({
    async start() {
      file = await Deno.open(path, { read: true })
      await file.seek(entry.dataOffset, Deno.SeekMode.Start)
    },
    async pull(controller) {
      if (!file || remaining === 0) {
        file?.close()
        controller.close()
        return
      }
      const buffer = new Uint8Array(Math.min(256 * 1024, remaining))
      const read = await file.read(buffer)
      if (read === null) {
        file.close()
        controller.error(invalidZip('container', `truncated entry: ${entry.name}`))
        return
      }
      remaining -= read
      controller.enqueue(buffer.subarray(0, read))
    },
    cancel() {
      file?.close()
    },
  })
  if (entry.compression === 0) return compressed
  return compressed.pipeThrough(
    new DecompressionStream('deflate-raw') as unknown as ReadableWritablePair<
      Uint8Array,
      Uint8Array
    >,
  )
}

async function collectTarSources(root: string): Promise<TarSource[]> {
  const output: TarSource[] = []
  const activeDirectories = new Set<string>()
  async function walk(physicalDirectory: string, archivePrefix: string): Promise<void> {
    const realDirectory = await Deno.realPath(physicalDirectory)
    assertWithin(root, realDirectory, archivePrefix || '.')
    if (activeDirectories.has(realDirectory)) {
      throw new UsageError('INVALID_SOURCE', `symlink cycle in source: ${archivePrefix}`)
    }
    activeDirectories.add(realDirectory)
    try {
      const children = [] as Deno.DirEntry[]
      for await (const child of Deno.readDir(realDirectory)) children.push(child)
      children.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0)
      for (const child of children) {
        if (child.name.includes('\0') || child.name.includes('/') || child.name.includes('\\')) {
          throw new UsageError('INVALID_SOURCE', 'source contains an unsafe file name')
        }
        const archivePath = archivePrefix ? `${archivePrefix}/${child.name}` : child.name
        const unresolved = resolve(realDirectory, child.name)
        const info = await Deno.lstat(unresolved)
        const physical = info.isSymlink ? await Deno.realPath(unresolved) : unresolved
        assertWithin(root, physical, archivePath)
        const target = info.isSymlink ? await Deno.stat(physical) : info
        if (target.isDirectory) {
          output.push({
            archivePath: `${archivePath}/`,
            physicalPath: physical,
            kind: 'directory',
            size: 0,
            executable: true,
            identity: fileIdentity(info, physical, target),
          })
          await walk(physical, archivePath)
        } else if (target.isFile) {
          output.push({
            archivePath,
            physicalPath: physical,
            kind: 'file',
            size: target.size,
            executable: ((target.mode ?? 0) & 0o111) !== 0,
            identity: fileIdentity(info, physical, target),
            contentDigest: (await digestFile(physical)).sha256,
          })
        } else {
          throw new UsageError('INVALID_SOURCE', `unsupported special file: ${archivePath}`)
        }
      }
    } finally {
      activeDirectories.delete(realDirectory)
    }
  }
  await walk(root, '')
  return output
}

function fileIdentity(link: Deno.FileInfo, realPath: string, target: Deno.FileInfo): string {
  return JSON.stringify({
    realPath,
    linkMtime: link.mtime?.getTime() ?? null,
    mtime: target.mtime?.getTime() ?? null,
    size: target.size,
    dev: target.dev,
    ino: target.ino,
    mode: target.mode,
  })
}

function sourceIdentity(source: TarSource): unknown {
  return [
    source.archivePath,
    source.kind,
    source.size,
    source.executable,
    source.identity,
    source.contentDigest ?? null,
  ]
}

function assertWithin(root: string, candidate: string, label: string): void {
  const path = relative(root, candidate)
  if (
    path === '..' || path.startsWith(`..${sep}`) ||
    resolve(candidate) === resolve(root) && label !== '.'
  ) {
    throw new UsageError('INVALID_SOURCE', `symlink escapes source root: ${label}`)
  }
}

function tarHeader(source: TarSource): Uint8Array {
  const header = new Uint8Array(BLOCK_SIZE)
  const { name, prefix } = splitUstarPath(source.archivePath)
  writeString(header, 0, 100, name)
  writeOctal(
    header,
    100,
    8,
    source.kind === 'directory' ? 0o755 : source.executable ? 0o755 : 0o644,
  )
  writeOctal(header, 108, 8, 0)
  writeOctal(header, 116, 8, 0)
  writeOctal(header, 124, 12, source.size)
  writeOctal(header, 136, 12, 0)
  header.fill(0x20, 148, 156)
  header[156] = source.kind === 'directory' ? 0x35 : 0x30
  writeString(header, 257, 6, 'ustar\0')
  writeString(header, 263, 2, '00')
  writeString(header, 345, 155, prefix)
  let checksum = 0
  for (const byte of header) checksum += byte
  writeChecksum(header, checksum)
  return header
}

function splitUstarPath(path: string): { name: string; prefix: string } {
  const encoder = new TextEncoder()
  if (encoder.encode(path).length <= 100) return { name: path, prefix: '' }
  const directory = path.endsWith('/')
  const core = directory ? path.slice(0, -1) : path
  const parts = core.split('/')
  for (let index = parts.length - 1; index > 0; index--) {
    const prefix = parts.slice(0, index).join('/')
    const name = `${parts.slice(index).join('/')}${directory ? '/' : ''}`
    if (encoder.encode(prefix).length <= 155 && encoder.encode(name).length <= 100) {
      return { name, prefix }
    }
  }
  throw new UsageError('INVALID_SOURCE', `source path is too long for a portable tar: ${path}`)
}

function writeString(target: Uint8Array, offset: number, length: number, value: string): void {
  const bytes = new TextEncoder().encode(value)
  if (bytes.length > length) throw new UsageError('INVALID_SOURCE', 'tar field is too long')
  target.set(bytes, offset)
}

function writeOctal(target: Uint8Array, offset: number, length: number, value: number): void {
  const text = value.toString(8).padStart(length - 1, '0')
  if (text.length >= length) throw new UsageError('FILE_TOO_LARGE', 'tar field overflows')
  writeString(target, offset, length, `${text}\0`)
}

function writeChecksum(target: Uint8Array, value: number): void {
  const text = value.toString(8).padStart(6, '0')
  writeString(target, 148, 8, `${text}\0 `)
}

function validateZipEntryName(name: string): void {
  if (!name || name.includes('\0') || name.includes('\\') || name.startsWith('/')) {
    throw invalidZip('structure', `unsafe entry name: ${JSON.stringify(name)}`)
  }
  if (/^[A-Za-z]:/.test(name)) throw invalidZip('structure', `absolute entry name: ${name}`)
  const directory = name.endsWith('/')
  const segments = name.split('/')
  for (let index = 0; index < segments.length; index++) {
    const segment = segments[index]
    if (
      segment === '.' || segment === '..' ||
      (!segment && !(directory && index === segments.length - 1))
    ) {
      throw invalidZip('structure', `unsafe entry name: ${name}`)
    }
  }
}

function invalidZip(stage: string, message: string, entry?: string): ToolError {
  return new ToolError('INVALID_PACKAGE', message, 6, false, {
    stage,
    ...(entry ? { entry } : {}),
  })
}

function crc32(bytes: Uint8Array): number {
  return (updateCrc32(0xffffffff, bytes) ^ 0xffffffff) >>> 0
}

function updateCrc32(crc: number, bytes: Uint8Array): number {
  let value = crc
  for (const byte of bytes) value = CRC_TABLE[(value ^ byte) & 0xff] ^ (value >>> 8)
  return value >>> 0
}

async function writeAll(file: Deno.FsFile, bytes: Uint8Array): Promise<void> {
  let offset = 0
  while (offset < bytes.length) offset += await file.write(bytes.subarray(offset))
}

async function copyFileToWriter(path: string, output: Deno.FsFile): Promise<void> {
  const input = await Deno.open(path, { read: true })
  try {
    const buffer = new Uint8Array(256 * 1024)
    while (true) {
      const read = await input.read(buffer)
      if (read === null) break
      await writeAll(output, buffer.subarray(0, read))
    }
  } finally {
    input.close()
  }
}

async function readAt(file: Deno.FsFile, offset: number, length: number): Promise<Uint8Array> {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0) {
    throw invalidZip('container', 'ZIP offset is invalid')
  }
  await file.seek(offset, Deno.SeekMode.Start)
  const output = new Uint8Array(length)
  let position = 0
  while (position < length) {
    const read = await file.read(output.subarray(position))
    if (read === null) throw invalidZip('container', 'ZIP file is truncated')
    position += read
  }
  return output
}

function findLastSignature(bytes: Uint8Array, signature: number): number {
  for (let index = bytes.length - 4; index >= 0; index--) {
    if (u32(bytes, index) === signature) return index
  }
  return -1
}

function parseZip64Extra(extra: Uint8Array): number[] {
  let position = 0
  while (position + 4 <= extra.length) {
    const id = u16(extra, position)
    const length = u16(extra, position + 2)
    const end = position + 4 + length
    if (end > extra.length) throw invalidZip('structure', 'ZIP extra field is truncated')
    if (id === 0x0001) {
      if (length % 8 !== 0) throw invalidZip('structure', 'ZIP64 extra field is malformed')
      const view = new DataView(extra.buffer, extra.byteOffset + position + 4, length)
      const values: number[] = []
      for (let offset = 0; offset < length; offset += 8) values.push(uint64(view, offset))
      return values
    }
    position = end
  }
  throw invalidZip('structure', 'ZIP64 extra field is missing')
}

function requiredZip64(values: number[], index: number, name: string): number {
  const value = values[index]
  if (value === undefined) throw invalidZip('structure', `ZIP64 values are missing for ${name}`)
  return value
}

function zip64Extra(values: number[]): Uint8Array {
  const output = new Uint8Array(4 + values.length * 8)
  const view = new DataView(output.buffer)
  view.setUint16(0, 0x0001, true)
  view.setUint16(2, values.length * 8, true)
  values.forEach((value, index) => setUint64(view, 4 + index * 8, value))
  return output
}

function setUint64(view: DataView, offset: number, value: number): void {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error('uint64 value is unsafe')
  view.setBigUint64(offset, BigInt(value), true)
}

function uint64(view: DataView, offset: number): number {
  const value = view.getBigUint64(offset, true)
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw invalidZip('limits', 'ZIP64 value is too large')
  }
  return Number(value)
}

function u16(bytes: Uint8Array, offset: number): number {
  return new DataView(bytes.buffer, bytes.byteOffset + offset, 2).getUint16(0, true)
}

function u32(bytes: Uint8Array, offset: number): number {
  return new DataView(bytes.buffer, bytes.byteOffset + offset, 4).getUint32(0, true)
}

async function removeIfExists(path: string): Promise<void> {
  try {
    await Deno.remove(path)
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
}

const EMPTY = new Uint8Array()
