/* A directory's members as a file list (框架 PRD §7.4): a `buckyos.dir` maps member names to files or to child
 * directories (other `buckyos.dir` entities). A name may hold `/` (an extension's sources: `src/main.js`), so a
 * directory also has folders of its own made of name prefixes. A place in the list is a directory entity and a prefix
 * inside it; the breadcrumb is the way down from the editor's root. Nothing here touches React or the store. */

import type { KeyedContent } from '../../api/types'

export type FileMember = { kind: 'file'; object_id: string; media_type?: string; size?: number }
export type DirRefMember = { kind: 'dir'; entity_id: string }
export type DirectoryMember = FileMember | DirRefMember
export type DirectoryPayload = { members: Record<string, DirectoryMember> }
export type DirectoryContent = KeyedContent<DirectoryPayload>

/** Where the list is: a directory entity and a name prefix inside it (`''`, or `a/b/`). */
export interface Place { entityId: string; prefix: string }

/** One step of the breadcrumb below the root: into a child directory, or into a folder of names. */
export type Crumb = { kind: 'dir'; name: string; entityId: string } | { kind: 'folder'; name: string }

/** One row of the list. `key` is the member name (files and child directories); a folder is every name under `prefix`. */
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

/** The rows at `prefix`: folders and child directories first, then files, each by name. */
export function entriesAt(members: Record<string, DirectoryMember>, prefix: string, compare: (a: string, b: string) => number): Entry[] {
  const folders = new Map<string, { count: number; size: number }>()
  const out: Entry[] = []
  for (const [key, member] of Object.entries(members)) {
    if (!key.startsWith(prefix)) continue
    const rest = key.slice(prefix.length)
    const slash = rest.indexOf('/')
    if (slash >= 0) {
      const name = rest.slice(0, slash)
      const folder = folders.get(name) ?? { count: 0, size: 0 }
      folder.count += 1
      folder.size += member.kind === 'file' ? member.size ?? 0 : 0
      folders.set(name, folder)
      continue
    }
    out.push(member.kind === 'file' ? { id: `m:${key}`, kind: 'file', name: rest, key, member } : { id: `m:${key}`, kind: 'dir', name: rest, key, member })
  }
  for (const [name, folder] of folders) out.push({ id: `f:${prefix}${name}/`, kind: 'folder', name, prefix: `${prefix}${name}/`, ...folder })
  return out.sort((a, b) => (isFolderLike(a) === isFolderLike(b) ? compare(a.name, b.name) || (a.kind === 'folder' ? 1 : -1) : isFolderLike(a) ? -1 : 1))
}

/** The member names a row stands for (a folder: every name under it). */
export function keysOf(members: Record<string, DirectoryMember>, entry: Entry): string[] {
  if (entry.kind !== 'folder') return [entry.key]
  return Object.keys(members).filter((key) => key.startsWith(entry.prefix))
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
    } else if (Object.keys(current).some((k) => k.startsWith(`${key}/`))) {
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
