import { zodResolver } from '@hookform/resolvers/zod'
import { buckyos } from 'buckyos'
import { useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import {
  Alert,
  Button,
  CircularProgress,
  IconButton,
  TextField,
} from '@mui/material'
import { ChevronLeft, ChevronRight, RefreshCw, UserPlus, X } from 'lucide-react'
import { createUser, type UserCreateResponse } from '../../../../api/user_mgr'
import { useSudoByPassword } from '../../../../components/sudo'
import type { NewZoneUserInput } from '../../datamodel/types'
import { newZoneUserInputSchema } from '../../datamodel/types'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface NewUserWizardProps {
  onClose: () => void
  onCreated?: (userId: string, result: UserCreateResponse) => void
}

type SubmitPhase = 'idle' | 'sudo' | 'creating' | 'reloading' | 'reload-failed'

const defaultValues: NewZoneUserInput = {
  username: '',
  displayName: '',
  password: '',
  confirmPassword: '',
}

const stepLabels = ['Account', 'Review']

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error ?? '')
}

function friendlyCreateError(error: unknown): string {
  const message = errorText(error)
  const lower = message.toLowerCase()
  if (lower.includes('already exists') || lower.includes('duplicate')) {
    return 'That username already exists. Choose another username.'
  }
  if (lower.includes('user_id') || lower.includes('username') || lower.includes('reserved')) {
    return 'The username is invalid. Use 1–64 letters, numbers, underscores, hyphens, or dots.'
  }
  if (lower.includes('password_hash') || lower.includes('password')) {
    return 'The password could not be processed. Check it and try again.'
  }
  if (lower.includes('permission') || lower.includes('admin')) {
    return 'Administrator permission is required to create a local user.'
  }
  if (lower.includes('expired') || lower.includes('invalid token')) {
    return 'The temporary administrator permission expired. Try again.'
  }
  if (lower.includes('network') || lower.includes('fetch') || lower.includes('connection')) {
    return 'The request could not be confirmed because the connection was interrupted.'
  }
  if (
    lower.includes('unavailable') ||
    lower.includes('timeout') ||
    lower.includes('failed to create user') ||
    lower.includes('503')
  ) {
    return 'The account service is temporarily unavailable. Wait a moment and try again.'
  }
  return message || 'The user could not be created. Try again.'
}

function isUncertainCreateError(error: unknown): boolean {
  const lower = errorText(error).toLowerCase()
  return lower.includes('network') || lower.includes('fetch') || lower.includes('connection')
}

