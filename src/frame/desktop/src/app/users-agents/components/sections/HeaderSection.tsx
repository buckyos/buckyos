/* ── Shared entity card section ── */

import { Copy, ExternalLink, ImagePlus, Pencil } from 'lucide-react'
import { Button, IconButton } from '@mui/material'
import { EntityAvatar } from '../shared/EntityAvatar'
import type { EntityAvatarKind } from '../shared/EntityAvatar'

interface HeaderSectionProps {
  name: string
  kind: EntityAvatarKind
  avatarUrl?: string
  did?: string
  subtitle?: string
  badges?: React.ReactNode
  isOnline?: boolean
  previewUrl?: string
  onAvatarEdit?: () => void
  onSubtitleEdit?: () => void
}

export function HeaderSection({
  name,
  kind,
  avatarUrl,
  did,
  subtitle,
  badges,
  isOnline,
  previewUrl,
  onAvatarEdit,
  onSubtitleEdit,
}: HeaderSectionProps) {
  const copyDid = () => {
    if (did) navigator.clipboard.writeText(did)
  }

  const openPreview = () => {
    if (previewUrl) {
      window.open(previewUrl, '_blank', 'noopener,noreferrer')
    }
  }

  return (
    <div
      className="flex flex-col gap-4 px-5 py-5 rounded-[22px] sm:flex-row sm:items-start"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 50%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      <div className="flex items-center gap-3 sm:flex-col sm:items-start">
        <EntityAvatar name={name} kind={kind} avatarUrl={avatarUrl} size="lg" isOnline={isOnline} />
        {onAvatarEdit && (
          <Button size="small" variant="text" startIcon={<ImagePlus size={13} />} onClick={onAvatarEdit}>
            Avatar
          </Button>
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
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
          </div>

          {previewUrl && (
            <Button size="small" variant="outlined" startIcon={<ExternalLink size={13} />} onClick={openPreview}>
              Preview
            </Button>
          )}
        </div>

        {onSubtitleEdit && (
          <Button size="small" variant="text" startIcon={<Pencil size={13} />} onClick={onSubtitleEdit}>
            Bio
          </Button>
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
