/* ── Exclusive invites (admins): pre-created Limited accounts with a password-free link (SSO-06) ──
 *
 * A link is shown once, right after create or "new link"; the issuer sends it through the bound
 * channel. A visitor's resend request only shows here as a badge, never with a link.
 */

import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { Alert, Button, Chip, CircularProgress, MenuItem, TextField } from '@mui/material'
import { KeyRound, Plus, RefreshCw, RotateCcw, Ban } from 'lucide-react'
import { accessDenialOf } from 'buckyos'
import {
  createExclusiveInvite,
  listExclusiveInvites,
  retryExclusiveInviteResource,
  revokeExclusiveInvite,
  rotateExclusiveInvite,
  type ExclusiveInvite,
  type ExclusiveInviteCreateInput,
  type InviteResource,
} from '../../../../api/sso'
import { useI18n } from '../../../../i18n/provider'
import { denialAdjustText, denialText } from '../../../../sso/denial'
import { INVITE_CHANNEL_KINDS, maskChannelTarget } from '../../../../sso/channel'
import { OneTimeLink } from '../../../../sso/OneTimeLink'
import { errorMessage } from '../../../agent-setup/model'
import { useLocalUsers } from '../../hooks/use-users-agents-store'

const USER_ID_PATTERN = /^[A-Za-z0-9_.-]{1,64}$/

const panelStyle = {
  background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
  border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
}

const itemStyle = {
  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
}

type Translate = ReturnType<typeof useI18n>['t']

/** A failure as admins read it: the denial and where it is adjusted, or the error itself. */
function failureText(t: Translate, error: unknown): string {
  const denial = accessDenialOf(error)
  if (!denial) return t('sso.invites.failed', 'The change failed: {{reason}}', { reason: errorMessage(error) || '—' })
  const adjust = denialAdjustText(t, denial)
  return adjust ? `${denialText(t, denial)} ${adjust}` : denialText(t, denial)
}

function Panel({ title, children, testId, actions }: { title: string; children: ReactNode; testId: string; actions?: ReactNode }) {
  return (
    <section className="rounded-[22px] px-5 py-4" style={panelStyle} data-testid={testId}>
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{title}</h3>
        {actions}
      </div>
      {children}
    </section>
  )
}

const formatTime = (secs: number) => new Date(secs * 1000).toLocaleString()

type ResourceKind = 'none' | InviteResource['kind']

