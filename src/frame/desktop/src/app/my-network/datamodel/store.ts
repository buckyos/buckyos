import { isMockRuntime } from '../../../runtime'
import {
  createEmptyMyNetworkSnapshot,
  createMockMyNetworkSnapshot,
  fetchMyNetworkSnapshot,
} from './api'
import type {
  Collection,
  ContactEntity,
  MyNetworkEntity,
  MyNetworkSnapshot,
} from './types'

export interface MyNetworkStoreOptions {
  useMock?: boolean
}

const mockModeValues = new Set(['1', 'true', 'yes', 'mock'])

function defaultUseMock(): boolean {
  const value = import.meta.env.VITE_USERS_AGENTS_USE_MOCK
  if (value !== undefined) {
    return mockModeValues.has(String(value).toLowerCase())
  }
  return isMockRuntime()
}

export class MyNetworkStore {
  private readonly useMock: boolean

  private contacts: ContactEntity[]
  private entityGroups: MyNetworkSnapshot['entityGroups']
  private collections: Collection[]

  private snapshot: MyNetworkSnapshot
  private listeners = new Set<() => void>()

  constructor(options: MyNetworkStoreOptions = {}) {
    this.useMock = options.useMock ?? defaultUseMock()
    const initial = this.useMock
      ? createMockMyNetworkSnapshot()
      : createEmptyMyNetworkSnapshot()
    this.contacts = initial.contacts
    this.entityGroups = initial.entityGroups
    this.collections = initial.collections
    this.snapshot = this.buildSnapshot()

    if (!this.useMock) {
      void this.reload()
    }
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = (): MyNetworkSnapshot => this.snapshot

  async reload() {
    if (this.useMock) {
      this.applySnapshot(createMockMyNetworkSnapshot())
      this.notify()
      return
    }

    try {
      this.applySnapshot(await fetchMyNetworkSnapshot())
      this.notify()
    } catch (error) {
      console.warn('Failed to load my-network datamodel from backend.', error)
    }
  }

  private applySnapshot(snapshot: MyNetworkSnapshot) {
    this.contacts = snapshot.contacts
    this.entityGroups = snapshot.entityGroups
    this.collections = snapshot.collections
  }

  private notify() {
    this.snapshot = this.buildSnapshot()
    this.listeners.forEach((listener) => listener())
  }

  private buildSnapshot(): MyNetworkSnapshot {
    return {
      contacts: this.contacts,
      entityGroups: this.entityGroups,
      collections: this.collections,
    }
  }

  findEntity(id: string): MyNetworkEntity | undefined {
    return (
      this.contacts.find((contact) => contact.id === id) ??
      this.entityGroups.find((group) => group.id === id)
    )
  }

  addContacts(contacts: ContactEntity[]) {
    this.contacts = [...this.contacts, ...contacts]
    const contactIds = contacts.map((contact) => contact.id)
    this.collections = this.collections.map((collection) =>
      collection.type === 'contacts'
        ? {
            ...collection,
            entityIds: Array.from(new Set([...collection.entityIds, ...contactIds])),
            updatedAt: new Date().toISOString(),
          }
        : collection,
    )
    this.notify()
  }

  removeContact(id: string) {
    this.contacts = this.contacts.filter((contact) => contact.id !== id)
    this.collections = this.collections.map((collection) => ({
      ...collection,
      entityIds: collection.entityIds.filter((entityId) => entityId !== id),
    }))
    this.notify()
  }

  mergeContacts(primaryId: string, mergedIds: string[]) {
    const mergedSet = new Set(mergedIds.filter((id) => id !== primaryId))
    if (mergedSet.size === 0) return

    const mergedNames = this.contacts
      .filter((contact) => mergedSet.has(contact.id))
      .map((contact) => contact.displayName)

    this.contacts = this.contacts
      .filter((contact) => !mergedSet.has(contact.id))
      .map((contact) =>
        contact.id === primaryId
          ? {
              ...contact,
              isMergeCandidate: false,
              tags: Array.from(new Set([...contact.tags, 'merged'])),
              notes: [
                contact.notes,
                mergedNames.length > 0
                  ? `Merged with ${mergedNames.join(', ')}.`
                  : undefined,
              ].filter(Boolean).join(' '),
            }
          : contact,
      )

    this.collections = this.collections.map((collection) => {
      const nextIds = collection.entityIds.map((id) =>
        mergedSet.has(id) ? primaryId : id,
      )
      return {
        ...collection,
        entityIds: Array.from(new Set(nextIds)),
        updatedAt: new Date().toISOString(),
      }
    })
    this.notify()
  }

  addCollection(name: string, description = '') {
    const collection: Collection = {
      id: `col-custom-${Date.now()}`,
      name,
      type: 'custom',
      mode: 'static',
      description,
      sourceType: 'manual',
      isBuiltIn: false,
      isReadOnly: false,
      entityIds: [],
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    }
    this.collections = [...this.collections, collection]
    this.notify()
    return collection
  }

  renameCollection(id: string, name: string) {
    this.collections = this.collections.map((collection) =>
      collection.id === id
        ? { ...collection, name, updatedAt: new Date().toISOString() }
        : collection,
    )
    this.notify()
  }

  removeCollection(id: string) {
    this.collections = this.collections.filter((collection) =>
      collection.id !== id || collection.isBuiltIn,
    )
    this.notify()
  }

  addToCollection(collectionId: string, entityId: string) {
    this.collections = this.collections.map((collection) =>
      collection.id === collectionId &&
      !collection.isReadOnly &&
      !collection.entityIds.includes(entityId)
        ? {
            ...collection,
            entityIds: [...collection.entityIds, entityId],
            updatedAt: new Date().toISOString(),
          }
        : collection,
    )
    this.notify()
  }

  removeManyFromCollection(collectionId: string, entityIds: string[]) {
    const removeSet = new Set(entityIds)
    this.collections = this.collections.map((collection) =>
      collection.id === collectionId && !collection.isReadOnly
        ? {
            ...collection,
            entityIds: collection.entityIds.filter((id) => !removeSet.has(id)),
            updatedAt: new Date().toISOString(),
          }
        : collection,
    )
    this.notify()
  }
}
