import { useI18n } from '../../i18n/provider'
import { statusLabel } from '../../sysdlg/shareFormat'

/** Status as text with a quiet border cue; color never carries it alone. */
export function StatusChip({ status }: { status: string }) {
  const { t } = useI18n()
  return <span className="mc-chip" data-status={status} data-testid="mc-status">{statusLabel(t, status)}</span>
}
