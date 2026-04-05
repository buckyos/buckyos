/* ── Collection element list (middle column in Mode B) ── */

import { useState, useMemo } from 'react'
import { SearchFilterBar } from '../shared/SearchFilterBar'
import { CollectionListItem } from '../shared/CollectionListItem'
import { useCollection, useCollectionEntities } from '../../hooks/use-users-agents-store'
import type { AnyEntity } from '../../mock/types'

interface CollectionListProps {
  collectionId: string
  selectedElementId: string | null
  onSelectElement: (id: string) => void
}

export function CollectionList({ collectionId, selectedElementId, onSelectElement }: CollectionListProps) {
  const [query, setQuery] = useState('')
  const collection = useCollection(collectionId)
  const entities = useCollectionEntities(collectionId)

  const filtered = useMemo(() => {
    if (!query.trim()) return entities
    const q = query.toLowerCase()
    return entities.filter((e: AnyEntity) =>
      e.displayName.toLowerCase().includes(q) ||
      (e.did?.toLowerCase().includes(q) ?? false),
    )
  }, [entities, query])

  if (!collection) return null

  return (
    <div
      className="flex flex-col h-full w-64 shrink-0 overflow-hidden"
      style={{
        borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      {/* collection header */}
      <div className="px-4 pt-3 pb-1">
        <h3
          className="font-display text-sm font-semibold truncate"
          style={{ color: 'var(--cp-text)' }}
        >
          {collection.name}
        </h3>
        <div
          className="text-[11px] mt-0.5"
          style={{ color: 'var(--cp-muted)' }}
        >
          {entities.length} items
        </div>
      </div>

      <SearchFilterBar
        query={query}
        onQueryChange={setQuery}
        placeholder={`Search ${collection.name}…`}
      />

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
              onClick={() => onSelectElement(entity.id)}
            />
          ))
        )}
      </div>
    </div>
  )
}
