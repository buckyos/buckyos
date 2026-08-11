import { useMemo, useState } from 'react'
import { Button, Chip, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, TextField, ToggleButton, ToggleButtonGroup, useMediaQuery } from '@mui/material'
import { Plus, Search, UserPlus, Users2 } from 'lucide-react'
import { CollectionCard } from '../users-agents/components/cards/CollectionCard'
import { ContactDetailPage } from '../users-agents/components/detail/ContactDetailPage'
import { EntityGroupDetailPage } from '../users-agents/components/detail/EntityGroupDetailPage'
import { CollectionList } from '../users-agents/components/layout/CollectionList'
import { SearchFilterBar } from '../users-agents/components/shared/SearchFilterBar'
import { MyNetworkStore } from './datamodel/store'
import type { MyNetworkSelection } from './datamodel/types'
import { MyNetworkStoreContext, useCollections, useEntity, useMyNetworkStore } from './hooks/use-my-network-store'

type CollectionCreateMode = 'manual' | 'import'

function NetworkDetail({ entityId, onRemoved }: { entityId: string | null; onRemoved: () => void }) {
  const entity = useEntity(entityId ?? '')

  if (!entity) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-sm" style={{ color: 'var(--cp-muted)' }}>
        Select a contact, joined group, or collection member.
      </div>
    )
  }

  if (entity.kind === 'contact') {
    return <ContactDetailPage contact={entity} onRemoved={onRemoved} />
  }

  if (entity.kind === 'entity-group') {
    return <EntityGroupDetailPage group={entity} />
  }

  return (
    <div className="flex h-full items-center justify-center px-8 text-center text-sm" style={{ color: 'var(--cp-muted)' }}>
      This relationship view only opens contacts and external groups.
    </div>
  )
}

