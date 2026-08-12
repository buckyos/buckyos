/* ── Social account management section ── */

import { useState } from 'react'
import { AlertCircle, Check, Clock, Eye, EyeOff, Plus, Trash2 } from 'lucide-react'
import { Button, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, Switch } from '@mui/material'
import type { SocialAccount } from '../../datamodel/types'
import { socialAccountPlatformOptions } from '../../datamodel/types'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface SocialAccountsSectionProps {
  entityId?: string
  accounts: SocialAccount[]
  editable?: boolean
}

const statusIcon = {
  active: Check,
  pending: Clock,
  error: AlertCircle,
}

const statusColor = {
  active: 'var(--cp-success)',
  pending: 'var(--cp-warning)',
  error: 'var(--cp-danger)',
}

export function SocialAccountsSection({ entityId, accounts, editable = true }: SocialAccountsSectionProps) {
  const [open, setOpen] = useState(false)
  const store = useUsersAgentsStore()

  const handleAdd = (platform: string) => {
    if (!entityId) return
    const accountId = platform === 'telegram' ? '@new_channel' : platform === 'phone' ? '+1-555-0123' : 'new@example.com'
    store.addSocialAccount(entityId, {
      id: `social-${platform}-${accounts.length + 1}`,
      platform,
      accountId,
      displayId: accountId,
      status: 'pending',
      isPublic: false,
      canIdentify: true,
    })
    setOpen(false)
  }

  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <Eye size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            Social Accounts
          </h3>
        </div>
        {editable && (
          <Button
            size="small"
            startIcon={<Plus size={14} />}
            variant="text"
            onClick={() => setOpen(true)}
          >
            Add
          </Button>
        )}
      </div>

      {accounts.length === 0 ? (
        <div className="text-sm py-3" style={{ color: 'var(--cp-muted)' }}>
          No social accounts configured. Add an account to complete this DID profile.
        </div>
      ) : (
        <div className="space-y-2">
          {accounts.map((account) => {
            const StatusIcon = statusIcon[account.status]
            const color = statusColor[account.status]
            return (
              <div
                key={account.id}
                className="flex flex-col gap-2 px-3 py-2.5 rounded-[14px] sm:flex-row sm:items-center"
                style={{
                  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
                }}
              >
                <div
                  className="shrink-0 flex items-center justify-center rounded-full"
                  style={{
                    width: 28,
                    height: 28,
                    background: `color-mix(in srgb, ${color} 14%, var(--cp-surface))`,
                    color,
                  }}
                >
                  <StatusIcon size={14} />
                </div>

                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium capitalize" style={{ color: 'var(--cp-text)' }}>
                    {account.platform}
                  </div>
                  <div className="text-[11px] truncate" style={{ color: 'var(--cp-muted)' }}>
                    {account.displayId}
                  </div>
                </div>

                <div className="flex items-center justify-between gap-2 sm:justify-end">
                  <span
                    className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold"
                    style={{
                      background: account.isPublic
                        ? 'color-mix(in srgb, var(--cp-success) 14%, transparent)'
                        : 'color-mix(in srgb, var(--cp-muted) 14%, transparent)',
                      color: account.isPublic ? 'var(--cp-success)' : 'var(--cp-muted)',
                    }}
                  >
                    {account.isPublic ? <Eye size={11} /> : <EyeOff size={11} />}
                    {account.isPublic ? 'Public' : 'Private'}
                  </span>

                  {account.lastSyncAt && (
                    <span className="text-[10px] shrink-0" style={{ color: 'var(--cp-muted)' }}>
                      {new Date(account.lastSyncAt).toLocaleDateString()}
                    </span>
                  )}

                  {editable && entityId && (
                    <>
                      <Switch
                        checked={account.isPublic}
                        size="small"
                        inputProps={{ 'aria-label': `Toggle ${account.platform} public visibility` }}
                        onChange={() => store.toggleSocialAccountVisibility(entityId, account.id)}
                      />
                      <IconButton
                        size="small"
                        aria-label={`Remove ${account.platform}`}
                        onClick={() => store.removeSocialAccount(entityId, account.id)}
                      >
                        <Trash2 size={12} />
                      </IconButton>
                    </>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}

      <Dialog open={open} onClose={() => setOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>Add social account</DialogTitle>
        <DialogContent>
          <div className="pb-3 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
            Add accounts you use on other platforms to this DID profile. You can choose which accounts are public and which are only used for identity recognition.
          </div>
          <div className="space-y-2 pt-1">
            {socialAccountPlatformOptions.map((option) => (
              <button
                key={option.id}
                type="button"
                className="w-full rounded-[14px] px-3 py-2 text-left"
                style={{
                  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
                }}
                onClick={() => handleAdd(option.id)}
              >
                <div className="text-sm font-medium capitalize" style={{ color: 'var(--cp-text)' }}>
                  {option.label}
                </div>
                <div className="mt-0.5 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>
                  {option.hint}
                </div>
              </button>
            ))}
          </div>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setOpen(false)}>Cancel</Button>
        </DialogActions>
      </Dialog>
    </div>
  )
}
