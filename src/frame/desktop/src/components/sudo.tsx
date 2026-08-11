/* eslint-disable react-refresh/only-export-components */
import {
  Alert,
  Button,
  CircularProgress,
  IconButton,
  InputAdornment,
  TextField,
} from '@mui/material'
import { Eye, EyeOff, LockKeyhole, ShieldCheck } from 'lucide-react'
import {
  useCallback,
  useMemo,
  useState,
  type FormEvent,
} from 'react'
import { buckyos } from 'buckyos'
import { useI18n } from '../i18n/provider'
import {
  useWindowDialog,
  type WindowDialogControls,
} from '../desktop/windows/dialogs'

const VERIFY_HUB_ENDPOINT = '/kapi/verify-hub'
const DEFAULT_APP_ID = 'control-panel'
const SUDO_TOKEN_FALLBACK_TTL_MS = 3 * 60 * 1000

export type SudoErrorCode =
  | 'missing_account'
  | 'empty_password'
  | 'invalid_response'
  | 'request_failed'

export class SudoRequestError extends Error {
  code: SudoErrorCode

  constructor(code: SudoErrorCode, message: string) {
    super(message)
    this.name = 'SudoRequestError'
    this.code = code
  }
}

export interface SudoByPasswordParams {
  username: string
  password: string
  appid: string
  aud?: string
}

export interface SudoGrant {
  sessionToken: string
  expiresAtMs: number
  expiresInSeconds: number
  username: string
  appid: string
  aud?: string
}

export interface SudoDialogOptions {
  username?: string
  appid?: string
  aud?: string
  title?: string
  description?: string
  reason?: string
  confirmLabel?: string
  cancelLabel?: string
}

interface SudoByPasswordResponse {
  session_token?: unknown
}

interface JwtClaims {
  exp?: unknown
}

type AccountInfo = NonNullable<Awaited<ReturnType<typeof buckyos.getAccountInfo>>>

function decodeJwtClaims(sessionToken: string): JwtClaims | null {
  const payload = sessionToken.split('.')[1]
  if (!payload) {
    return null
  }

  try {
    const base64 = payload.replace(/-/g, '+').replace(/_/g, '/')
    const padded = base64 + '='.repeat((4 - (base64.length % 4)) % 4)
    return JSON.parse(window.atob(padded)) as JwtClaims
  } catch {
    return null
  }
}

function resolveExpiresAtMs(sessionToken: string) {
  const claims = decodeJwtClaims(sessionToken)
  if (typeof claims?.exp === 'number' && Number.isFinite(claims.exp)) {
    return claims.exp * 1000
  }

  return Date.now() + SUDO_TOKEN_FALLBACK_TTL_MS
}

function resolveUsername(accountInfo: AccountInfo | null, username?: string) {
  const candidate =
    username?.trim() ||
    accountInfo?.user_id?.trim() ||
    accountInfo?.user_name?.trim() ||
    ''

  if (!candidate) {
    throw new SudoRequestError(
      'missing_account',
      'Cannot request sudo without a signed-in account.',
    )
  }

  return candidate
}

function resolveAppId(appid?: string) {
  return appid?.trim() || buckyos.getAppId()?.trim() || DEFAULT_APP_ID
}

function normalizeSudoError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error ?? '')
  const lower = message.toLowerCase()

  if (lower.includes('invalidpassword') || lower.includes('invalid password')) {
    return 'Incorrect password. Please try again.'
  }

  if (lower.includes('no permission') || lower.includes('only admin')) {
    return 'Only admin users can request sudo permission.'
  }

  if (lower.includes('network') || lower.includes('fetch') || lower.includes('failed to fetch')) {
    return 'Cannot reach Verify Hub. Please check the current connection.'
  }

  if (lower.includes('invalid nonce') || lower.includes('nonce already used')) {
    return 'This sudo request expired. Please try again.'
  }

  return message || 'Sudo request failed. Please try again.'
}

export async function sudoByPassword({
  username,
  password,
  appid,
  aud,
}: SudoByPasswordParams): Promise<SudoGrant> {
  const normalizedUsername = username.trim()
  const normalizedAppId = appid.trim()

  if (!normalizedUsername || !normalizedAppId) {
    throw new SudoRequestError(
      'missing_account',
      'Username and appid are required for sudo.',
    )
  }

  if (!password) {
    throw new SudoRequestError('empty_password', 'Password is required for sudo.')
  }

  const nonce = Date.now()
  const passwordHash = buckyos.hashPassword(normalizedUsername, password, nonce)
  const rpcClient = new buckyos.kRPCClient(VERIFY_HUB_ENDPOINT)
  rpcClient.setSeq(nonce)

  try {
    const response = await rpcClient.call<SudoByPasswordResponse, Record<string, unknown>>(
      'sudo_by_password',
      {
        username: normalizedUsername,
        password: passwordHash,
        appid: normalizedAppId,
        ...(aud ? { aud } : {}),
        login_nonce: nonce,
      },
    )
    const sessionToken =
      typeof response.session_token === 'string'
        ? response.session_token.trim()
        : ''

    if (!sessionToken) {
      throw new SudoRequestError(
        'invalid_response',
        'Verify Hub did not return a sudo token.',
      )
    }

    const expiresAtMs = resolveExpiresAtMs(sessionToken)
    const expiresInSeconds = Math.max(0, Math.floor((expiresAtMs - Date.now()) / 1000))

    return {
      sessionToken,
      expiresAtMs,
      expiresInSeconds,
      username: normalizedUsername,
      appid: normalizedAppId,
      ...(aud ? { aud } : {}),
    }
  } catch (error) {
    if (error instanceof SudoRequestError) {
      throw error
    }

    throw new SudoRequestError('request_failed', normalizeSudoError(error))
  }
}

