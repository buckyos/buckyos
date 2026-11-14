/* A directory's members as a file list (框架 PRD §7.4): a `buckyos.dir` maps member names to files, to folders and to
 * linked directories (other `buckyos.dir` entities, "add an existing directory"). A name may hold `/` (`docs/a.txt`),
 * so the directory's sub-folders are part of its own members (Q25): the prefixes of its names, and `folder` members
 * for folders that may be empty. A place in the list is a directory entity and a prefix inside it; the breadcrumb is
 * the way down from the editor's root. Nothing here touches React or the store. */

import type { KeyedContent } from '../../api/types'

export type FileMember = { kind: 'file'; object_id: string; media_type?: string; size?: number }
/** A sub-folder of this directory that exists without names under it (an empty one). */
export type FolderMember = { kind: 'folder' }
export type DirRefMember = { kind: 'dir'; entity_id: string }
export type DirectoryMember = FileMember | FolderMember | DirRefMember
export type DirectoryPayload = { members: Record<string, DirectoryMember> }
export type DirectoryContent = KeyedContent<DirectoryPayload>

/** Where the list is: a directory entity and a name prefix inside it (`''`, or `a/b/`). */
export interface Place { entityId: string; prefix: string }

/** One step of the breadcrumb below the root: into a linked directory, or into a folder of this one. */
export type Crumb = { kind: 'dir'; name: string; entityId: string } | { kind: 'folder'; name: string }

/** One row of the list. `key` is the member name (files and linked directories); a folder is its `folder` member and
 * every name under `prefix`; `count` is what it shows when opened, `size` the bytes of the files anywhere under it. */
export type Entry =
  | { id: string; kind: 'file'; name: string; key: string; member: FileMember }
  | { id: string; kind: 'dir'; name: string; key: string; member: DirRefMember }
  | { id: string; kind: 'folder'; name: string; prefix: string; count: number; size: number }

export const isFolderLike = (entry: Entry) => entry.kind !== 'file'

/** The place a breadcrumb leads to. */
export function placeOf(rootId: string, crumbs: readonly Crumb[]): Place {
  let place: Place = { entityId: rootId, prefix: '' }
  for (const crumb of crumbs) place = crumb.kind === 'dir' ? { entityId: crumb.entityId, prefix: '' } : { entityId: place.entityId, prefix: `${place.prefix}${crumb.name}/` }
  return place
}

/** The place of a row that can be entered. */
export function placeInside(place: Place, entry: Entry): Place | null {
  if (entry.kind === 'dir') return { entityId: entry.member.entity_id, prefix: '' }
  if (entry.kind === 'folder') return { entityId: place.entityId, prefix: entry.prefix }
  return null
}

export const samePlace = (a: Place, b: Place) => a.entityId === b.entityId && a.prefix === b.prefix

/** The rows at `prefix`: folders and linked directories first, then files, each by name. */
export function entriesAt(members: Record<string, DirectoryMember>, prefix: string, compare: (a: string, b: string) => number): Entry[] {
  const folders = new Map<string, { inside: Set<string>; size: number }>()
  const out: Entry[] = []
  for (const [key, member] of Object.entries(members)) {
    if (!key.startsWith(prefix)) continue
    const rest = key.slice(prefix.length)
    const slash = rest.indexOf('/')
    if (slash >= 0 || member.kind === 'folder') {
      const name = slash >= 0 ? rest.slice(0, slash) : rest
      const folder = folders.get(name) ?? { inside: new Set<string>(), size: 0 }
      if (slash >= 0) {
        const inner = rest.slice(slash + 1)
        folder.inside.add(inner.split('/', 1)[0])
        folder.size += member.kind === 'file' ? member.size ?? 0 : 0
      }
      folders.set(name, folder)
      continue
    }
    out.push(member.kind === 'file' ? { id: `m:${key}`, kind: 'file', name: rest, key, member } : { id: `m:${key}`, kind: 'dir', name: rest, key, member })
  }
  for (const [name, folder] of folders) out.push({ id: `f:${prefix}${name}/`, kind: 'folder', name, prefix: `${prefix}${name}/`, count: folder.inside.size, size: folder.size })
  return out.sort((a, b) => (isFolderLike(a) === isFolderLike(b) ? compare(a.name, b.name) || (a.kind === 'folder' ? 1 : -1) : isFolderLike(a) ? -1 : 1))
}

