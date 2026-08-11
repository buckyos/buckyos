import { zodResolver } from '@hookform/resolvers/zod'
import { useForm } from 'react-hook-form'
import {
  Alert,
  Button,
  Checkbox,
  FormControlLabel,
  IconButton,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
} from '@mui/material'
import {
  ChevronLeft,
  ChevronRight,
  Link,
  ShieldAlert,
  UserPlus,
  X,
} from 'lucide-react'
import { useState } from 'react'
import type { LocalUserEntity, NewZoneUserInput } from '../../datamodel/types'
import {
  newZoneUserInputSchema,
  storageQuotaOptions,
  userAppOptions,
} from '../../datamodel/types'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface NewUserWizardProps {
  onClose: () => void
  onCreated?: (userId: string) => void
}

const defaultValues: NewZoneUserInput = {
  source: 'primary-did',
  identifier: '',
  displayName: '',
  userType: 'user',
  storageQuota: '10 GB',
  invitationExpiresIn: '7d',
  localPassword: '',
  availableApps: ['Files', 'MessageHub', 'Settings'],
}

const stepLabels = ['Intent', 'Source', 'Identity', 'Apps', 'Review']

function slugify(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-|-$/g, '')
}

function didFromIdentifier(identifier: string, source: NewZoneUserInput['source']) {
  if (identifier.startsWith('did:')) return identifier
  if (source === 'primary-did') return `did:bns:${identifier}`
  return `did:bns:${identifier}.alice`
}

function expiryDate(value: NewZoneUserInput['invitationExpiresIn']) {
  const days = value === '24h' ? 1 : value === '7d' ? 7 : 30
  return new Date(Date.now() + days * 24 * 60 * 60 * 1000).toISOString()
}

