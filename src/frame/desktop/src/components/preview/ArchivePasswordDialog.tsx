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
import './files.css'

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
    <div className="files-ui files-scrim absolute inset-0 z-40 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label={t('archive.password.title', 'Password required')} data-testid="archive-password">
      <form onSubmit={submit} className="w-full max-w-sm rounded-[12px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] p-5 shadow-[var(--cp-panel-shadow)]">
        <h2 className="flex items-center gap-2 text-[16px] font-medium text-[color:var(--cp-text)]"><Lock size={16} className="text-[color:var(--cp-muted)]" /> {t('archive.password.title', 'Password required')}</h2>
        <p className="mt-2 text-[14px] leading-5 text-[color:var(--cp-muted)]">{t('archive.password.body', '"{{name}}" is encrypted. Enter the archive password.', { name })}</p>
        <label className="files-label mt-4 block">
          {t('archive.password.label', 'Password')}
          <input
            type="password"
            autoFocus
            autoComplete="off"
            value={value}
            onChange={(e) => { setValue(e.target.value); if (state !== 'checking') setState('input') }}
            className={clsx('files-input mt-1.5 block font-normal', state === 'invalid' && '!border-[color:var(--cp-danger)]')}
            data-testid="archive-password-input"
          />
        </label>
        {state === 'invalid' ? <p className="mt-2 text-[12px] text-[color:var(--cp-danger)]" role="alert">{t('archive.password.invalid', 'The password is not right.')}</p> : null}
        {state === 'failed' ? <p className="mt-2 text-[12px] text-[color:var(--cp-danger)]" role="alert">{t('archive.password.failed', 'The password could not be checked.')}</p> : null}
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onCancel} className="files-btn">{t('archive.password.cancel', 'Cancel')}</button>
          <button type="submit" disabled={!value || state === 'checking'} className="files-btn files-btn-primary" data-testid="archive-password-submit">
            {state === 'checking' ? t('archive.password.checking', 'Checking…') : t('archive.password.unlock', 'Unlock')}
          </button>
        </div>
      </form>
    </div>
  )
}
