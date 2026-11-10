import type { SkillPackage } from './model'

// Reads a skill PIKG in the browser: a zip with `APPDOC.json`,
// `PACKAGE_META.json` and the `agent_skills` sub package as a tar.gz. The
// same checks as `libopendan::skills::read_pikg` and `buckyos skill install`
// (websdk `cli/modules/skill.ts`); the SDK's `pikg_archive.ts` reads files
// through the CLI host and is not in its browser build.

const SKILLS_SUB_PKG = 'agent_skills'
const SKILL_DIRS = ['cmd_skills', 'plan_skills', 'goal_skills', 'tools']
const MAX_PIKG_BYTES = 128 * 1024 * 1024
const MAX_PAYLOAD_BYTES = 64 * 1024 * 1024
const MAX_ENTRIES = 4096
const BLOCK = 512
const ZIP_LOCAL = 0x04034b50
const ZIP_CENTRAL = 0x02014b50
const ZIP_EOCD = 0x06054b50

type Bytes = Uint8Array<ArrayBuffer>

const bad = (message: string) => new Error(`not a skill package: ${message}`)

const hex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')

const sha256 = async (bytes: Bytes): Promise<string> => {
  if (!globalThis.crypto?.subtle) throw new Error('this page cannot compute digests (it is not served over https)')
  return `sha256:${hex(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)))}`
}

const base64 = (bytes: Uint8Array): string => {
  let text = ''
  for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000))
  return btoa(text)
}

const inflate = async (bytes: Bytes, format: 'gzip' | 'deflate-raw', limit: number): Promise<Bytes> => {
  const reader = new Blob([bytes]).stream().pipeThrough(new DecompressionStream(format)).getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.length
    if (size > limit) {
      await reader.cancel()
      throw bad(`more than ${limit} bytes unpacked`)
    }
    chunks.push(value)
  }
  const out = new Uint8Array(size)
  let at = 0
  for (const chunk of chunks) {
    out.set(chunk, at)
    at += chunk.length
  }
  return out
}

interface ZipEntry {
  method: number
  compressedSize: number
  size: number
  localOffset: number
}

const readZip = (raw: Bytes): Map<string, ZipEntry> => {
  const view = new DataView(raw.buffer, raw.byteOffset, raw.byteLength)
  // The end record is the last thing in the file, before a comment of at most 64 KiB.
  let end = -1
  for (let at = raw.length - 22; at >= Math.max(0, raw.length - 22 - 0xffff); at--) {
    if (view.getUint32(at, true) === ZIP_EOCD) {
      end = at
      break
    }
  }
  if (end < 0) throw bad('no zip directory')
  const count = view.getUint16(end + 10, true)
  const directorySize = view.getUint32(end + 12, true)
  let at = view.getUint32(end + 16, true)
  if (count === 0xffff || at === 0xffffffff || directorySize === 0xffffffff) throw bad('zip64 archives are not read here')
  if (count > MAX_ENTRIES) throw bad(`${count} zip entries`)
  if (at + directorySize > end) throw bad('zip directory is out of range')
  const decoder = new TextDecoder()
  const entries = new Map<string, ZipEntry>()
  for (let index = 0; index < count; index++) {
    if (at + 46 > end || view.getUint32(at, true) !== ZIP_CENTRAL) throw bad('zip directory is damaged')
    const nameLength = view.getUint16(at + 28, true)
    const name = decoder.decode(raw.subarray(at + 46, at + 46 + nameLength))
    if (entries.has(name)) throw bad(`duplicate zip entry ${name}`)
    entries.set(name, {
      method: view.getUint16(at + 10, true),
      compressedSize: view.getUint32(at + 20, true),
      size: view.getUint32(at + 24, true),
      localOffset: view.getUint32(at + 42, true),
    })
    at += 46 + nameLength + view.getUint16(at + 30, true) + view.getUint16(at + 32, true)
  }
  return entries
}

const readZipEntry = async (raw: Bytes, entries: Map<string, ZipEntry>, name: string, limit: number): Promise<Bytes> => {
  const entry = entries.get(name)
  if (!entry) throw bad(`${name} is missing`)
  if (entry.size > limit || entry.compressedSize > limit) throw bad(`${name} is larger than ${limit} bytes`)
  const view = new DataView(raw.buffer, raw.byteOffset, raw.byteLength)
  const header = entry.localOffset
  if (header + 30 > raw.length || view.getUint32(header, true) !== ZIP_LOCAL) throw bad(`${name}: zip entry is damaged`)
  const start = header + 30 + view.getUint16(header + 26, true) + view.getUint16(header + 28, true)
  if (start + entry.compressedSize > raw.length) throw bad(`${name}: zip entry is truncated`)
  const data = raw.slice(start, start + entry.compressedSize)
  if (entry.method === 0) return data
  if (entry.method !== 8) throw bad(`${name}: zip compression ${entry.method} is not read here`)
  const bytes = await inflate(data, 'deflate-raw', limit)
  if (bytes.length !== entry.size) throw bad(`${name}: zip entry is truncated`)
  return bytes
}