function CreateInviteForm({ limitedAccounts, onCreated }: { limitedAccounts: string[]; onCreated: (link: string) => void }) {
  const { t } = useI18n()
  const [mode, setMode] = useState<'new' | 'existing'>('new')
  const [userId, setUserId] = useState('')
  const [showName, setShowName] = useState('')
  const [existingId, setExistingId] = useState('')
  const [channelKind, setChannelKind] = useState<string>('email')
  const [address, setAddress] = useState('')
  const [resourceKind, setResourceKind] = useState<ResourceKind>('none')
  const [groupDid, setGroupDid] = useState('')
  const [sessionId, setSessionId] = useState('')
  const [workspaceId, setWorkspaceId] = useState('')
  const [resourceTitle, setResourceTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const hint = maskChannelTarget(channelKind, address)
  const id = userId.trim()
  const idInvalid = mode === 'new' && id.length > 0 && !USER_ID_PATTERN.test(id)

  const resource = (): InviteResource | undefined | null => {
    const title = resourceTitle.trim()
    if (resourceKind === 'group_session') {
      return groupDid.trim() && sessionId.trim() ? { kind: 'group_session', group_did: groupDid.trim(), session_id: sessionId.trim(), title } : null
    }
    if (resourceKind === 'workspace') return workspaceId.trim() ? { kind: 'workspace', workspace_id: workspaceId.trim(), title } : null
    return undefined
  }

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const account: ExclusiveInviteCreateInput['account'] | null = mode === 'new'
      ? (id && !idInvalid ? { kind: 'new', userId: id, showName: showName.trim() || undefined } : null)
      : (existingId ? { kind: 'existing', userId: existingId } : null)
    const target = resource()
    if (!account || target === null) {
      setError(t('sso.invites.incomplete', 'Fill in the account and, for a resource, its identifiers.'))
      return
    }
    setBusy(true)
    setError(null)
    const { data, error: failure } = await createExclusiveInvite({ account, channel: { kind: channelKind, target_hint: hint }, ...(target ? { resource: target } : {}) })
    setBusy(false)
    if (!data) {
      setError(failureText(t, failure))
      return
    }
    setUserId('')
    setShowName('')
    setAddress('')
    setResourceTitle('')
    onCreated(data.link)
  }

  return (
    <form className="space-y-3" onSubmit={(event) => void submit(event)} noValidate data-testid="exclusive-invite-create">
      <div className="flex flex-wrap gap-2">
        <Chip label={t('sso.invites.newAccount', 'New customer account')} variant={mode === 'new' ? 'filled' : 'outlined'} onClick={() => setMode('new')} data-testid="exclusive-invite-mode-new" />
        <Chip
          label={t('sso.invites.existingAccount', 'Reuse a restricted account')}
          variant={mode === 'existing' ? 'filled' : 'outlined'}
          onClick={() => setMode('existing')}
          disabled={limitedAccounts.length === 0}
          data-testid="exclusive-invite-mode-existing"
        />
      </div>
      {mode === 'new' ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            size="small"
            label={t('sso.invites.userId', 'Account ID')}
            value={userId}
            disabled={busy}
            error={idInvalid}
            helperText={idInvalid ? t('sso.register.userIdInvalid', 'Use 1-64 letters, digits, _, - or . (root, system, admin and guest are reserved).') : t('sso.invites.userIdHint', 'A restricted account without a password; it cannot change later.')}
            onChange={(event) => setUserId(event.target.value)}
          />
          <TextField
            size="small"
            label={t('sso.invites.showName', 'Display name')}
            value={showName}
            disabled={busy}
            onChange={(event) => setShowName(event.target.value)}
          />
        </div>
      ) : (
        <TextField
          select
          fullWidth
          size="small"
          label={t('sso.invites.existingSelect', 'Restricted account')}
          value={existingId}
          disabled={busy}
          onChange={(event) => setExistingId(event.target.value)}
          slotProps={{ htmlInput: { 'data-testid': 'exclusive-invite-existing' } }}
        >
          {limitedAccounts.map((account) => <MenuItem key={account} value={account}>{account}</MenuItem>)}
        </TextField>
      )}
      <div className="grid gap-3 sm:grid-cols-[10rem_1fr]">
        <TextField select size="small" label={t('sso.invites.channel', 'Channel')} value={channelKind} disabled={busy} onChange={(event) => setChannelKind(event.target.value)}>
          {INVITE_CHANNEL_KINDS.map((kind) => <MenuItem key={kind} value={kind}>{t(`sso.invites.channel.${kind}`, kind)}</MenuItem>)}
        </TextField>
        <TextField
          size="small"
          label={t('sso.invites.address', 'Address you will send to')}
          value={address}
          disabled={busy}
          helperText={hint ? t('sso.invites.addressMasked', 'Only "{{hint}}" is stored.', { hint }) : t('sso.invites.addressHint', 'Only a masked hint is stored; you send the link yourself.')}
          onChange={(event) => setAddress(event.target.value)}
          slotProps={{ htmlInput: { 'data-testid': 'exclusive-invite-address' } }}
        />
      </div>
      <TextField select fullWidth size="small" label={t('sso.invites.resource', 'Resource (optional)')} value={resourceKind} disabled={busy} onChange={(event) => setResourceKind(event.target.value as ResourceKind)}>
        <MenuItem value="none">{t('sso.invites.resource.none', 'None')}</MenuItem>
        <MenuItem value="group_session">{t('sso.invites.resource.groupSession', 'Message Hub session')}</MenuItem>
        <MenuItem value="workspace">{t('sso.invites.resource.workspace', 'AI Workspace')}</MenuItem>
      </TextField>
      {resourceKind === 'group_session' ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField size="small" label={t('sso.invites.groupDid', 'Group DID')} value={groupDid} disabled={busy} onChange={(event) => setGroupDid(event.target.value)} />
          <TextField size="small" label={t('sso.invites.sessionId', 'Session ID')} value={sessionId} disabled={busy} onChange={(event) => setSessionId(event.target.value)} />
        </div>
      ) : null}
      {resourceKind === 'workspace' ? (
        <TextField fullWidth size="small" label={t('sso.invites.workspaceId', 'Workspace ID')} value={workspaceId} disabled={busy} onChange={(event) => setWorkspaceId(event.target.value)} />
      ) : null}
      {resourceKind !== 'none' ? (
        <TextField fullWidth size="small" label={t('sso.invites.resourceTitle', 'Title the customer sees')} value={resourceTitle} disabled={busy} onChange={(event) => setResourceTitle(event.target.value)} />
      ) : null}
      {error ? <Alert severity="error" data-testid="exclusive-invite-create-error">{error}</Alert> : null}
      <Button type="submit" size="small" variant="contained" startIcon={busy ? <CircularProgress size={14} color="inherit" /> : <Plus size={14} />} disabled={busy} data-testid="exclusive-invite-submit">
        {t('sso.invites.create', 'Create invitation')}
      </Button>
    </form>
  )
}

