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
import { useI18n } from '../../../../i18n/provider'

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

const stepLabelKeys = ['usersAgents.newUser.step.account', 'usersAgents.newUser.step.review']

type Translate = ReturnType<typeof useI18n>['t']

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error ?? '')
}

function friendlyCreateError(error: unknown, t: Translate): string {
  const message = errorText(error)
  const lower = message.toLowerCase()
  if (lower.includes('already exists') || lower.includes('duplicate')) {
    return t('usersAgents.newUser.error.exists')
  }
  if (lower.includes('user_id') || lower.includes('username') || lower.includes('reserved')) {
    return t('usersAgents.newUser.error.invalidName')
  }
  if (lower.includes('password_hash') || lower.includes('password')) {
    return t('usersAgents.newUser.error.password')
  }
  if (lower.includes('permission') || lower.includes('admin')) {
    return t('usersAgents.newUser.error.permission')
  }
  if (lower.includes('expired') || lower.includes('invalid token')) {
    return t('usersAgents.newUser.error.sudoExpired')
  }
  if (lower.includes('network') || lower.includes('fetch') || lower.includes('connection')) {
    return t('usersAgents.newUser.error.network')
  }
  if (
    lower.includes('unavailable') ||
    lower.includes('timeout') ||
    lower.includes('failed to create user') ||
    lower.includes('503')
  ) {
    return t('usersAgents.newUser.error.unavailable')
  }
  return message || t('usersAgents.newUser.error.generic')
}

function isUncertainCreateError(error: unknown): boolean {
  const lower = errorText(error).toLowerCase()
  return lower.includes('network') || lower.includes('fetch') || lower.includes('connection')
}

export function NewUserWizard({ onClose, onCreated }: NewUserWizardProps) {
  const { t } = useI18n()
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
    setSubmitError(t('usersAgents.newUser.mayExist'))
  }

  const handleCreate = form.handleSubmit(async (data) => {
    if (submissionRef.current || phase === 'reload-failed') return
    submissionRef.current = true
    setSubmitError(null)

    const userId = data.username.trim().toLowerCase()
    if (store.findEntity(userId)) {
      setSubmitError(t('usersAgents.newUser.error.exists'))
      submissionRef.current = false
      return
    }

    try {
      setPhase('sudo')
      const grant = await requestSudo({
        aud: 'system-config',
        title: t('usersAgents.newUser.sudo.title'),
        description: t('usersAgents.newUser.sudo.description'),
        reason: `Create local user ${userId}`,
        confirmLabel: t('usersAgents.newUser.sudo.confirm'),
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
              warning: t('usersAgents.newUser.responseLost'),
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
        setSubmitError(friendlyCreateError(error ?? 'Invalid user.create response', t))
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
      setSubmitError(friendlyCreateError(error, t))
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
      warning: t('usersAgents.newUser.responseLost'),
      user_id: committedUserId,
      user_type: 'user',
      state: 'active',
    }
    try {
      if (await finishAfterReload(committedUserId, result)) return
      setPhase('reload-failed')
      setSubmitError(t('usersAgents.newUser.notVisible'))
    } catch {
      setPhase('reload-failed')
      setSubmitError(t('usersAgents.newUser.reloadFailed'))
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
            {t('usersAgents.newUser.title')}
          </h3>
        </div>
        <IconButton
          type="button"
          size="small"
          onClick={onClose}
          aria-label={t('usersAgents.newUser.close')}
          disabled={busy}
        >
          <X size={16} />
        </IconButton>
      </div>

      <div className="mb-4 flex items-center gap-1">
        {stepLabelKeys.map((labelKey, index) => (
          <div key={labelKey} className="flex items-center gap-1">
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
              {index + 1}. {t(labelKey)}
            </div>
          </div>
        ))}
      </div>

      <div className="min-h-[260px]">
        {submitError && <Alert severity={phase === 'reload-failed' ? 'warning' : 'error'}>{submitError}</Alert>}

        {step === 0 && (
          <div className="mt-3 space-y-3">
            <Alert severity="info">
              {t('usersAgents.newUser.intro')}
            </Alert>
            <TextField
              label={t('usersAgents.newUser.username')}
              size="small"
              fullWidth
              autoFocus
              autoComplete="off"
              error={Boolean(form.formState.errors.username)}
              helperText={form.formState.errors.username?.message ? t(form.formState.errors.username.message) : t('usersAgents.newUser.usernameHint')}
              {...form.register('username')}
            />
            <TextField
              label={t('usersAgents.newUser.displayName')}
              size="small"
              fullWidth
              error={Boolean(form.formState.errors.displayName)}
              helperText={form.formState.errors.displayName?.message ? t(form.formState.errors.displayName.message) : undefined}
              {...form.register('displayName')}
            />
            <TextField
              label={t('usersAgents.newUser.password')}
              type="password"
              size="small"
              fullWidth
              autoComplete="new-password"
              error={Boolean(form.formState.errors.password)}
              helperText={form.formState.errors.password?.message ? t(form.formState.errors.password.message) : t('usersAgents.newUser.passwordHint')}
              {...form.register('password')}
            />
            <TextField
              label={t('usersAgents.newUser.confirmPassword')}
              type="password"
              size="small"
              fullWidth
              autoComplete="new-password"
              error={Boolean(form.formState.errors.confirmPassword)}
              helperText={form.formState.errors.confirmPassword?.message ? t(form.formState.errors.confirmPassword.message) : undefined}
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
                [t('usersAgents.newUser.username'), values.username.trim().toLowerCase() || '-'],
                [t('usersAgents.newUser.displayName'), values.displayName.trim() || '-'],
                [t('usersAgents.newUser.userType'), t('usersAgents.role.user')],
                [t('usersAgents.newUser.state'), t('usersAgents.userStatus.active')],
                [t('usersAgents.newUser.passwordChanges'), t('usersAgents.newUser.allowed')],
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
              {t('usersAgents.newUser.reviewWarning')}
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
          {t('agentSetup.back')}
        </Button>

        {phase === 'reload-failed' ? (
          <Button
            type="button"
            size="small"
            variant="contained"
            onClick={() => void handleRetryReload()}
            startIcon={<RefreshCw size={14} />}
          >
            {t('usersAgents.newUser.retryReload')}
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
            {t('agentSetup.next')}
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
              ? t('usersAgents.newUser.waitingPermission')
              : phase === 'creating'
                ? t('usersAgents.newUser.creating')
                : phase === 'reloading'
                  ? t('usersAgents.newUser.reloading')
                  : t('usersAgents.newUser.create')}
          </Button>
        )}
      </div>
    </form>
  )
}
