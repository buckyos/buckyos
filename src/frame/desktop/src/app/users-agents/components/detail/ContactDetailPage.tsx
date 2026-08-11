/* ── Contact detail page ── */

import { useState } from 'react'
import { Alert, Chip, Button } from '@mui/material'
import { UserCheck, UserX, Trash2, FolderPlus, MessageSquare, ShieldCheck } from 'lucide-react'
import type { ContactEntity } from '../../../my-network/datamodel/types'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { useCollections, useMyNetworkStore } from '../../../my-network/hooks/use-my-network-store'

interface ContactDetailPageProps {
  contact: ContactEntity
  onRemoved?: () => void
}

export function ContactDetailPage({ contact, onRemoved }: ContactDetailPageProps) {
  const [showCommentFlow, setShowCommentFlow] = useState(false)
  const store = useMyNetworkStore()
  const collections = useCollections()
  const memberships = collections.filter((collection) =>
    collection.entityIds.includes(contact.id),
  )

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
              label={contact.relation === 'mutual' ? 'Mutual DID relation' : 'One-way relation'}
              size="small"
              color={contact.isVerified ? 'success' : 'default'}
              variant="outlined"
            />
            {contact.isMergeCandidate && (
              <Chip label="Merge candidate" size="small" color="warning" variant="outlined" />
            )}
            {contact.tags.map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </>
        }
      />

      <SocialAccountsSection entityId={contact.id} accounts={contact.socialAccounts} editable={false} />

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
          {contact.importBatch && (
            <div className="flex items-baseline gap-3">
              <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
                Import batch
              </span>
              <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
                {contact.importBatch}
              </span>
            </div>
          )}
          {contact.lastSyncedAt && (
            <div className="flex items-baseline gap-3">
              <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
                Last synced
              </span>
              <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
                {new Date(contact.lastSyncedAt).toLocaleString()}
              </span>
            </div>
          )}
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
          Collections
        </h3>
        <div className="flex flex-wrap gap-1.5">
          {memberships.map((collection) => (
            <Chip
              key={collection.id}
              label={collection.name}
              size="small"
              variant="outlined"
            />
          ))}
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
          <Button
            size="small"
            variant="outlined"
            startIcon={<MessageSquare size={14} />}
            onClick={() => setShowCommentFlow((value) => !value)}
          >
            Comment Login Flow
          </Button>
          <Button size="small" color="error" variant="outlined" startIcon={<Trash2 size={14} />} onClick={handleRemove}>
            Remove
          </Button>
        </div>
      </div>

      {showCommentFlow && (
        <div
          className="rounded-[22px] px-5 py-4"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent-soft) 10%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-accent) 20%, transparent)',
          }}
        >
          <div className="mb-3 flex items-center gap-2">
            <ShieldCheck size={16} style={{ color: 'var(--cp-accent)' }} />
            <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              HomeStation Comment Authorization
            </h3>
          </div>
          <div className="space-y-2">
            {[
              ['Guest read', 'Public content can be read with Session Token = None.'],
              ['DID proof', `${contact.displayName} proves a DID / BNS identity with a wallet, passkey, or their own Zone.`],
              ['Scoped token', 'Verify Hub issues a token scoped to this Zone, HomeStation, and the comment action.'],
              ['App policy', 'HomeStation checks contact relation, moderation, block list, and audit rules before publishing.'],
            ].map(([title, body]) => (
              <div key={title} className="rounded-[14px] px-3 py-2" style={{
                background: 'color-mix(in srgb, var(--cp-surface) 78%, transparent)',
                border: '1px solid color-mix(in srgb, var(--cp-border) 36%, transparent)',
              }}>
                <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{title}</div>
                <div className="mt-0.5 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{body}</div>
              </div>
            ))}
          </div>
          <Alert severity="info" sx={{ mt: 2 }}>
            Writing a comment is not the same as creating a system user.
            Anonymous write access remains an app-level policy exception.
          </Alert>
        </div>
      )}
    </div>
  )
}
