/* ── Collection card for sidebar ── */

import { useState } from 'react'
import { Heart, Users2, FolderOpen, MoreHorizontal, Pencil, Trash2 } from 'lucide-react'
import { IconButton, Menu, MenuItem, ListItemIcon, ListItemText } from '@mui/material'
import type { Collection } from '../../mock/types'

interface CollectionCardProps {
  collection: Collection
  isActive: boolean
  onClick: () => void
  onRename?: () => void
  onDelete?: () => void
}

const typeIcon = {
  friends: Heart,
  groups: Users2,
  custom: FolderOpen,
}

export function CollectionCard({ collection, isActive, onClick, onRename, onDelete }: CollectionCardProps) {
  const Icon = typeIcon[collection.type] ?? FolderOpen
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null)
  const hasMenu = Boolean(onRename || onDelete)

  return (
    <div className="relative group">
      <button
        type="button"
        onClick={onClick}
        className="w-full flex items-center gap-3 px-3 py-2.5 rounded-[16px] text-left transition-all duration-150"
        style={{
          background: isActive
            ? 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))'
            : 'transparent',
          border: isActive
            ? '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)'
            : '1px solid transparent',
        }}
      >
        <div
          className="flex items-center justify-center rounded-full shrink-0"
          style={{
            width: 32,
            height: 32,
            background: 'color-mix(in srgb, var(--cp-accent-soft) 18%, var(--cp-surface))',
            color: 'var(--cp-accent)',
          }}
        >
          <Icon size={16} />
        </div>

        <div className="flex-1 min-w-0">
          <div
            className="truncate text-sm font-medium"
            style={{ color: 'var(--cp-text)' }}
          >
            {collection.name}
          </div>
        </div>

        <span
          className="shrink-0 text-[11px] font-medium tabular-nums"
          style={{ color: 'var(--cp-muted)' }}
        >
          {collection.entityIds.length}
        </span>
      </button>

      {hasMenu && (
        <>
          <IconButton
            size="small"
            className="opacity-0 group-hover:opacity-100 transition-opacity"
            onClick={(e) => {
              e.stopPropagation()
              setAnchorEl(e.currentTarget)
            }}
            sx={{
              position: 'absolute',
              right: 4,
              top: '50%',
              transform: 'translateY(-50%)',
              width: 24,
              height: 24,
            }}
          >
            <MoreHorizontal size={14} />
          </IconButton>
          <Menu
            anchorEl={anchorEl}
            open={Boolean(anchorEl)}
            onClose={() => setAnchorEl(null)}
            slotProps={{ paper: { sx: { minWidth: 140 } } }}
          >
            {onRename && (
              <MenuItem
                onClick={() => {
                  setAnchorEl(null)
                  onRename()
                }}
              >
                <ListItemIcon><Pencil size={14} /></ListItemIcon>
                <ListItemText>Rename</ListItemText>
              </MenuItem>
            )}
            {onDelete && (
              <MenuItem
                onClick={() => {
                  setAnchorEl(null)
                  onDelete()
                }}
              >
                <ListItemIcon><Trash2 size={14} /></ListItemIcon>
                <ListItemText>Delete</ListItemText>
              </MenuItem>
            )}
          </Menu>
        </>
      )}
    </div>
  )
}
