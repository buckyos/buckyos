/* ── Search + filter bar for collection list ── */

import { Search } from 'lucide-react'

interface SearchFilterBarProps {
  query: string
  onQueryChange: (q: string) => void
  placeholder?: string
}

export function SearchFilterBar({ query, onQueryChange, placeholder = 'Search…' }: SearchFilterBarProps) {
  return (
    <div
      className="flex items-center gap-2 px-3 py-2 mx-2 mt-2 mb-1 rounded-[12px]"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 60%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <Search size={14} style={{ color: 'var(--cp-muted)', flexShrink: 0 }} />
      <input
        type="text"
        value={query}
        onChange={(e) => onQueryChange(e.target.value)}
        placeholder={placeholder}
        className="flex-1 bg-transparent text-sm outline-none"
        style={{ color: 'var(--cp-text)' }}
      />
    </div>
  )
}
