import { Button } from '@mui/material'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { copyText } from './format'

/** Copies `text()` and says whether it worked for two seconds. */
export function CopyButton({ label, text, testId }: { label: string; text: () => string; testId?: string }) {
  const { t } = useI18n()
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')

  useEffect(() => {
    if (state === 'idle') return
    const timer = window.setTimeout(() => setState('idle'), 2000)
    return () => window.clearTimeout(timer)
  }, [state])

  return (
    <Button
      size="small"
      variant="outlined"
      startIcon={state === 'copied' ? <Check size={14} /> : <Copy size={14} />}
      onClick={() => void copyText(text()).then((ok) => setState(ok ? 'copied' : 'failed'))}
      data-testid={testId}
    >
      {state === 'copied'
        ? t('settings.common.copied', 'Copied')
        : state === 'failed'
          ? t('settings.common.copyFailed', 'Copy failed')
          : label}
    </Button>
  )
}