function InviteRow({
  invite,
  now,
  busy,
  onRotate,
  onRevoke,
  onRetry,
}: {
  invite: ExclusiveInvite
  /** Unix seconds the list was read at. */
  now: number
  busy: boolean
  onRotate: () => void
  onRevoke: () => void
  onRetry: () => void
}) {
  const { t } = useI18n()
  const revoked = invite.state === 'revoked'
  const passwordSet = invite.credential.state === 'consumed_by_password'
  const expired = invite.credential.state === 'active' && now > invite.credential.expires_at
  const status = revoked
    ? t('sso.invites.state.revoked', 'Revoked')
    : passwordSet
      ? t('sso.invites.state.passwordSet', 'Password set; the link is only an entry')
      : invite.credential.state === 'superseded'
        ? t('sso.invites.state.superseded', 'Replaced')
        : expired
          ? t('sso.invites.state.expired', 'Link expired')
          : t('sso.invites.state.active', 'Link valid until {{time}}', { time: formatTime(invite.credential.expires_at) })

  return (
    <li className="rounded-[16px] px-3 py-2.5" style={itemStyle} data-testid="exclusive-invite-row" data-invite-id={invite.invite_id}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{invite.user_id}</span>
            {invite.resource?.title ? <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>· {invite.resource.title}</span> : null}
          </div>
          <div className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t(`sso.invites.channel.${invite.channel.kind}`, invite.channel.kind)}{invite.channel.target_hint ? ` · ${invite.channel.target_hint}` : ''}
            {' · '}
            <span data-testid="exclusive-invite-status">{status}</span>
          </div>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {invite.resend?.requested_at && !revoked && !passwordSet ? (
              <Chip
                size="small"
                color="warning"
                label={t('sso.invites.resendRequested', 'Customer asked for a new link · {{time}}', { time: formatTime(invite.resend.requested_at) })}
                data-testid="exclusive-invite-resend-badge"
              />
            ) : null}
            {invite.resource_state === 'pending' ? (
              <Chip size="small" color="error" variant="outlined" label={t('sso.invites.resourcePending', 'Session invitation not completed')} data-testid="exclusive-invite-resource-pending" />
            ) : null}
          </div>
        </div>
        <div className="flex flex-wrap gap-1">
          {invite.resource_state === 'pending' && !revoked ? (
            <Button size="small" variant="text" startIcon={<RefreshCw size={14} />} disabled={busy} onClick={onRetry} data-testid="exclusive-invite-retry">
              {t('sso.invites.retry', 'Retry')}
            </Button>
          ) : null}
          <Button size="small" variant="text" startIcon={<RotateCcw size={14} />} disabled={busy || revoked || passwordSet} onClick={onRotate} data-testid="exclusive-invite-rotate">
            {t('sso.invites.rotate', 'New link')}
          </Button>
          <Button size="small" variant="text" color="error" startIcon={<Ban size={14} />} disabled={busy || revoked} onClick={onRevoke} data-testid="exclusive-invite-revoke">
            {t('sso.invites.revoke', 'Revoke')}
          </Button>
        </div>
      </div>
    </li>
  )
}