export function NewUserWizard({ onClose, onCreated }: NewUserWizardProps) {
  const [step, setStep] = useState(0)
  const [phase, setPhase] = useState<SubmitPhase>('idle')
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [committedUserId, setCommittedUserId] = useState<string | null>(null)
  const [createResult, setCreateResult] = useState<UserCreateResponse | null>(null)
  const submissionRef = useRef(false)
  const store = useUsersAgentsStore()
  const requestSudo = useSudoByPassword()
  const form = useForm<NewZoneUserInput>({
    resolver: zodResolver(newZoneUserInputSchema),
    defaultValues,
    mode: 'onChange',
  })
  const values = form.watch()
  const busy = phase === 'sudo' || phase === 'creating' || phase === 'reloading'

  const finishAfterReload = async (
    userId: string,
    result: UserCreateResponse,
  ): Promise<boolean> => {
    const snapshot = await store.reload()
    if (!snapshot.localUsers.some((user) => user.id === userId)) {
      return false
    }
    onCreated?.(userId, result)
    onClose()
    return true
  }

  const markReloadUncertain = (userId: string, result: UserCreateResponse | null) => {
    setCommittedUserId(userId)
    setCreateResult(result)
    setPhase('reload-failed')
    setSubmitError(
      'The user may already have been created. Retry reload before submitting again.',
    )
  }

  const handleCreate = form.handleSubmit(async (data) => {
    if (submissionRef.current || phase === 'reload-failed') return
    submissionRef.current = true
    setSubmitError(null)

    const userId = data.username.trim().toLowerCase()
    if (store.findEntity(userId)) {
      setSubmitError('That username already exists. Choose another username.')
      submissionRef.current = false
      return
    }

    try {
      setPhase('sudo')
      const grant = await requestSudo({
        aud: 'system-config',
        title: 'Create local user',
        description: 'Confirm your administrator password to create this Zone-local account.',
        reason: `Create local user ${userId}`,
        confirmLabel: 'Create user',
      })
      if (!grant) {
        setPhase('idle')
        return
      }

      setPhase('creating')
      const passwordHash = buckyos.hashPassword(userId, data.password)
      const { data: result, error } = await createUser(
        {
          userId,
          showName: data.displayName.trim(),
          passwordHash,
          userType: 'user',
          allowPasswordChange: true,
        },
        { sessionToken: grant.sessionToken },
      )

      if (error || !result?.ok || !result.created) {
        if (isUncertainCreateError(error)) {
          try {
            setPhase('reloading')
            const recoveredResult: UserCreateResponse = {
              ok: true,
              created: true,
              rbac_refreshed: false,
              warning: 'The creation response was lost; the account was found after reload.',
              user_id: userId,
              user_type: 'user',
              state: 'active',
            }
            if (await finishAfterReload(userId, recoveredResult)) return
          } catch {
            // Fall through to the explicit uncertain-result state.
          }
          markReloadUncertain(userId, null)
          return
        }
        setPhase('idle')
        setSubmitError(friendlyCreateError(error ?? 'Invalid user.create response'))
        return
      }

      setCreateResult(result)
      setCommittedUserId(userId)
      setPhase('reloading')
      try {
        if (await finishAfterReload(userId, result)) return
      } catch {
        // The committed account is reconciled through the retry path below.
      }
      markReloadUncertain(userId, result)
    } catch (error) {
      setPhase('idle')
      setSubmitError(friendlyCreateError(error))
    } finally {
      submissionRef.current = false
    }
  })

  const handleRetryReload = async () => {
    if (!committedUserId || submissionRef.current) return
    submissionRef.current = true
    setPhase('reloading')
    setSubmitError(null)
    const result = createResult ?? {
      ok: true,
      created: true,
      rbac_refreshed: false,
      warning: 'The creation response was lost; the account was found after reload.',
      user_id: committedUserId,
      user_type: 'user',
      state: 'active',
    }
    try {
      if (await finishAfterReload(committedUserId, result)) return
      setPhase('reload-failed')
      setSubmitError('The account is not visible yet. Retry reload; do not submit it again.')
    } catch {
      setPhase('reload-failed')
      setSubmitError('Reload failed. The user may already exist; retry reload when connected.')
    } finally {
      submissionRef.current = false
    }
  }

  const handleNext = async () => {
    if (await form.trigger()) setStep(1)
  }

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
          <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            New Local User
          </h3>
        </div>
        <IconButton
          type="button"
          size="small"
          onClick={onClose}
          aria-label="Close new user wizard"
          disabled={busy}
        >
          <X size={16} />
        </IconButton>
      </div>

      <div className="mb-4 flex items-center gap-1">
        {stepLabels.map((label, index) => (
          <div key={label} className="flex items-center gap-1">
            {index > 0 && (
              <div
                className="h-[1px] w-8"
                style={{ background: index <= step ? 'var(--cp-accent)' : 'var(--cp-border)' }}
              />
            )}
            <div
              className="rounded-full px-2 py-0.5 text-[11px] font-medium"
              style={{ color: index <= step ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
            >
              {index + 1}. {label}
            </div>
          </div>
        ))}
      </div>

      <div className="min-h-[260px]">
        {submitError && <Alert severity={phase === 'reload-failed' ? 'warning' : 'error'}>{submitError}</Alert>}

        {step === 0 && (
          <div className="mt-3 space-y-3">
            <Alert severity="info">
              This creates an ordinary user who can sign in to this Zone immediately.
            </Alert>
            <TextField
              label="Local username"
              size="small"
              fullWidth
              autoFocus
              autoComplete="off"
              error={Boolean(form.formState.errors.username)}
              helperText={form.formState.errors.username?.message ?? 'Saved in lowercase.'}
              {...form.register('username')}
            />
            <TextField
              label="Display name"
              size="small"
              fullWidth
              error={Boolean(form.formState.errors.displayName)}
              helperText={form.formState.errors.displayName?.message}
              {...form.register('displayName')}
            />
            <TextField
              label="Initial password"
              type="password"
              size="small"
              fullWidth
              autoComplete="new-password"
              error={Boolean(form.formState.errors.password)}
              helperText={form.formState.errors.password?.message ?? 'At least 8 characters.'}
              {...form.register('password')}
            />
            <TextField
              label="Confirm password"
              type="password"
              size="small"
              fullWidth
              autoComplete="new-password"
              error={Boolean(form.formState.errors.confirmPassword)}
              helperText={form.formState.errors.confirmPassword?.message}
              {...form.register('confirmPassword')}
            />
          </div>
        )}

        {step === 1 && (
          <div className="mt-3 space-y-3">
            <div
              className="rounded-[16px] px-4 py-3"
              style={{
                background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
              }}
            >
              {[
                ['Local username', values.username.trim().toLowerCase() || '-'],
                ['Display name', values.displayName.trim() || '-'],
                ['User type', 'User'],
                ['State', 'Active'],
                ['Password changes', 'Allowed'],
              ].map(([label, value]) => (
                <div key={label} className="flex items-baseline gap-3 py-1">
                  <span className="w-36 shrink-0 text-[12px] font-medium" style={{ color: 'var(--cp-muted)' }}>
                    {label}
                  </span>
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {value}
                  </span>
                </div>
              ))}
            </div>
            <Alert severity="warning">
              This account exists only in the current Zone, depends on this Zone staying available,
              can sign in immediately, and consumes local resources. No apps are installed or promised
              for the new user by this action.
            </Alert>
          </div>
        )}
      </div>

      <div className="mt-4 flex items-center justify-between border-t pt-3" style={{ borderColor: 'var(--cp-border)' }}>
        <Button
          type="button"
          size="small"
          disabled={step === 0 || busy || phase === 'reload-failed'}
          onClick={() => setStep(0)}
          startIcon={<ChevronLeft size={14} />}
        >
          Back
        </Button>

        {phase === 'reload-failed' ? (
          <Button
            type="button"
            size="small"
            variant="contained"
            onClick={() => void handleRetryReload()}
            startIcon={<RefreshCw size={14} />}
          >
            Retry reload
          </Button>
        ) : step === 0 ? (
          <Button
            type="button"
            size="small"
            variant="contained"
            disabled={busy}
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
            disabled={busy}
            startIcon={busy ? <CircularProgress color="inherit" size={14} /> : <UserPlus size={14} />}
          >
            {phase === 'sudo'
              ? 'Waiting for permission…'
              : phase === 'creating'
                ? 'Creating…'
                : phase === 'reloading'
                  ? 'Reloading…'
                  : 'Create User'}
          </Button>
        )}
      </div>
    </form>
  )
}