/** The member names a row stands for (a folder: its `folder` member and every name under it). */
export function keysOf(members: Record<string, DirectoryMember>, entry: Entry): string[] {
  if (entry.kind !== 'folder') return [entry.key]
  return Object.keys(members).filter((key) => key.startsWith(entry.prefix) || key === entry.prefix.slice(0, -1))
}

/** Is there a folder at `prefix` (`a/b/`): a `folder` member of that name, or names under it. The root always is. */
export function folderExists(members: Record<string, DirectoryMember>, prefix: string): boolean {
  if (!prefix) return true
  if (members[prefix.slice(0, -1)]?.kind === 'folder') return true
  return Object.keys(members).some((key) => key.startsWith(prefix))
}

/** What removing `entries` takes away: the files and folders under them (a folder counts itself too) and the linked
 * directories (only unlinked). */
export function removalOf(members: Record<string, DirectoryMember>, entries: readonly Entry[]): { files: number; folders: number; links: number } {
  const out = { files: 0, folders: 0, links: 0 }
  const folders = new Set<string>()
  for (const entry of entries) {
    for (const key of keysOf(members, entry)) {
      const member = members[key]
      if (member?.kind === 'file') out.files += 1
      else if (member?.kind === 'dir') out.links += 1
      else if (member?.kind === 'folder') folders.add(key)
      if (entry.kind !== 'folder') continue
      // the folders a name lies in, from the removed folder down
      const parts = key.split('/')
      for (let i = entry.prefix.split('/').length - 1; i < parts.length; i += 1) folders.add(parts.slice(0, i).join('/'))
    }
    if (entry.kind === 'folder') folders.add(entry.prefix.slice(0, -1))
  }
  out.folders = folders.size
  return out
}

/** Is `name` taken at `prefix`: a member of that name, or a folder of that name. */
export function nameTaken(members: Record<string, DirectoryMember>, prefix: string, name: string): boolean {
  const key = `${prefix}${name}`
  if (Object.hasOwn(members, key)) return true
  const folder = `${key}/`
  return Object.keys(members).some((k) => k.startsWith(folder))
}

/** What a name the user typed is wrong with, or null: one path segment, the whole member name within 255 bytes (the
 * service's rule, `check_member_name`). */
export function nameProblem(prefix: string, name: string): 'empty' | 'invalid' | 'long' | null {
  if (!name.trim()) return 'empty'
  if (name.includes('/') || name.includes('\0') || name === '.' || name === '..') return 'invalid'
  if (new TextEncoder().encode(`${prefix}${name}`).length > 255) return 'long'
  return null
}

/** "Keep both": `photo.png` → `photo (2).png`, the first such name `taken` does not refuse. */
export function keepBothName(name: string, taken: (candidate: string) => boolean): string {
  const dot = name.lastIndexOf('.')
  const stem = dot > 0 ? name.slice(0, dot) : name
  const ext = dot > 0 ? name.slice(dot) : ''
  for (let n = 2; n < 10_000; n += 1) {
    const candidate = `${stem} (${n})${ext}`
    if (!taken(candidate)) return candidate
  }
  return `${stem} (${Date.now()})${ext}`
}

/** Resolve a path below the root (`a/b/c.png`, as a link names it) to a breadcrumb and the last name. `members` reads a
 * directory's members (undefined when it cannot be read). */
export async function resolvePath(rootId: string, path: string, members: (entityId: string) => Promise<Record<string, DirectoryMember> | undefined>): Promise<{ crumbs: Crumb[]; name: string | null } | null> {
  const segments = path.split('/').filter(Boolean)
  const crumbs: Crumb[] = []
  let entityId = rootId
  let prefix = ''
  let current = await members(entityId)
  for (let i = 0; i < segments.length; i += 1) {
    if (!current) return null
    const key = `${prefix}${segments[i]}`
    const member = current[key]
    if (i === segments.length - 1 && member?.kind === 'file') return { crumbs, name: segments[i] }
    if (member?.kind === 'dir') {
      crumbs.push({ kind: 'dir', name: segments[i], entityId: member.entity_id })
      entityId = member.entity_id
      prefix = ''
      current = await members(entityId)
    } else if (folderExists(current, `${key}/`)) {
      crumbs.push({ kind: 'folder', name: segments[i] })
      prefix = `${key}/`
    } else return null
  }
  return { crumbs, name: null }
}

/** The path of a member below the root (for its link). */
export function pathOf(crumbs: readonly Crumb[], name: string): string {
  return [...crumbs.map((crumb) => crumb.name), name].join('/')
}
