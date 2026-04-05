/* ── Contact detail page ── */

import { Chip, Button } from '@mui/material'
import { UserCheck, UserX, Trash2, FolderPlus } from 'lucide-react'
import type { ContactEntity } from '../../mock/types'
import { HeaderSection } from '../sections/HeaderSection'
import { BindingsSection } from '../sections/BindingsSection'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface ContactDetailPageProps {
  contact: ContactEntity
  onRemoved?: () => void
}

export function ContactDetailPage({ contact, onRemoved }: ContactDetailPageProps) {
  const store = useUsersAgentsStore()

  const handleRemove = () => {
    if (window.confirm(`Remove contact "${contact.displayName}"?`)) {
      store.removeContact(contact.id)
      onRemoved?.()
    }
  }

  return (
    <div className="space-y-4">
      <HeaderSection
        name={contact.displayName}
        kind="contact"
        avatarUrl={contact.avatarUrl}
        did={contact.did}
        subtitle={contact.sourceLabel ?? `Source: ${contact.source}`}
        badges={
          <>
            <Chip
              icon={contact.isVerified ? <UserCheck size={12} /> : <UserX size={12} />}
              label={contact.isVerified ? 'Verified (mutual)' : 'Unverified'}
              size="small"
              color={contact.isVerified ? 'success' : 'default'}
              variant="outlined"
            />
            {contact.tags.map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </>
        }
      />

      <BindingsSection bindings={contact.bindings} />

      {/* Source & history */}
      <div
        className="rounded-[22px] px-5 py-4"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <h3
          className="font-display text-sm font-semibold mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          Source & History
        </h3>
        <div className="space-y-1.5">
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Import source
            </span>
            <span className="text-sm capitalize" style={{ color: 'var(--cp-text)' }}>
              {contact.source}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Created
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(contact.createdAt).toLocaleDateString()}
            </span>
          </div>
          {contact.lastInteraction && (
            <div className="flex items-baseline gap-3">
              <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
                Last interaction
              </span>
              <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
                {new Date(contact.lastInteraction).toLocaleString()}
              </span>
            </div>
          )}
        </div>
      </div>

      {/* Notes */}
      {contact.notes && (
        <div
          className="rounded-[22px] px-5 py-4"
          style={{
            background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
          }}
        >
          <h3
            className="font-display text-sm font-semibold mb-2"
            style={{ color: 'var(--cp-text)' }}
          >
            Notes
          </h3>
          <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
            {contact.notes}
          </p>
        </div>
      )}

      {/* Actions */}
      <div
        className="rounded-[22px] px-5 py-4"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <h3
          className="font-display text-sm font-semibold mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          Actions
        </h3>
        <div className="flex flex-wrap gap-2">
          <Button size="small" variant="outlined" startIcon={<FolderPlus size={14} />}>
            Add to Collection
          </Button>
          <Button size="small" color="error" variant="outlined" startIcon={<Trash2 size={14} />} onClick={handleRemove}>
            Remove
          </Button>
        </div>
      </div>
    </div>
  )
}