function MyNetworkShell() {
  const collections = useCollections()
  const store = useMyNetworkStore()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [selection, setSelection] = useState<MyNetworkSelection>(() => ({ kind: 'collection', collectionId: 'col-contacts' }))
  const [selectedElementId, setSelectedElementId] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [showSearch, setShowSearch] = useState(false)
  const [createOpen, setCreateOpen] = useState(false)
  const [createMode, setCreateMode] = useState<CollectionCreateMode>('manual')
  const [collectionName, setCollectionName] = useState('')
  const [collectionDescription, setCollectionDescription] = useState('')
  const [notice, setNotice] = useState<string | null>(null)

  const selectedCollectionId = selection.kind === 'collection' ? selection.collectionId : 'col-contacts'
  const filteredCollections = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return collections
    return collections.filter((collection) =>
      [collection.name, collection.description, collection.type, collection.mode]
        .filter(Boolean)
        .join(' ')
        .toLowerCase()
        .includes(normalized),
    )
  }, [collections, query])

  const selectCollection = (collectionId: string) => {
    setSelection({ kind: 'collection', collectionId })
    setSelectedElementId(null)
  }

  const addContact = () => {
    const now = new Date().toISOString()
    const id = `ct-manual-${Date.now()}`
    store.addContacts([
      {
        id,
        kind: 'contact',
        displayName: 'Nora',
        did: 'did:bns:nora',
        socialAccounts: [],
        source: 'manual',
        isVerified: false,
        relation: 'one-way',
        tags: ['new'],
        notes: 'Added from My Network.',
        createdAt: now,
      },
    ])
    selectCollection('col-contacts')
    setSelectedElementId(id)
  }

  const openCreateCollection = () => {
    setCollectionName('')
    setCollectionDescription('')
    setCreateMode('manual')
    setCreateOpen(true)
  }

  const createCollection = () => {
    const fallbackName = createMode === 'manual' ? 'New Contact Collection' : 'Imported Contact Group'
    const collection = store.addCollection(collectionName.trim() || fallbackName, collectionDescription.trim())
    if (createMode === 'import') {
      store.addToCollection(collection.id, 'ct-001')
      store.addToCollection(collection.id, 'ct-002')
      store.addToCollection(collection.id, 'ct-008')
    }
    setCreateOpen(false)
    selectCollection(collection.id)
  }

  const sidebar = (
    <aside
      className="flex h-full w-64 shrink-0 flex-col overflow-y-auto desktop-scrollbar"
      style={{ borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }}
    >
      <div className="px-4 pb-2 pt-3">
        <div className="flex items-center justify-between gap-2">
          <div>
            <div className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              My Network
            </div>
            <div className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
              {collections.length} collections
            </div>
          </div>
          <IconButton size="small" aria-label="Search collections" onClick={() => setShowSearch((value) => !value)}>
            <Search size={14} />
          </IconButton>
        </div>

        {(showSearch || query) && (
          <SearchFilterBar query={query} onQueryChange={setQuery} placeholder="Search collections..." />
        )}

        <div className="mt-3 grid grid-cols-2 gap-2">
          <Button size="small" variant="contained" startIcon={<UserPlus size={14} />} onClick={addContact}>
            Add Contact
          </Button>
          <Button size="small" variant="outlined" startIcon={<Users2 size={14} />} onClick={() => setNotice('Join Group is coming soon.')}>
            Join Group
          </Button>
        </div>

        <Button className="mt-2 w-full" size="small" variant="text" startIcon={<Plus size={14} />} onClick={openCreateCollection}>
          Add Collection
        </Button>
      </div>

      <div className="space-y-0.5 px-2 pb-3">
        {filteredCollections.map((collection) => (
          <CollectionCard
            key={collection.id}
            collection={collection}
            isActive={selectedCollectionId === collection.id}
            onClick={() => selectCollection(collection.id)}
            onRename={!collection.isBuiltIn ? () => {
              const nextName = window.prompt('Rename collection:', collection.name)
              if (nextName?.trim()) store.renameCollection(collection.id, nextName.trim())
            } : undefined}
            onDelete={!collection.isBuiltIn ? () => {
              if (window.confirm('Delete this collection?')) {
                store.removeCollection(collection.id)
                selectCollection('col-contacts')
              }
            } : undefined}
          />
        ))}
      </div>
    </aside>
  )

  const collectionList = (
    <CollectionList
      collectionId={selectedCollectionId}
      selectedElementId={selectedElementId}
      onSelectElement={setSelectedElementId}
      wide={isMobile}
    />
  )

  return (
    <div className="flex h-full w-full overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
      {isMobile ? (
        <div className="flex h-full w-full flex-col overflow-hidden">
          <div className="shrink-0 px-4 py-3" style={{ borderBottom: '1px solid color-mix(in srgb, var(--cp-border) 55%, transparent)' }}>
            <div className="flex items-center justify-between">
              <div className="font-display text-base font-semibold" style={{ color: 'var(--cp-text)' }}>My Network</div>
              <div className="flex gap-1">
                <IconButton size="small" aria-label="Add Contact" onClick={addContact}><UserPlus size={14} /></IconButton>
                <IconButton size="small" aria-label="Add Collection" onClick={openCreateCollection}><Plus size={14} /></IconButton>
              </div>
            </div>
            <div className="mt-2 flex gap-1 overflow-x-auto pb-1">
              {collections.map((collection) => (
                <Chip
                  key={collection.id}
                  label={collection.name}
                  size="small"
                  variant={selectedCollectionId === collection.id ? 'filled' : 'outlined'}
                  onClick={() => selectCollection(collection.id)}
                />
              ))}
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-hidden">
            {selectedElementId ? (
              <div className="h-full overflow-y-auto desktop-scrollbar px-4 py-4">
                <Button size="small" onClick={() => setSelectedElementId(null)}>Back</Button>
                <NetworkDetail entityId={selectedElementId} onRemoved={() => setSelectedElementId(null)} />
              </div>
            ) : (
              collectionList
            )}
          </div>
        </div>
      ) : (
        <>
          {sidebar}
          {collectionList}
          <main className="min-w-0 flex-1 overflow-y-auto desktop-scrollbar">
            <div className="px-6 py-5">
              <NetworkDetail entityId={selectedElementId} onRemoved={() => setSelectedElementId(null)} />
            </div>
          </main>
        </>
      )}

      <Dialog open={createOpen} onClose={() => setCreateOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>Add Collection</DialogTitle>
        <DialogContent>
          <ToggleButtonGroup
            value={createMode}
            exclusive
            fullWidth
            size="small"
            onChange={(_, value: CollectionCreateMode | null) => {
              if (value) setCreateMode(value)
            }}
          >
            <ToggleButton value="manual">Manual Group</ToggleButton>
            <ToggleButton value="import">Import Group</ToggleButton>
          </ToggleButtonGroup>

          <div className="mt-3 space-y-3">
            <TextField
              label="Group Name"
              value={collectionName}
              onChange={(event) => setCollectionName(event.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Description"
              value={collectionDescription}
              onChange={(event) => setCollectionDescription(event.target.value)}
              fullWidth
              size="small"
              multiline
              minRows={2}
            />
            {createMode === 'import' && (
              <div className="flex flex-wrap gap-1.5">
                <Chip label="Eve" size="small" />
                <Chip label="Frank" size="small" />
                <Chip label="Lily" size="small" />
              </div>
            )}
          </div>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCreateOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={createCollection}>Create</Button>
        </DialogActions>
      </Dialog>

      <Dialog open={Boolean(notice)} onClose={() => setNotice(null)} fullWidth maxWidth="xs">
        <DialogTitle>Coming Soon</DialogTitle>
        <DialogContent>
          <div className="text-sm" style={{ color: 'var(--cp-text)' }}>{notice}</div>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setNotice(null)}>Done</Button>
        </DialogActions>
      </Dialog>
    </div>
  )
}

export function MyNetworkAppPanel() {
  const [store] = useState(() => new MyNetworkStore())

  return (
    <MyNetworkStoreContext.Provider value={store}>
      <MyNetworkShell />
    </MyNetworkStoreContext.Provider>
  )
}