function SudoPasswordForm({
  appid,
  aud,
  cancelLabel,
  confirmLabel,
  controls,
  reason,
  username,
}: {
  appid: string
  aud?: string
  cancelLabel: string
  confirmLabel: string
  controls: WindowDialogControls<SudoGrant>
  reason?: string
  username: string
}) {
  const { t } = useI18n()
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const canSubmit = password.length > 0 && !submitting

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!canSubmit) {
      return
    }

    setSubmitting(true)
    setError(null)

    try {
      const grant = await sudoByPassword({
        username,
        password,
        appid,
        aud,
      })
      controls.close(grant)
    } catch (submitError) {
      setError(normalizeSudoError(submitError))
      setSubmitting(false)
    }
  }

  return (
    <form className="space-y-4" onSubmit={(event) => void handleSubmit(event)}>
      <div className="rounded-[18px] border border-[color:color-mix(in_srgb,var(--cp-border)_76%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_54%,var(--cp-surface))] px-4 py-3">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-[color:var(--cp-muted)]">
          {t('sudo.account', 'Account')}
        </p>
        <p className="mt-1 break-all text-sm font-medium text-[color:var(--cp-text)]">
          {username}
        </p>
        {aud ? (
          <p className="mt-2 break-all text-xs text-[color:var(--cp-muted)]">
            {t('sudo.audience', 'Scope')}: {aud}
          </p>
        ) : null}
      </div>

      {reason ? (
        <div className="rounded-[18px] border border-[color:color-mix(in_srgb,var(--cp-accent-soft)_24%,var(--cp-border))] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_8%,var(--cp-surface))] px-4 py-3 text-sm leading-6 text-[color:var(--cp-text)]">
          {reason}
        </div>
      ) : null}

      {error ? <Alert severity="error">{error}</Alert> : null}

      <TextField
        autoComplete="current-password"
        autoFocus
        disabled={submitting}
        fullWidth
        label={t('sudo.password', 'Password')}
        type={showPassword ? 'text' : 'password'}
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        InputProps={{
          startAdornment: (
            <InputAdornment position="start">
              <LockKeyhole size={16} />
            </InputAdornment>
          ),
          endAdornment: (
            <InputAdornment position="end">
              <IconButton
                aria-label={
                  showPassword
                    ? t('sudo.hidePassword', 'Hide password')
                    : t('sudo.showPassword', 'Show password')
                }
                edge="end"
                size="small"
                type="button"
                onClick={() => setShowPassword((value) => !value)}
              >
                {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
              </IconButton>
            </InputAdornment>
          ),
        }}
      />

      <p className="text-xs leading-5 text-[color:var(--cp-muted)]">
        {t(
          'sudo.tokenHint',
          'The sudo token is returned to this operation only and expires in about 3 minutes.',
        )}
      </p>

      <div className="flex flex-wrap items-center justify-end gap-3 pt-1">
        <Button
          disabled={submitting}
          type="button"
          variant="text"
          onClick={() => controls.dismiss()}
        >
          {cancelLabel}
        </Button>
        <Button
          disabled={!canSubmit}
          startIcon={
            submitting ? (
              <CircularProgress color="inherit" size={16} />
            ) : (
              <ShieldCheck size={16} />
            )
          }
          type="submit"
          variant="contained"
        >
          {submitting ? t('sudo.requesting', 'Requesting...') : confirmLabel}
        </Button>
      </div>
    </form>
  )
}

export function useSudoByPassword() {
  const windowDialog = useWindowDialog()
  const { t } = useI18n()

  return useCallback(
    async (options: SudoDialogOptions = {}): Promise<SudoGrant | null> => {
      const accountInfo = await buckyos.getAccountInfo()
      const username = resolveUsername(accountInfo, options.username)
      const appid = resolveAppId(options.appid)
      const title = options.title ?? t('sudo.title', 'Administrator permission')
      const description =
        options.description ??
        (options.aud
          ? t(
              'sudo.descriptionWithAudience',
              'Confirm your password to grant temporary sudo access for {{aud}}.',
              { aud: options.aud },
            )
          : t(
              'sudo.description',
              'Confirm your password to grant temporary sudo access.',
            ))
      const confirmLabel = options.confirmLabel ?? t('sudo.confirm', 'Grant sudo')
      const cancelLabel = options.cancelLabel ?? t('common.cancel', 'Cancel')

      const result = await windowDialog.open<SudoGrant>({
        closeOnBackdrop: false,
        description,
        dismissible: true,
        presentation: 'auto',
        size: 'sm',
        title,
        renderBody: (controls) => (
          <SudoPasswordForm
            appid={appid}
            aud={options.aud}
            cancelLabel={cancelLabel}
            confirmLabel={confirmLabel}
            controls={controls}
            reason={options.reason}
            username={username}
          />
        ),
      })

      return result ?? null
    },
    [t, windowDialog],
  )
}

export function useSudo() {
  const requestSudo = useSudoByPassword()

  return useMemo(
    () => ({
      requestSudo,
      sudoByPassword: requestSudo,
    }),
    [requestSudo],
  )
}