export function ExclusiveInvitesPage() {
  const { t } = useI18n()
  const localUsers = useLocalUsers()
  const [invites, setInvites] = useState<ExclusiveInvite[] | null>(null)
  const [readAt, setReadAt] = useState(0)
  const [loadFailed, setLoadFailed] = useState<string | null>(null)
  const [link, setLink] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [seq, setSeq] = useState(0)

  useEffect(() => {
    let live = true
    void listExclusiveInvites().then(({ data, error }) => {
      if (!live) return
      if (data) {
        setInvites(data.invites)
        setReadAt(Math.floor(Date.now() / 1000))
        setLoadFailed(null)
      } else {
        setLoadFailed(failureText(t, error))
      }
    })
    return () => { live = false }
  }, [seq, t])

  const reload = () => setSeq((value) => value + 1)

  const limitedAccounts = useMemo(() => {
    const ids = new Set<string>(localUsers.filter((user) => user.role === 'limited').map((user) => user.id))
    for (const invite of invites ?? []) ids.add(invite.user_id)
    return [...ids].sort()
  }, [invites, localUsers])

  const act = async (invite: ExclusiveInvite, action: 'rotate' | 'revoke' | 'retry') => {
    if (action === 'rotate' && !window.confirm(t('sso.invites.rotateConfirm', 'Make a new link for {{user}}? The current link stops working.', { user: invite.user_id }))) return
    if (action === 'revoke' && !window.confirm(t('sso.invites.revokeConfirm', 'Revoke the invitation of {{user}}? Its link stops working and sessions it opened end within 15 minutes.', { user: invite.user_id }))) return
    setBusyId(invite.invite_id)
    setActionError(null)
    const result = action === 'rotate'
      ? await rotateExclusiveInvite(invite.invite_id)
      : action === 'revoke'
        ? await revokeExclusiveInvite(invite.invite_id)
        : await retryExclusiveInviteResource(invite.invite_id)
    setBusyId(null)
    if (!result.data) {
      setActionError(failureText(t, result.error))
      return
    }
    if (action === 'rotate' && 'link' in result.data && typeof result.data.link === 'string') setLink(result.data.link)
    reload()
  }

  return (
    <div className="space-y-4" data-testid="exclusive-invites-page">
      <div className="flex items-center gap-3">
        <span className="flex h-10 w-10 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))', color: 'var(--cp-accent)' }}>
          <KeyRound size={18} />
        </span>
        <div>
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{t('sso.invites.title', 'Exclusive invites')}</h2>
          <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {t('sso.invites.intro', 'Create a restricted account for a customer and a link that signs them in without a password for 48 hours. You send the link yourself; the system never sends mail or SMS.')}
          </p>
        </div>
      </div>

      {link ? <OneTimeLink link={link} onDismiss={() => setLink(null)} testId="exclusive-invite-link" /> : null}

      <Panel title={t('sso.invites.createTitle', 'New invitation')} testId="exclusive-invite-create-panel">
        <CreateInviteForm limitedAccounts={limitedAccounts} onCreated={(created) => { setLink(created); reload() }} />
      </Panel>

      <Panel
        title={t('sso.invites.listTitle', 'Invitations ({{count}})', { count: invites?.length ?? 0 })}
        testId="exclusive-invite-list"
        actions={<Button size="small" variant="text" startIcon={<RefreshCw size={14} />} onClick={reload}>{t('sso.common.refresh', 'Refresh')}</Button>}
      >
        {actionError ? <Alert severity="error" sx={{ mb: 1.5 }} data-testid="exclusive-invite-action-error">{actionError}</Alert> : null}
        {loadFailed ? (
          <Alert severity="error">{loadFailed}</Alert>
        ) : !invites ? (
          <div className="flex justify-center py-4"><CircularProgress size={22} /></div>
        ) : invites.length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('sso.invites.empty', 'No invitations yet.')}</p>
        ) : (
          <ul className="space-y-2">
            {invites.map((invite) => (
              <InviteRow
                key={invite.invite_id}
                invite={invite}
                now={readAt}
                busy={busyId !== null}
                onRotate={() => void act(invite, 'rotate')}
                onRevoke={() => void act(invite, 'revoke')}
                onRetry={() => void act(invite, 'retry')}
              />
            ))}
          </ul>
        )}
      </Panel>
    </div>
  )
}
