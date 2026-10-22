import { useState } from 'react'
import { Alert, Button } from '@mui/material'
import { Check, Copy } from 'lucide-react'
import { useI18n } from '../i18n/provider'

/**
 * A credential link shown once (exclusive invite create / rotate): copy it now and send it through
 * the bound channel; it cannot be read again. `onDismiss` drops it from the page.
 */
export function OneTimeLink({ link, onDismiss, testId = 'sso-one-time-link' }: { link: string; onDismiss: () => void; testId?: string }) {
  const { t } = useI18n()
  const [copy, setCopy] = useState<'idle' | 'copied' | 'failed'>('idle')

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(link)
      setCopy('copied')
    } catch {
      setCopy('failed')
    }
  }

  return (
    <Alert severity="warning" data-testid={testId} onClose={onDismiss}>
      <span className="block font-medium">
        {t('sso.oneTimeLink.warning', 'This link is shown only now. Copy it and send it through the bound channel; it will not be shown again.')}
      </span>
      <code
        className="mt-2 block select-all break-all rounded-lg px-2 py-1.5 text-xs"
        style={{ background: 'color-mix(in srgb, var(--cp-surface) 85%, transparent)', color: 'var(--cp-text)' }}
        data-testid={`${testId}-value`}
      >
        {link}
      </code>
      <span className="mt-2 flex flex-wrap gap-2">
        <Button size="small" variant="outlined" startIcon={copy === 'copied' ? <Check size={14} /> : <Copy size={14} />} onClick={() => void copyLink()} data-testid={`${testId}-copy`}>
          {copy === 'copied'
            ? t('sso.oneTimeLink.copied', 'Copied')
            : copy === 'failed'
              ? t('sso.oneTimeLink.copyFailed', 'Copy failed; select the link instead')
              : t('sso.oneTimeLink.copy', 'Copy link')}
        </Button>
        <Button size="small" variant="text" onClick={onDismiss} data-testid={`${testId}-done`}>
          {t('sso.oneTimeLink.done', 'Done')}
        </Button>
      </span>
    </Alert>
  )
}
