/* ── Social account management section ── */

import { useState } from 'react'
import { AlertCircle, Check, Clock, Eye, EyeOff, Plus, Trash2 } from 'lucide-react'
import { Alert, Button, CircularProgress, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, Switch, TextField } from '@mui/material'
import type { SocialAccount } from '../../datamodel/types'
import { socialAccountPlatformOptions } from '../../datamodel/types'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { useI18n } from '../../../../i18n/provider'
import { errorMessage, isTelegramAccountId } from '../../../agent-setup/model'

interface SocialAccountsSectionProps {
  entityId?: string
  accounts: SocialAccount[]
  editable?: boolean
  /** The signed-in user's own page: Telegram is stored by the control panel as the Owner identity. */
  ownProfile?: boolean
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

function createSocialAccountId(platform: string) {
  const suffix = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  return `social-${platform}-${suffix}`
}

export function SocialAccountsSection({ entityId, accounts, editable = true, ownProfile = false }: SocialAccountsSectionProps) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [telegramForm, setTelegramForm] = useState(false)
  const [telegramId, setTelegramId] = useState('')
  const [saving, setSaving] = useState(false)
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [removing, setRemoving] = useState<string | null>(null)
  const [removeError, setRemoveError] = useState<string | null>(null)
  const store = useUsersAgentsStore()

  const closeDialog = () => {
    if (saving) return
    setOpen(false)
    setTelegramForm(false)
    setTelegramId('')
    setDialogError(null)
  }

  const handleAdd = (platform: string) => {
    if (!entityId) return
    if (ownProfile && platform === 'telegram') {
      setTelegramForm(true)
      return
    }
    const accountId = platform === 'phone' ? '+1-555-0123' : 'new@example.com'
    store.addSocialAccount(entityId, {
      id: createSocialAccountId(platform),
      platform,
      accountId,
      displayId: accountId,
      status: 'pending',
      isPublic: false,
      canIdentify: true,
    })
    closeDialog()
  }

  const saveTelegram = async () => {
    if (!isTelegramAccountId(telegramId) || saving) return
    setSaving(true)
    setDialogError(null)
    const error = await store.addOwnTelegram(telegramId)
    setSaving(false)
    if (error) {
      setDialogError(t('usersAgents.social.saveFailed', undefined, { detail: errorMessage(error) }))
      return
    }
    setOpen(false)
    setTelegramForm(false)
    setTelegramId('')
  }

  const handleRemove = async (account: SocialAccount) => {
    if (!entityId) return
    if (!(ownProfile && account.platform === 'telegram')) {
      store.removeSocialAccount(entityId, account.id)
      return
    }
    setRemoving(account.id)
    setRemoveError(null)
    const error = await store.removeOwnTelegram()
    setRemoving(null)
    if (error) setRemoveError(t('usersAgents.social.removeFailed', undefined, { detail: errorMessage(error) }))
  }

  const telegramIdInvalid = telegramId.trim().length > 0 && !isTelegramAccountId(telegramId)

  return (
    <div
      className="rounded-[22px] px-5 py-4"
      data-testid="social-accounts"
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
            {t('usersAgents.social.title')}
          </h3>
        </div>
        {editable && (
          <Button
            size="small"
            startIcon={<Plus size={14} />}
            variant="text"
            onClick={() => setOpen(true)}
          >
            {t('usersAgents.social.add')}
          </Button>
        )}
      </div>

      {removeError ? <Alert severity="error" className="mb-3">{removeError}</Alert> : null}

      {accounts.length === 0 ? (
        <div className="text-sm py-3" style={{ color: 'var(--cp-muted)' }}>
          {t('usersAgents.social.empty')}
        </div>
      ) : (
        <div className="space-y-2">
          {accounts.map((account) => {
            const StatusIcon = statusIcon[account.status]
            const color = statusColor[account.status]
            return (
              <div
                key={account.id}
                data-platform={account.platform}
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
                    {account.isPublic ? t('usersAgents.social.public') : t('usersAgents.social.private')}
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
                        slotProps={{ input: { 'aria-label': t('usersAgents.social.toggleVisibility', undefined, { platform: account.platform }) } }}
                        onChange={() => store.toggleSocialAccountVisibility(entityId, account.id)}
                      />
                      <IconButton
                        size="small"
                        aria-label={t('usersAgents.social.remove', undefined, { platform: account.platform })}
                        disabled={removing === account.id}
                        onClick={() => void handleRemove(account)}
                      >
                        {removing === account.id ? <CircularProgress size={12} color="inherit" /> : <Trash2 size={12} />}
                      </IconButton>
                    </>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}

      <Dialog open={open} onClose={closeDialog} fullWidth maxWidth="xs">
        <DialogTitle>{telegramForm ? t('usersAgents.social.telegramTitle') : t('usersAgents.social.addTitle')}</DialogTitle>
        <DialogContent>
          {telegramForm ? (
            <div className="space-y-3 pt-1">
              <p className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.social.telegramBody')}</p>
              <TextField
                label={t('usersAgents.social.telegramId')}
                value={telegramId}
                autoFocus
                autoComplete="off"
                inputProps={{ inputMode: 'numeric', 'data-testid': 'social-telegram-id' }}
                error={telegramIdInvalid}
                helperText={telegramIdInvalid ? t('usersAgents.social.telegramIdInvalid') : t('usersAgents.social.telegramIdHint')}
                onChange={(event) => setTelegramId(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') void saveTelegram()
                }}
              />
              {dialogError ? <Alert severity="error">{dialogError}</Alert> : null}
            </div>
          ) : (
            <>
              <div className="pb-3 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
                {t('usersAgents.social.addBody')}
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
                      {t(`usersAgents.social.hint.${option.id}`)}
                    </div>
                  </button>
                ))}
              </div>
            </>
          )}
        </DialogContent>
        <DialogActions>
          <Button variant="text" disabled={saving} onClick={closeDialog}>{t('common.cancel')}</Button>
          {telegramForm ? (
            <Button
              disabled={saving || !isTelegramAccountId(telegramId)}
              startIcon={saving ? <CircularProgress size={13} color="inherit" /> : undefined}
              onClick={() => void saveTelegram()}
            >
              {t('common.save')}
            </Button>
          ) : null}
        </DialogActions>
      </Dialog>
    </div>
  )
}