interface TarFile {
  path: string
  executable: boolean
  bytes: Bytes
}

const readTarGz = async (payload: Bytes, limit: number): Promise<TarFile[]> => {
  const tar = await inflate(payload, 'gzip', limit)
  const decoder = new TextDecoder('utf-8', { fatal: true })
  const files: TarFile[] = []
  const seen = new Set<string>()
  let offset = 0
  while (offset + BLOCK <= tar.length && tar.subarray(offset, offset + BLOCK).some(Boolean)) {
    const block = tar.subarray(offset, offset + BLOCK)
    const text = (at: number, length: number) => decoder.decode(block.subarray(at, at + length)).replace(/\0.*$/s, '')
    let raw: string
    let size: number
    let mode: number
    try {
      const prefix = text(257, 6) === 'ustar' ? text(345, 155) : ''
      raw = prefix ? `${prefix}/${text(0, 100)}` : text(0, 100)
      size = Number.parseInt(text(124, 12).trim() || '0', 8)
      mode = Number.parseInt(text(100, 8).trim() || '0', 8)
    } catch {
      throw bad('a tar header is not UTF-8')
    }
    const start = offset + BLOCK
    if (!Number.isSafeInteger(size) || size < 0 || start + size > tar.length) throw bad(`tar entry is truncated: ${raw}`)
    offset = start + Math.ceil(size / BLOCK) * BLOCK
    const type = String.fromCharCode(block[156] || 0x30)
    const path = raw.replace(/^(?:\.\/)+/, '').replace(/\/$/, '')
    if (type === '5') continue
    if (type !== '0') throw bad(`\`${path}\` is not a plain file`)
    if (!path || path.startsWith('/') || /^[A-Za-z]:/.test(path) || path.includes('\\') || path.split('/').some((part) => !part || part === '.' || part === '..')) {
      throw bad(`\`${raw}\` is not a plain relative file`)
    }
    if (seen.has(path)) throw bad(`duplicate tar entry ${path}`)
    seen.add(path)
    files.push({ path, executable: (mode & 0o111) !== 0, bytes: tar.slice(start, start + size) })
  }
  return files
}

type Json = Record<string, unknown>
const object = (value: unknown): Json => (value && typeof value === 'object' ? (value as Json) : {})

export interface ReadPikg {
  package: SkillPackage
  /** Files of the sub package outside the skill directories: left out. */
  ignored: string[]
}

export const readSkillPikg = async (file: Blob): Promise<ReadPikg> => {
  if (file.size > MAX_PIKG_BYTES) throw bad(`the file is larger than ${MAX_PIKG_BYTES} bytes`)
  const raw: Bytes = new Uint8Array(await file.arrayBuffer())
  const entries = readZip(raw)
  const json = async (name: string): Promise<Json> => {
    const bytes = await readZipEntry(raw, entries, name, 16 * 1024 * 1024)
    try {
      return object(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)))
    } catch {
      throw bad(`${name} is not JSON`)
    }
  }
  const app = await json('APPDOC.json')
  const meta = await json('PACKAGE_META.json')
  if (app.app_type !== SKILLS_SUB_PKG) {
    throw bad(`app_type is \`${typeof app.app_type === 'string' ? app.app_type : ''}\`, a skill package has \`${SKILLS_SUB_PKG}\``)
  }
  const objid = object(object(app.pkg_list)[SKILLS_SUB_PKG]).pkg_objid
  if (typeof objid !== 'string') throw bad(`APPDOC.json has no pkg_list.${SKILLS_SUB_PKG}.pkg_objid`)
  const size = object(object(meta.package_objects)[objid]).size
  if (typeof size !== 'number') throw bad(`PACKAGE_META.json has no package object ${objid}`)
  const content = Object.values(object(meta.content_index)).map(object).find((entry) => entry.sub_pkg_name === SKILLS_SUB_PKG)
  if (!content || typeof content.path !== 'string' || typeof content.digest !== 'string') {
    throw bad(`PACKAGE_META.json has no content of \`${SKILLS_SUB_PKG}\``)
  }
  const payload = await readZipEntry(raw, entries, content.path, MAX_PAYLOAD_BYTES)
  const actual = await sha256(payload)
  if (actual !== content.digest || payload.length !== size) {
    throw bad(`${content.path} does not match PACKAGE_META.json (${actual}, ${payload.length} bytes; expected ${content.digest}, ${size} bytes)`)
  }
  const files: SkillPackage['files'] = {}
  const ignored: string[] = []
  for (const entry of await readTarGz(payload, MAX_PAYLOAD_BYTES)) {
    if (!SKILL_DIRS.includes(entry.path.split('/')[0])) {
      ignored.push(entry.path)
      continue
    }
    files[entry.path] = { content: base64(entry.bytes), executable: entry.executable }
  }
  if (Object.keys(files).length === 0) throw bad(`no file under ${SKILL_DIRS.join('/, ')}/`)
  return {
    package: {
      package: typeof app.name === 'string' ? app.name : '',
      version: typeof app.version === 'string' ? app.version : '',
      pikg_digest: await sha256(raw),
      files,
    },
    ignored,
  }
}
