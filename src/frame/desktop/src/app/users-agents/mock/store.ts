/* ── Users & Agents – mock store ── */

import type {
  UsersAgentsSnapshot,
  SelfEntity,
  AgentEntity,
  LocalUserEntity,
  ContactEntity,
  EntityGroupEntity,
  Collection,
  AnyEntity,
} from './types'
import {
  mockSelf,
  mockAgent,
  mockLocalUsers,
  mockContacts,
  mockEntityGroups,
  mockCollections,
} from './seed'

export class UsersAgentsMockStore {
  private self: SelfEntity
  private agent: AgentEntity
  private localUsers: LocalUserEntity[]
  private contacts: ContactEntity[]
  private entityGroups: EntityGroupEntity[]
  private collections: Collection[]

  private snapshot: UsersAgentsSnapshot
  private listeners = new Set<() => void>()

  constructor() {
    this.self = structuredClone(mockSelf)
    this.agent = structuredClone(mockAgent)
    this.localUsers = structuredClone(mockLocalUsers)
    this.contacts = structuredClone(mockContacts)
    this.entityGroups = structuredClone(mockEntityGroups)
    this.collections = structuredClone(mockCollections)
    this.snapshot = this.buildSnapshot()
  }

  // ── external store protocol ──

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = (): UsersAgentsSnapshot => this.snapshot

  private notify() {
    this.snapshot = this.buildSnapshot()
    this.listeners.forEach((l) => l())
  }

  private buildSnapshot(): UsersAgentsSnapshot {
    return {
      self: this.self,
      agent: this.agent,
      localUsers: this.localUsers,
      contacts: this.contacts,
      entityGroups: this.entityGroups,
      collections: this.collections,
    }
  }

  // ── entity lookup ──

  findEntity(id: string): AnyEntity | undefined {
    if (this.self.id === id) return this.self
    if (this.agent.id === id) return this.agent
    return (
      this.localUsers.find((u) => u.id === id) ??
      this.contacts.find((c) => c.id === id) ??
      this.entityGroups.find((g) => g.id === id)
    )
  }

  // ── local user CRUD ──

  addLocalUser(user: LocalUserEntity) {
    this.localUsers = [...this.localUsers, user]
    this.notify()
  }

  removeLocalUser(id: string) {
    this.localUsers = this.localUsers.filter((u) => u.id !== id)
    this.notify()
  }

  // ── contact CRUD ──

  addContacts(contacts: ContactEntity[]) {
    this.contacts = [...this.contacts, ...contacts]
    this.notify()
  }

  removeContact(id: string) {
    this.contacts = this.contacts.filter((c) => c.id !== id)
    // also remove from collections
    this.collections = this.collections.map((col) => ({
      ...col,
      entityIds: col.entityIds.filter((eid) => eid !== id),
    }))
    this.notify()
  }

  // ── collection CRUD ──

  addCollection(name: string) {
    const col: Collection = {
      id: `col-custom-${Date.now()}`,
      name,
      type: 'custom',
      isBuiltIn: false,
      entityIds: [],
      createdAt: new Date().toISOString(),
    }
    this.collections = [...this.collections, col]
    this.notify()
    return col
  }

  renameCollection(id: string, name: string) {
    this.collections = this.collections.map((c) =>
      c.id === id ? { ...c, name } : c,
    )
    this.notify()
  }

  removeCollection(id: string) {
    this.collections = this.collections.filter((c) => c.id !== id)
    this.notify()
  }

  addToCollection(collectionId: string, entityId: string) {
    this.collections = this.collections.map((c) =>
      c.id === collectionId && !c.entityIds.includes(entityId)
        ? { ...c, entityIds: [...c.entityIds, entityId] }
        : c,
    )
    this.notify()
  }

  removeFromCollection(collectionId: string, entityId: string) {
    this.collections = this.collections.map((c) =>
      c.id === collectionId
        ? { ...c, entityIds: c.entityIds.filter((eid) => eid !== entityId) }
        : c,
    )
    this.notify()
  }
}
