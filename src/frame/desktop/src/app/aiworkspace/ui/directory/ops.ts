/* Directory writes (框架 PRD §7.4), all through the existing operations: `dir.set_member` / `dir.remove_member` on the
 * directories, `entity.create` for a new subdirectory. A rename, a move between directories or folders and a removal
 * of several rows is one Commit (one undo step). Each write reads the directories it touches first, so the version
 * each member is expected at is the current one. */

import { randomId } from '../../api/ids'
import { aiwsT } from '../../i18n'
import type { Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { keepBothName, keysOf, nameProblem, nameTaken, type DirectoryContent, type DirectoryMember, type Entry, type Place } from './model'

export type Resolution = 'replace' | 'keep' | 'skip'

export class DirectoryError extends Error {}

const NAME_PROBLEM = { empty: 'aiworkspace.directory.name.empty', invalid: 'aiworkspace.directory.name.invalid', long: 'aiworkspace.directory.name.long' } as const

export async function readDirectory(store: WorkspaceStore, entityId: string) {
  return store.session.read<DirectoryContent>(entityId)
}

const membersOf = (read: Awaited<ReturnType<typeof readDirectory>>) => read.content.payload.members ?? {}
const revOf = (read: Awaited<ReturnType<typeof readDirectory>>, key: string) => read.content.key_revs[`m:${key}`] ?? 0

/** The member as it is written: files by their object (the service fills in type and size). */
const written = (member: DirectoryMember): DirectoryMember => (member.kind === 'file' ? { kind: 'file', object_id: member.object_id } : { kind: 'dir', entity_id: member.entity_id })

const setOp = (entityId: string, key: string, member: DirectoryMember, rev: number): Operation => ({ op: 'dir.set_member', entity_id: entityId, name: key, member: written(member), expect: { rev } })
const removeOp = (entityId: string, key: string, rev: number): Operation => ({ op: 'dir.remove_member', entity_id: entityId, name: key, expect: { rev } })

/** Commit; anything but an accepted (or locally saved) change is an error with the save state's reason. */
export async function submitDirectory(store: WorkspaceStore, editId: string, label: string, operations: Operation[]) {
  if (operations.length === 0) return
  const outcome = await store.submit({ editId, label, operations })
  if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') {
    throw new DirectoryError(store.edits.get(editId)?.detail ?? aiwsT('aiworkspace.directory.saveFailed'))
  }
}

/** Why `name` cannot be used at `place` (null when it can): `members` as read. */
export function nameError(members: Record<string, DirectoryMember>, prefix: string, name: string, own?: string): string | null {
  const problem = nameProblem(prefix, name)
  if (problem) return aiwsT(NAME_PROBLEM[problem])
  if (name !== own && nameTaken(members, prefix, name)) return aiwsT('aiworkspace.directory.nameExists', { name })
  return null
}

/** Rename a row (a folder: every name under it). */
export async function renameEntry(store: WorkspaceStore, place: Place, entry: Entry, to: string) {
  if (to === entry.name) return
  const read = await readDirectory(store, place.entityId)
  const members = membersOf(read)
  const problem = nameError(members, place.prefix, to)
  if (problem) throw new DirectoryError(problem)
  const from = `${place.prefix}${entry.name}`
  const keys = keysOf(members, entry)
  const operations: Operation[] = keys.map((key) => removeOp(place.entityId, key, revOf(read, key)))
  for (const key of keys) {
    const next = `${place.prefix}${to}${key.slice(from.length)}`
    operations.push(setOp(place.entityId, next, members[key], revOf(read, next)))
  }
  await submitDirectory(store, `directory:${place.entityId}:rename:${from}`, aiwsT('aiworkspace.directory.renameEdit', { from: entry.name, to }), operations)
}

/** Remove rows (their members; a child directory is only unlinked). */
export async function removeEntries(store: WorkspaceStore, place: Place, entries: Entry[]) {
  const read = await readDirectory(store, place.entityId)
  const members = membersOf(read)
  const keys = [...new Set(entries.flatMap((entry) => keysOf(members, entry)))].filter((key) => Object.hasOwn(members, key))
  const label = entries.length === 1 ? aiwsT('aiworkspace.directory.removeEdit', { name: entries[0].name }) : aiwsT('aiworkspace.directory.removeManyEdit', { count: entries.length })
  await submitDirectory(store, `directory:${place.entityId}:remove:${randomId()}`, label, keys.map((key) => removeOp(place.entityId, key, revOf(read, key))))
}

/** The names of `entries` already taken at `to` (each needs a resolution before a move). */
export async function moveConflicts(store: WorkspaceStore, entries: Entry[], to: Place): Promise<string[]> {
  const members = membersOf(await readDirectory(store, to.entityId))
  return entries.filter((entry) => nameTaken(members, to.prefix, entry.name)).map((entry) => entry.name)
}

/** Move rows from `from` to `to` in one Commit: the source loses them, the target gains them under the same name, or
 * per `resolutions` (by name) replaces what is there, keeps both under a new name, or leaves the row where it is. */
