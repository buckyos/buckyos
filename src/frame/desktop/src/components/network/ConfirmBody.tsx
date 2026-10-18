/* ── Confirmation bodies of relationship actions; the strength is chosen by the action, never by the host ── */

import { Button, Checkbox, FormControlLabel } from '@mui/material'
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { dangerContainedSx } from './buttonStyles'

/** light: one sentence, confirm focused; explicit: consequences, cancel focused; strong: also an acknowledgement to tick. */
export type ConfirmStrength = 'light' | 'explicit' | 'strong'

export interface ConfirmOptions {
  title: string
  body: string
  consequences?: string[]
  confirmLabel: string
  strength: ConfirmStrength
  danger?: boolean
  /** Required tick of a `strong` confirmation. */
  acknowledge?: string
  /** An extra choice returned with the confirmation (e.g. “Tell them”). */
  option?: { label: string; initial: boolean }
}

export interface ConfirmResult {
  option: boolean
}

export function ConfirmBody({ options, onClose }: { options: ConfirmOptions; onClose: (result: ConfirmResult | null) => void }) {
  const { t } = useI18n()
  const [acknowledged, setAcknowledged] = useState(false)
  const [option, setOption] = useState(options.option?.initial ?? false)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    (options.strength === 'light' ? confirmRef : cancelRef).current?.focus()
  }, [options.strength])
  const blocked = options.strength === 'strong' && Boolean(options.acknowledge) && !acknowledged
  return (
    <div className="space-y-4" data-testid="network-confirm" data-strength={options.strength}>
      <p className="text-sm text-[color:var(--cp-text)]">{options.body}</p>
      {options.consequences?.length ? (
        <ul className="list-disc space-y-1 pl-5 text-[13px] text-[color:var(--cp-muted)]">
          {options.consequences.map(item => <li key={item}>{item}</li>)}
        </ul>
      ) : null}
      {options.option ? (
        <FormControlLabel control={<Checkbox size="small" checked={option} onChange={event => setOption(event.target.checked)} />} label={<span className="text-sm">{options.option.label}</span>} />
      ) : null}
      {options.strength === 'strong' && options.acknowledge ? (
        <FormControlLabel
          control={<Checkbox size="small" checked={acknowledged} onChange={event => setAcknowledged(event.target.checked)} />}
          label={<span className="text-sm">{options.acknowledge}</span>}
        />
      ) : null}
      <div className="flex justify-end gap-2">
        <Button variant="text" ref={cancelRef} onClick={() => onClose(null)}>{t('network.cancel')}</Button>
        <Button ref={confirmRef} variant="contained" sx={options.danger ? dangerContainedSx : undefined} disabled={blocked} onClick={() => onClose({ option })} data-testid="network-confirm-submit">
          {options.confirmLabel}
        </Button>
      </div>
    </div>
  )
}
