/* ── New local user wizard (inline multi-step form) ── */

import { useState } from 'react'
import {
  Button,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  IconButton,
} from '@mui/material'
import { X, ChevronRight, ChevronLeft, UserPlus } from 'lucide-react'
import type { NewUserDraft, LocalUserEntity } from '../../mock/types'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface NewUserWizardProps {
  onClose: () => void
  onCreated?: (userId: string) => void
}

const QUOTA_OPTIONS = ['1 GB', '5 GB', '10 GB', '50 GB', '100 GB']

const INITIAL_DRAFT: NewUserDraft = {
  step: 0,
  displayName: '',
  role: 'member',
  initialPassword: '',
  storageQuota: '10 GB',
}

export function NewUserWizard({ onClose, onCreated }: NewUserWizardProps) {
  const [draft, setDraft] = useState<NewUserDraft>(INITIAL_DRAFT)
  const store = useUsersAgentsStore()

  const update = (patch: Partial<NewUserDraft>) =>
    setDraft((d) => ({ ...d, ...patch }))

  const canNext = () => {
    if (draft.step === 0) return draft.displayName.trim().length >= 2
    if (draft.step === 1) return draft.initialPassword.length >= 4
    return true
  }

  const handleCreate = () => {
    const id = `user-${draft.displayName.toLowerCase().replace(/\s+/g, '-')}-${Date.now()}`
    const now = new Date().toISOString()
    const user: LocalUserEntity = {
      id,
      kind: 'local-user',
      displayName: draft.displayName.trim(),
      role: draft.role,
      storageUsed: '0 B',
      storageQuota: draft.storageQuota,
      lastActive: now,
      isOnline: false,
      availableApps: ['Files', 'MessageHub', 'Settings'],
      defaultGroup: draft.role === 'admin' ? 'admins' : 'users',
      bindings: [],
      createdAt: now,
    }
    store.addLocalUser(user)
    onCreated?.(id)
    onClose()
  }

  const steps = ['Identity', 'Security', 'Review']

  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 60%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)',
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <UserPlus size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            New Local User
          </h3>
        </div>
        <IconButton size="small" onClick={onClose}>
          <X size={16} />
        </IconButton>
      </div>

      {/* Step indicator */}
      <div className="flex items-center gap-1 mb-4">
        {steps.map((label, i) => (
          <div key={label} className="flex items-center gap-1">
            {i > 0 && (
              <div
                className="w-6 h-[1px]"
                style={{
                  background:
                    i <= draft.step
                      ? 'var(--cp-accent)'
                      : 'color-mix(in srgb, var(--cp-border) 60%, transparent)',
                }}
              />
            )}
            <div
              className="flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[11px] font-medium"
              style={{
                background:
                  i === draft.step
                    ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                    : 'transparent',
                color:
                  i <= draft.step ? 'var(--cp-accent)' : 'var(--cp-muted)',
              }}
            >
              <span>{i + 1}</span>
              <span>{label}</span>
            </div>
          </div>
        ))}
      </div>

      {/* Step content */}
      <div className="min-h-[120px]">
        {draft.step === 0 && (
          <div className="space-y-3">
            <TextField
              label="Display Name"
              value={draft.displayName}
              onChange={(e) => update({ displayName: e.target.value })}
              size="small"
              fullWidth
              autoFocus
              placeholder="e.g. Bob"
              helperText="At least 2 characters"
            />
            <div>
              <div
                className="text-[12px] font-medium mb-1.5"
                style={{ color: 'var(--cp-muted)' }}
              >
                Role
              </div>
              <ToggleButtonGroup
                value={draft.role}
                exclusive
                onChange={(_, v) => v && update({ role: v })}
                size="small"
              >
                <ToggleButton value="admin">Admin</ToggleButton>
                <ToggleButton value="member">Member</ToggleButton>
                <ToggleButton value="guest">Guest</ToggleButton>
              </ToggleButtonGroup>
            </div>
          </div>
        )}

        {draft.step === 1 && (
          <div className="space-y-3">
            <TextField
              label="Initial Password"
              type="password"
              value={draft.initialPassword}
              onChange={(e) => update({ initialPassword: e.target.value })}
              size="small"
              fullWidth
              autoFocus
              helperText="Min 4 characters. User can change later."
            />
            <div>
              <div
                className="text-[12px] font-medium mb-1.5"
                style={{ color: 'var(--cp-muted)' }}
              >
                Storage Quota
              </div>
              <ToggleButtonGroup
                value={draft.storageQuota}
                exclusive
                onChange={(_, v) => v && update({ storageQuota: v })}
                size="small"
              >
                {QUOTA_OPTIONS.map((q) => (
                  <ToggleButton key={q} value={q}>
                    {q}
                  </ToggleButton>
                ))}
              </ToggleButtonGroup>
            </div>
          </div>
        )}

        {draft.step === 2 && (
          <div
            className="rounded-[16px] px-4 py-3 space-y-1.5"
            style={{
              background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
              border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
            }}
          >
            {[
              ['Name', draft.displayName],
              ['Role', draft.role],
              ['Storage', draft.storageQuota],
              ['Password', '••••••••'],
            ].map(([label, value]) => (
              <div key={label} className="flex items-baseline gap-3">
                <span
                  className="text-[12px] font-medium w-20 shrink-0"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  {label}
                </span>
                <span
                  className="text-sm font-medium"
                  style={{ color: 'var(--cp-text)' }}
                >
                  {value}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Footer actions */}
      <div className="flex items-center justify-between mt-4 pt-3" style={{ borderTop: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)' }}>
        <Button
          size="small"
          disabled={draft.step === 0}
          onClick={() => update({ step: draft.step - 1 })}
          startIcon={<ChevronLeft size={14} />}
        >
          Back
        </Button>

        {draft.step < 2 ? (
          <Button
            size="small"
            variant="contained"
            disabled={!canNext()}
            onClick={() => update({ step: draft.step + 1 })}
            endIcon={<ChevronRight size={14} />}
          >
            Next
          </Button>
        ) : (
          <Button
            size="small"
            variant="contained"
            onClick={handleCreate}
            startIcon={<UserPlus size={14} />}
          >
            Create User
          </Button>
        )}
      </div>
    </div>
  )
}