export async function moveEntries(store: WorkspaceStore, from: Place, entries: Entry[], to: Place, resolutions: Record<string, Resolution> = {}) {
  const same = to.entityId === from.entityId
  const source = await readDirectory(store, from.entityId)
  const target = same ? source : await readDirectory(store, to.entityId)
  const sourceMembers = membersOf(source)
  const targetMembers = { ...membersOf(target) }
  const removes = new Map<string, number>()
  const sets = new Map<string, DirectoryMember>()
  const cleared = new Set<string>()
  const takenAt = (name: string) => nameTaken(targetMembers, to.prefix, name) || [...sets.keys()].some((key) => key === `${to.prefix}${name}` || key.startsWith(`${to.prefix}${name}/`))
  let moved = 0
  for (const entry of entries) {
    if (entry.kind === 'dir' && entry.member.entity_id === to.entityId) throw new DirectoryError(aiwsT('aiworkspace.directory.move.inside'))
    if (same && entry.kind === 'folder' && to.prefix.startsWith(entry.prefix)) throw new DirectoryError(aiwsT('aiworkspace.directory.move.inside'))
    const resolution = takenAt(entry.name) ? resolutions[entry.name] ?? 'skip' : null
    if (resolution === 'skip') continue
    const name = resolution === 'keep' ? keepBothName(entry.name, takenAt) : entry.name
    if (resolution === 'replace') {
      // what holds the name at the target goes: the member of that name and a folder of that name
      for (const key of Object.keys(targetMembers)) {
        if (key === `${to.prefix}${name}` || key.startsWith(`${to.prefix}${name}/`)) { cleared.add(key); delete targetMembers[key] }
      }
    }
    const base = `${from.prefix}${entry.name}`
    for (const key of keysOf(sourceMembers, entry)) {
      const next = `${to.prefix}${name}${key.slice(base.length)}`
      if (nameProblem('', next) === 'long') throw new DirectoryError(aiwsT('aiworkspace.directory.name.long'))
      removes.set(key, revOf(source, key))
      sets.set(next, sourceMembers[key])
    }
    moved += 1
  }
  if (moved === 0) return 0
  const operations: Operation[] = [...removes].map(([key, rev]) => removeOp(from.entityId, key, rev))
  // a replaced member is written over (its own version); one that only held the name goes
  for (const key of cleared) if (!sets.has(key) && !(same && removes.has(key))) operations.push(removeOp(to.entityId, key, revOf(target, key)))
  for (const [key, member] of sets) operations.push(setOp(to.entityId, key, member, revOf(target, key)))
  const editTarget = same ? from.entityId : `${from.entityId}:${to.entityId}`
  await submitDirectory(store, `directory:${editTarget}:move:${randomId()}`, aiwsT('aiworkspace.directory.moveEdit', { count: moved }), operations)
  return moved
}

/** A new, empty subdirectory: a `buckyos.dir` next to the directory it is created in (directories cannot hold tree
 * children), and its member in that directory. */
export async function createSubdirectory(store: WorkspaceStore, place: Place, name: string) {
  const read = await readDirectory(store, place.entityId)
  const problem = nameError(membersOf(read), place.prefix, name)
  if (problem) throw new DirectoryError(problem)
  const host = store.outline.get(place.entityId)
  const parentId = host?.parent_id ?? 'data'
  const siblings = store.outline.childrenOf(parentId)
  const at = siblings.findIndex((item) => item.entity_id === place.entityId)
  const orderKey = store.core.order_key_between(at >= 0 ? siblings[at].order_key : siblings.at(-1)?.order_key, at >= 0 ? siblings[at + 1]?.order_key : undefined)
  const id = randomId('dir')
  const key = `${place.prefix}${name}`
  await submitDirectory(store, `directory:${place.entityId}:mkdir:${key}`, aiwsT('aiworkspace.directory.newFolderEdit', { name }), [
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.dir', parent_id: parentId, order_key: orderKey, name, payload: { members: {} } },
    setOp(place.entityId, key, { kind: 'dir', entity_id: id }, revOf(read, key)),
  ])
  return id
}

/** An existing directory as a member. */
export async function addExistingDirectory(store: WorkspaceStore, place: Place, directoryId: string, name: string) {
  const read = await readDirectory(store, place.entityId)
  const problem = nameError(membersOf(read), place.prefix, name)
  if (problem) throw new DirectoryError(problem)
  const key = `${place.prefix}${name}`
  await submitDirectory(store, `directory:${place.entityId}:m:${key}`, aiwsT('aiworkspace.directory.addEdit', { name }), [setOp(place.entityId, key, { kind: 'dir', entity_id: directoryId }, revOf(read, key))])
}

/** An uploaded file as a member: `replace` writes over the member of that name; a new name taken meanwhile keeps both. */
export async function commitFile(store: WorkspaceStore, place: Place, name: string, objectId: string, mode: 'new' | 'replace') {
  const read = await readDirectory(store, place.entityId)
  const members = membersOf(read)
  const final = mode === 'new' && nameTaken(members, place.prefix, name) ? keepBothName(name, (candidate) => nameTaken(members, place.prefix, candidate)) : name
  const key = `${place.prefix}${final}`
  const label = aiwsT(mode === 'replace' ? 'aiworkspace.directory.replaceEdit' : 'aiworkspace.directory.addFileEdit', { name: final })
  await submitDirectory(store, `directory:${place.entityId}:m:${key}`, label, [setOp(place.entityId, key, { kind: 'file', object_id: objectId }, revOf(read, key))])
  return final
}
