import { Alert } from '@mui/material'
import type { AccessDenial } from 'buckyos'
import { useI18n } from '../i18n/provider'
import { denialAdjustText, denialText } from './denial'

/**
 * A denial as one notice. `showAdjust` is for a non-restricted viewer (an admin): it adds who
 * adjusts the cause and in which Control Panel page; visitors only see the category.
 */
export function DenialNotice({
  denial,
  showAdjust = false,
  testId = 'sso-denial',
}: {
  denial: AccessDenial
  showAdjust?: boolean
  testId?: string
}) {
  const { t } = useI18n()
  const adjust = showAdjust ? denialAdjustText(t, denial) : null
  const severity = denial.reason === 'authorization_unavailable' || denial.reason === 'session_ceiling_exceeded' ? 'warning' : 'error'
  return (
    <Alert severity={severity} data-testid={testId} data-reason={denial.reason}>
      <span className="block">{denialText(t, denial)}</span>
      {adjust ? (
        <span className="mt-1 block text-xs opacity-80" data-testid={`${testId}-adjust`}>
          {adjust}
        </span>
      ) : null}
    </Alert>
  )
}
