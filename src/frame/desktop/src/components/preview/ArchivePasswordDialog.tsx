/**
 * The archive password prompt (archive TODO T6, more-formats E7): a member's
 * password in Preview, or the root's for an archive whose names are encrypted
 * (Preview and Files). The caller places it over the area it covers.
 */

import clsx from 'clsx'
import { Lock } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { useI18n } from '../../i18n/provider'
import type { NfspError } from '../../api/nfsp_client'
import { unlockArchive } from './archive'

export function ArchivePasswordDialog({ name, url, onUnlocked, onCancel }: { name: string; url: string; onUnlocked: () => void; onCancel: () => void }) {
  const { t } = useI18n()
  const [value, setValue] = useState('')
  const [state, setState] = useState<'input' | 'checking' | 'invalid' | 'failed'>('input')
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (!value || state === 'checking') return
    setState('checking')
    unlockArchive(url, value).then(onUnlocked, (error: unknown) => {
      setState((error as NfspError | undefined)?.code === 'PASSWORD_INVALID' ? 'invalid' : 'failed')
    })
  }
  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-[rgba(8,10,16,0.45)] p-4" role="dialog" aria-modal="true" aria-label={t('archive.password.title', 'Password required')} data-testid="archive-password">
      <form onSubmit={submit} className="w-full max-w-sm rounded-2xl bg-[color:var(--cp-surface)] p-5 shadow-xl">
        <h2 className="flex items-center gap-2 text-base font-semibold text-[color:var(--cp-text)]"><Lock size={16} /> {t('archive.password.title', 'Password required')}</h2>
        <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('archive.password.body', '"{{name}}" is encrypted. Enter the archive password.', { name })}</p>
        <label className="mt-4 block text-xs font-semibold text-[color:var(--cp-muted)]">
          {t('archive.password.label', 'Password')}
          <input
            type="password"
            autoFocus
            autoComplete="off"
            value={value}
            onChange={(e) => { setValue(e.target.value); if (state !== 'checking') setState('input') }}
            className={clsx('mt-1 block w-full rounded-lg border bg-transparent px-3 py-2 text-sm text-[color:var(--cp-text)] outline-none', state === 'invalid' ? 'border-red-500' : 'border-[color:var(--cp-border)]')}
            data-testid="archive-password-input"
          />
        </label>
        {state === 'invalid' ? <p className="mt-2 text-xs text-red-500" role="alert">{t('archive.password.invalid', 'The password is not right.')}</p> : null}
        {state === 'failed' ? <p className="mt-2 text-xs text-red-500" role="alert">{t('archive.password.failed', 'The password could not be checked.')}</p> : null}
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onCancel} className="rounded-full px-4 py-1.5 text-sm text-[color:var(--cp-text)]">{t('archive.password.cancel', 'Cancel')}</button>
          <button type="submit" disabled={!value || state === 'checking'} className="rounded-full bg-[color:var(--cp-accent)] px-4 py-1.5 text-sm font-semibold text-white disabled:opacity-50" data-testid="archive-password-submit">
            {state === 'checking' ? t('archive.password.checking', 'Checking…') : t('archive.password.unlock', 'Unlock')}
          </button>
        </div>
      </form>
    </div>
  )
}