export function NewUserWizard({ onClose, onCreated }: NewUserWizardProps) {
  const [step, setStep] = useState(0)
  const store = useUsersAgentsStore()
  const form = useForm<NewZoneUserInput>({
    resolver: zodResolver(newZoneUserInputSchema),
    defaultValues,
    mode: 'onChange',
  })

  const values = form.watch()
  const source = values.source
  const isPrimaryDid = source === 'primary-did'

  const validateStep = async () => {
    if (step === 0 || step === 1) return true
    if (step === 2) {
      return form.trigger([
        'identifier',
        'displayName',
        'userType',
        ...(isPrimaryDid ? [] : (['localPassword'] as const)),
      ])
    }
    if (step === 3) return form.trigger(['availableApps', 'storageQuota'])
    return form.trigger()
  }

  const handleNext = async () => {
    if (await validateStep()) {
      setStep((current) => Math.min(current + 1, stepLabels.length - 1))
    }
  }

  const handleCreate = form.handleSubmit((data) => {
    const now = new Date().toISOString()
    const cleanId = slugify(data.identifier || data.displayName)
    const id = `user-${cleanId}-${Date.now()}`
    const did = didFromIdentifier(data.identifier.trim(), data.source)
    const isInvite = data.source === 'primary-did'
    const user: LocalUserEntity = {
      id,
      kind: 'local-user',
      displayName: data.displayName.trim(),
      did,
      socialAccounts: [],
      role: data.source === 'local-account' ? data.userType : data.userType,
      source: data.source,
      status: isInvite ? 'pending-invitation' : 'active',
      credentialStatus: isInvite ? 'invite-pending' : 'password-set',
      canChangePassword: !isInvite && data.userType !== 'limited',
      storageUsed: '0 B',
      storageQuota: data.storageQuota,
      lastActive: now,
      isOnline: false,
      availableApps: data.availableApps,
      defaultGroup: 'zone-members',
      profile: {
        nickname: data.displayName.trim(),
        intro: isInvite
          ? 'Pending primary DID invitation.'
          : 'Local account created in this Zone.',
      },
      settings: {
        source: isInvite ? 'Primary BNS / DID' : 'Local account',
        credential: isInvite ? 'Invite pending' : 'Initial password',
        passwordChange: isInvite
          ? 'Managed outside this Zone'
          : data.userType === 'limited'
            ? 'Disabled'
            : 'Allowed',
        accountLimit: data.userType,
      },
      invitation: isInvite
        ? {
            inviteUrl: `https://alice.zone.buckyos.dev/invite/${cleanId}`,
            targetZone: 'did:zone:alice',
            requestedDid: did,
            expiresAt: expiryDate(data.invitationExpiresIn),
            bindedZoneListKey: 'binded_zone_list',
          }
        : undefined,
      createdAt: now,
    }

    store.addLocalUser(user)
    onCreated?.(id)
    onClose()
  })

  return (
    <form
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 60%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)',
      }}
      onSubmit={(event) => {
        event.preventDefault()
        void handleCreate()
      }}
    >
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <UserPlus size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            New Zone User
          </h3>
        </div>
        <IconButton size="small" onClick={onClose} aria-label="Close new user wizard">
          <X size={16} />
        </IconButton>
      </div>

      <div className="mb-4 flex flex-wrap items-center gap-1">
        {stepLabels.map((label, index) => (
          <div key={label} className="flex items-center gap-1">
            {index > 0 && (
              <div
                className="h-[1px] w-5"
                style={{
                  background:
                    index <= step
                      ? 'var(--cp-accent)'
                      : 'color-mix(in srgb, var(--cp-border) 60%, transparent)',
                }}
              />
            )}
            <div
              className="flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium"
              style={{
                background:
                  index === step
                    ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                    : 'transparent',
                color: index <= step ? 'var(--cp-accent)' : 'var(--cp-muted)',
              }}
            >
              <span>{index + 1}</span>
              <span>{label}</span>
            </div>
          </div>
        ))}
      </div>

      <div className="min-h-[280px]">
        {step === 0 && (
          <div className="space-y-3">
            <Alert severity="info">
              This creates a real user who can log in to the current Zone. Use
              My Network if you only want to add a friend or contact.
            </Alert>
            <div className="grid gap-2 sm:grid-cols-2">
              <div
                className="rounded-[16px] px-4 py-3"
                style={{
                  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
                }}
              >
                <div className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                  Primary DID invite
                </div>
                <p className="mt-1 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>
                  Recommended for users with an identity that can move across Zones.
                </p>
              </div>
              <div
                className="rounded-[16px] px-4 py-3"
                style={{
                  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
                }}
              >
                <div className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                  Local Limited account
                </div>
                <p className="mt-1 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>
                  Useful for temporary Desktop access in this Zone only.
                </p>
              </div>
            </div>
          </div>
        )}

        {step === 1 && (
          <div className="space-y-3">
            <ToggleButtonGroup
              value={source}
              exclusive
              fullWidth
              onChange={(_, value: NewZoneUserInput['source'] | null) => {
                if (!value) return
                form.setValue('source', value, { shouldDirty: true, shouldValidate: true })
                form.setValue('userType', value === 'local-account' ? 'limited' : 'user')
              }}
              size="small"
            >
              <ToggleButton value="primary-did">
                Invite primary BNS / DID
              </ToggleButton>
              <ToggleButton value="local-account">
                Create local account
              </ToggleButton>
            </ToggleButtonGroup>

            {isPrimaryDid ? (
              <Alert icon={<ShieldAlert size={18} />} severity="warning">
                The invited user must confirm the target Zone in their own
                BNS ownerconfig. Never ask for their external password here.
              </Alert>
            ) : (
              <Alert severity="warning">
                Local accounts depend on this Zone. Temporary Desktop access
                should stay Limited unless the admin explicitly changes it.
              </Alert>
            )}
          </div>
        )}

        {step === 2 && (
          <div className="space-y-3">
            <TextField
              label={isPrimaryDid ? 'BNS name or DID' : 'Local username'}
              size="small"
              fullWidth
              autoFocus
              placeholder={isPrimaryDid ? 'carol or did:bns:carol' : 'dave'}
              error={Boolean(form.formState.errors.identifier)}
              helperText={form.formState.errors.identifier?.message}
              {...form.register('identifier')}
            />
            <TextField
              label="Display name"
              size="small"
              fullWidth
              placeholder="Carol"
              error={Boolean(form.formState.errors.displayName)}
              helperText={form.formState.errors.displayName?.message}
              {...form.register('displayName')}
            />
            <div>
              <div
                className="mb-1.5 text-[12px] font-medium"
                style={{ color: 'var(--cp-muted)' }}
              >
                User type
              </div>
              <ToggleButtonGroup
                value={values.userType}
                exclusive
                onChange={(_, value: NewZoneUserInput['userType'] | null) => {
                  if (value) form.setValue('userType', value, { shouldValidate: true })
                }}
                size="small"
              >
                <ToggleButton value="admin">Admin</ToggleButton>
                <ToggleButton value="user">User</ToggleButton>
                <ToggleButton value="limited">Limited</ToggleButton>
              </ToggleButtonGroup>
            </div>
            {!isPrimaryDid && (
              <TextField
                label="Initial local password"
                type="password"
                size="small"
                fullWidth
                error={Boolean(form.formState.errors.localPassword)}
                helperText={
                  form.formState.errors.localPassword?.message ??
                  'Used only for this Zone-local account.'
                }
                {...form.register('localPassword')}
              />
            )}
          </div>
        )}

        {step === 3 && (
          <div className="space-y-3">
            <div>
              <div
                className="mb-1.5 text-[12px] font-medium"
                style={{ color: 'var(--cp-muted)' }}
              >
                Available apps
              </div>
              <div className="grid gap-1 sm:grid-cols-2">
                {userAppOptions.map((app) => {
                  const checked = values.availableApps.includes(app)
                  return (
                    <FormControlLabel
                      key={app}
                      control={
                        <Checkbox
                          size="small"
                          checked={checked}
                          onChange={(_, nextChecked) => {
                            const next = nextChecked
                              ? [...values.availableApps, app]
                              : values.availableApps.filter((item) => item !== app)
                            form.setValue('availableApps', next, { shouldValidate: true })
                          }}
                        />
                      }
                      label={app}
                    />
                  )
                })}
              </div>
            </div>
            <div>
              <div
                className="mb-1.5 text-[12px] font-medium"
                style={{ color: 'var(--cp-muted)' }}
              >
                Storage quota
              </div>
              <ToggleButtonGroup
                value={values.storageQuota}
                exclusive
                onChange={(_, value: NewZoneUserInput['storageQuota'] | null) => {
                  if (value) form.setValue('storageQuota', value, { shouldValidate: true })
                }}
                size="small"
              >
                {storageQuotaOptions.map((quota) => (
                  <ToggleButton key={quota} value={quota}>
                    {quota}
                  </ToggleButton>
                ))}
              </ToggleButtonGroup>
            </div>
            {isPrimaryDid && (
              <div>
                <div
                  className="mb-1.5 text-[12px] font-medium"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  Invitation expiry
                </div>
                <ToggleButtonGroup
                  value={values.invitationExpiresIn}
                  exclusive
                  onChange={(_, value: NewZoneUserInput['invitationExpiresIn'] | null) => {
                    if (value) form.setValue('invitationExpiresIn', value)
                  }}
                  size="small"
                >
                  <ToggleButton value="24h">24h</ToggleButton>
                  <ToggleButton value="7d">7d</ToggleButton>
                  <ToggleButton value="30d">30d</ToggleButton>
                </ToggleButtonGroup>
              </div>
            )}
          </div>
        )}

        {step === 4 && (
          <div className="space-y-3">
            <div
              className="rounded-[16px] px-4 py-3"
              style={{
                background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
              }}
            >
              {[
                ['Mode', isPrimaryDid ? 'Pending primary DID invitation' : 'Local account'],
                ['Identity', values.identifier || '-'],
                ['Display name', values.displayName || '-'],
                ['User type', values.userType],
                ['Default group', 'zone-members'],
                ['Apps', values.availableApps.join(', ')],
              ].map(([label, value]) => (
                <div key={label} className="flex items-baseline gap-3 py-1">
                  <span
                    className="w-28 shrink-0 text-[12px] font-medium"
                    style={{ color: 'var(--cp-muted)' }}
                  >
                    {label}
                  </span>
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {value}
                  </span>
                </div>
              ))}
            </div>
            {isPrimaryDid ? (
              <Alert icon={<Link size={18} />} severity="info">
                Completing this step creates a pending user and invitation URL.
                The target user activates it by updating binded_zone_list with
                their own root key.
              </Alert>
            ) : (
              <Alert severity="success">
                The local account will be active immediately and joined to the
                default base group.
              </Alert>
            )}
          </div>
        )}
      </div>

      <div
        className="mt-4 flex items-center justify-between border-t pt-3"
        style={{ borderColor: 'color-mix(in srgb, var(--cp-border) 40%, transparent)' }}
      >
        <Button
          type="button"
          size="small"
          disabled={step === 0}
          onClick={(event) => {
            event.preventDefault()
            setStep((current) => Math.max(current - 1, 0))
          }}
          startIcon={<ChevronLeft size={14} />}
        >
          Back
        </Button>

        {step < stepLabels.length - 1 ? (
          <Button
            type="button"
            size="small"
            variant="contained"
            onClick={(event) => {
              event.preventDefault()
              void handleNext()
            }}
            endIcon={<ChevronRight size={14} />}
          >
            Next
          </Button>
        ) : (
          <Button
            size="small"
            variant="contained"
            type="submit"
            startIcon={<UserPlus size={14} />}
          >
            {isPrimaryDid ? 'Create Invitation' : 'Create User'}
          </Button>
        )}
      </div>
    </form>
  )
}
