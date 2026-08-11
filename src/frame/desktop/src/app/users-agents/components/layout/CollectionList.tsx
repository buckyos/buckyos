/* ── Collection element list (middle column in Mode B) ── */

import { useMemo, useState } from 'react'
import { Button, Chip, Dialog, DialogActions, DialogContent, DialogTitle, ToggleButton, ToggleButtonGroup } from '@mui/material'
import { Download, Merge, Trash2 } from 'lucide-react'
import { SearchFilterBar } from '../shared/SearchFilterBar'
import { CollectionListItem } from '../shared/CollectionListItem'
import { useCollection, useCollectionEntities, useMyNetworkStore } from '../../../my-network/hooks/use-my-network-store'
import type { ContactEntity, MyNetworkEntity } from '../../../my-network/datamodel/types'

interface CollectionListProps {
  collectionId: string
  selectedElementId: string | null
  onSelectElement: (id: string) => void
  wide?: boolean
}

function CollectionListContent({ collectionId, selectedElementId, onSelectElement, wide = false }: CollectionListProps) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<'all' | 'contacts' | 'groups' | 'imported'>('all')
  const [sort, setSort] = useState<'name' | 'source' | 'updated'>('name')
  const [selectedIds, setSelectedIds] = useState<string[]>([])
  const [importOpen, setImportOpen] = useState(false)
  const [importStage, setImportStage] = useState<'idle' | 'importing' | 'complete'>('idle')
  const [mergeOpen, setMergeOpen] = useState(false)
  const collection = useCollection(collectionId)
  const entities = useCollectionEntities(collectionId)
  const store = useMyNetworkStore()

  const filtered = useMemo(() => {
    const q = query.toLowerCase()
    return entities
      .filter((entity: MyNetworkEntity) => {
        if (filter === 'contacts') return entity.kind === 'contact'
        if (filter === 'groups') return entity.kind === 'entity-group'
        if (filter === 'imported') {
          return entity.kind === 'contact' && entity.source !== 'manual'
        }
        return true
      })
      .filter((entity: MyNetworkEntity) => {
        if (!q.trim()) return true
        return entity.displayName.toLowerCase().includes(q) ||
          (entity.did?.toLowerCase().includes(q) ?? false)
      })
      .sort((left, right) => {
        if (sort === 'source') {
          const leftSource = left.kind === 'contact' ? left.source : left.kind
          const rightSource = right.kind === 'contact' ? right.source : right.kind
          return leftSource.localeCompare(rightSource)
        }
        if (sort === 'updated') {
          return right.createdAt.localeCompare(left.createdAt)
        }
        return left.displayName.localeCompare(right.displayName)
      })
  }, [entities, filter, query, sort])

  if (!collection) return null

  const selectedEntities = entities.filter((entity) => selectedIds.includes(entity.id))
  const selectedContacts = selectedEntities.filter((entity) => entity.kind === 'contact') as ContactEntity[]

  const toggleSelected = (id: string) => {
    setSelectedIds((current) =>
      current.includes(id)
        ? current.filter((item) => item !== id)
        : [...current, id],
    )
  }

  const handleImport = () => {
    setImportStage('importing')
    window.setTimeout(() => {
      const now = new Date().toISOString()
      store.addContacts([
        {
          id: `ct-import-${Date.now()}-1`,
          kind: 'contact',
          displayName: 'Mina.imported',
          sourceLabel: 'Mina.imported',
          socialAccounts: [{ id: `b-import-${Date.now()}`, platform: 'email', accountId: 'mina@example.com', displayId: 'mina@example.com', status: 'active', isPublic: false, canIdentify: true }],
          source: 'imported',
          isVerified: false,
          relation: 'one-way',
          importBatch: `csv-${Date.now()}`,
          tags: ['imported'],
          notes: 'Imported without overwriting existing contacts.',
          createdAt: now,
        },
        {
          id: `ct-import-${Date.now()}-2`,
          kind: 'contact',
          displayName: 'Grace.imported',
          sourceLabel: 'Grace.imported',
          socialAccounts: [],
          source: 'imported',
          isVerified: false,
          relation: 'one-way',
          importBatch: `csv-${Date.now()}`,
          isMergeCandidate: true,
          tags: ['imported', 'candidate'],
          createdAt: now,
        },
      ])
      setImportStage('complete')
    }, 500)
  }

  const handleMerge = () => {
    if (selectedContacts.length < 2) return
    store.mergeContacts(selectedContacts[0].id, selectedContacts.map((contact) => contact.id))
    setSelectedIds([])
    setMergeOpen(false)
  }

  return (
    <div
      className={`flex flex-col h-full overflow-hidden ${wide ? 'w-full' : 'w-64 shrink-0'}`}
      style={{
        borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      {/* collection header */}
      <div className="px-4 pt-3 pb-1">
        <div className="flex items-center justify-between gap-2">
          <h3
            className="font-display text-sm font-semibold truncate"
            style={{ color: 'var(--cp-text)' }}
          >
            {collection.name}
          </h3>
          {collection.type === 'contacts' && (
            <Button
              size="small"
              variant="text"
              startIcon={<Download size={13} />}
              onClick={() => {
                setImportOpen(true)
                setImportStage('idle')
              }}
            >
              Import
            </Button>
          )}
        </div>
        <div
          className="text-[11px] mt-0.5"
          style={{ color: 'var(--cp-muted)' }}
        >
          {entities.length} items · {collection.mode} {collection.sourceType} · updated {new Date(collection.updatedAt).toLocaleDateString()}
        </div>
        {collection.description && (
          <div className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
            {collection.description}
          </div>
        )}
      </div>

      <SearchFilterBar
        query={query}
        onQueryChange={setQuery}
        placeholder={`Search ${collection.name}…`}
      />

      <div className="mx-2 mt-1 flex flex-wrap gap-1">
        {(['all', 'contacts', 'groups', 'imported'] as const).map((value) => (
          <Chip
            key={value}
            label={value}
            size="small"
            variant={filter === value ? 'filled' : 'outlined'}
            onClick={() => setFilter(value)}
          />
        ))}
      </div>

      <div className="mx-2 mt-2">
        <ToggleButtonGroup
          value={sort}
          exclusive
          size="small"
          fullWidth
          onChange={(_, value: typeof sort | null) => {
            if (value) setSort(value)
          }}
        >
          <ToggleButton value="name">Name</ToggleButton>
          <ToggleButton value="source">Source</ToggleButton>
          <ToggleButton value="updated">Recent</ToggleButton>
        </ToggleButtonGroup>
      </div>

      {selectedIds.length > 0 && (
        <div
          className="mx-2 mt-2 rounded-[14px] px-2 py-2"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent-soft) 12%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-accent) 20%, transparent)',
          }}
        >
          <div className="mb-2 text-[12px] font-medium" style={{ color: 'var(--cp-text)' }}>
            {selectedIds.length} selected
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Button
              size="small"
              variant="outlined"
              startIcon={<Merge size={13} />}
              disabled={selectedContacts.length < 2}
              onClick={() => setMergeOpen(true)}
            >
              Merge
            </Button>
            <Button
              size="small"
              color="error"
              variant="outlined"
              startIcon={<Trash2 size={13} />}
              disabled={collection.isReadOnly}
              onClick={() => {
                store.removeManyFromCollection(collection.id, selectedIds)
                setSelectedIds([])
              }}
            >
              Remove
            </Button>
          </div>
        </div>
      )}

      {/* list */}
      <div className="flex-1 overflow-y-auto desktop-scrollbar mt-1">
        {filtered.length === 0 ? (
          <div
            className="px-4 py-8 text-center text-sm"
            style={{ color: 'var(--cp-muted)' }}
          >
            {query ? 'No matches found.' : 'No items in this collection.'}
          </div>
        ) : (
          filtered.map((entity) => (
            <CollectionListItem
              key={entity.id}
              entity={entity}
              isActive={entity.id === selectedElementId}
              isSelected={selectedIds.includes(entity.id)}
              onClick={() => onSelectElement(entity.id)}
              onToggleSelected={() => toggleSelected(entity.id)}
            />
          ))
        )}
      </div>

      <Dialog open={importOpen} onClose={() => setImportOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>Import contacts</DialogTitle>
        <DialogContent>
          {importStage === 'idle' && (
            <div className="space-y-2 pt-1 text-sm" style={{ color: 'var(--cp-text)' }}>
              <p>
                Imported contacts land in Contacts first. Existing profiles
                are not overwritten; possible duplicates are marked as merge
                candidates.
              </p>
              <div className="flex flex-wrap gap-1.5">
                <Chip label="CSV" size="small" />
                <Chip label="XML" size="small" />
                <Chip label="Address book" size="small" />
              </div>
            </div>
          )}
          {importStage === 'importing' && (
            <div className="py-6 text-center text-sm" style={{ color: 'var(--cp-muted)' }}>
              Importing contacts and preserving source history...
            </div>
          )}
          {importStage === 'complete' && (
            <div className="space-y-2 py-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <p>Import complete. Two contacts were added to Contacts.</p>
              <p style={{ color: 'var(--cp-muted)' }}>
                Grace.imported is marked as a merge candidate so the user can
                review before combining records.
              </p>
            </div>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setImportOpen(false)}>
            {importStage === 'complete' ? 'Done' : 'Cancel'}
          </Button>
          {importStage === 'idle' && (
            <Button variant="contained" onClick={handleImport}>
              Start import
            </Button>
          )}
        </DialogActions>
      </Dialog>

      <Dialog open={mergeOpen} onClose={() => setMergeOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>Merge contacts</DialogTitle>
        <DialogContent>
          <div className="space-y-2 pt-1 text-sm" style={{ color: 'var(--cp-text)' }}>
            <p>
              {selectedContacts[0]?.displayName} will be kept as the primary
              contact. The other selected records will be removed from all
              collections after their source information is preserved in notes.
            </p>
            <div className="flex flex-wrap gap-1.5">
              {selectedContacts.map((contact) => (
                <Chip key={contact.id} label={contact.displayName} size="small" />
              ))}
            </div>
          </div>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setMergeOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={handleMerge}>Merge</Button>
        </DialogActions>
      </Dialog>
    </div>
  )
}

export function CollectionList(props: CollectionListProps) {
  return <CollectionListContent key={props.collectionId} {...props} />
}
