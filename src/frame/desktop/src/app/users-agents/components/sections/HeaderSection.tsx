/* ── Shared entity header section ── */

import { Copy } from 'lucide-react'
import { IconButton } from '@mui/material'
import { EntityAvatar } from '../shared/EntityAvatar'
import type { EntityKind } from '../../mock/types'

interface HeaderSectionProps {
  name: string
  kind: EntityKind
  avatarUrl?: string
  did?: string
  subtitle?: string
  badges?: React.ReactNode
  isOnline?: boolean
}

export function HeaderSection({ name, kind, avatarUrl, did, subtitle, badges, isOnline }: HeaderSectionProps) {
  const copyDid = () => {
    if (did) navigator.clipboard.writeText(did)
  }

  return (
    <div
      className="flex items-start gap-4 px-5 py-5 rounded-[22px]"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 50%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      <EntityAvatar name={name} kind={kind} avatarUrl={avatarUrl} size="lg" isOnline={isOnline} />

      <div className="flex-1 min-w-0">
        <h2
          className="font-display text-xl font-semibold truncate"
          style={{ color: 'var(--cp-text)' }}
        >
          {name}
        </h2>

        {subtitle && (
          <div className="text-sm mt-0.5" style={{ color: 'var(--cp-muted)' }}>
            {subtitle}
          </div>
        )}

        {did && (
          <div className="flex items-center gap-1 mt-1.5">
            <code
              className="text-[12px] px-2 py-0.5 rounded-[8px] truncate"
              style={{
                background: 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface))',
                color: 'var(--cp-accent)',
              }}
            >
              {did}
            </code>
            <IconButton size="small" onClick={copyDid} aria-label="Copy DID">
              <Copy size={12} />
            </IconButton>
          </div>
        )}

        {badges && <div className="flex flex-wrap gap-1.5 mt-2">{badges}</div>}
      </div>
    </div>
  )
}
