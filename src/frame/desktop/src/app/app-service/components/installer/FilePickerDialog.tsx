import { useEffect, useRef, useState } from 'react'
import { Cloud, FileArchive, X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { PickedPikgFile } from '../../mock/types'

const personalServerFiles: PickedPikgFile[] = [
  { location: 'personal-server', name: 'nextcloud-28.0.2-aarch64.pikg', sizeBytes: 1_204_289_536 },
  { location: 'personal-server', name: 'paperless-2.9.0-aarch64.pikg', sizeBytes: 836_763_648 },
  { location: 'personal-server', name: 'home-dashboard-0.8.4.pikg', sizeBytes: 214_958_080 },
]

function formatBytes(bytes: number) {
  return `${(bytes / 1_073_741_824).toFixed(bytes >= 1_073_741_824 ? 1 : 2)} GB`
}

interface FilePickerDialogProps {
  onCancel: () => void
  onSelect: (file: PickedPikgFile) => void
}

export function FilePickerDialog({ onCancel, onSelect }: FilePickerDialogProps) {
  const { t } = useI18n()
  const dialogRef = useRef<HTMLDialogElement>(null)
  const [selected, setSelected] = useState<PickedPikgFile | null>(null)

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    dialog.showModal()
    return () => dialog.close()
  }, [])

  return (
    <dialog
      ref={dialogRef}
      onCancel={(event) => { event.preventDefault(); onCancel() }}
      className="m-auto w-[min(620px,calc(100vw-32px))] rounded-[22px] p-0 backdrop:bg-[color-mix(in_srgb,var(--cp-shadow)_36%,transparent)]"
      style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-opaque)', border: '1px solid var(--cp-border-opaque)', boxShadow: 'var(--cp-window-shadow)' }}
      aria-labelledby="app-service-file-picker-title"
    >
      <header className="flex items-start justify-between gap-4 border-b p-5" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="flex items-start gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-xl" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-accent)' }}>
            <Cloud size={18} aria-hidden="true" />
          </span>
          <div>
            <h2 id="app-service-file-picker-title" className="font-display text-base font-semibold">
              {t('appService.source.personalServerTitle', 'Choose from Personal Server')}
            </h2>
            <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.source.personalServerBody', 'Only .pikg files can be returned to App Service. Server paths remain hidden.')}
            </p>
          </div>
        </div>
        <button
          type="button"
          onClick={onCancel}
          className="flex size-11 shrink-0 items-center justify-center rounded-xl"
          aria-label={t('common.close', 'Close')}
          style={{ color: 'var(--cp-muted)' }}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>

      <div className="max-h-[52vh] overflow-y-auto p-3 shell-scrollbar">
        {personalServerFiles.map((file) => {
          const isSelected = selected?.name === file.name
          return (
            <button
              key={file.name}
              type="button"
              onClick={() => setSelected(file)}
              className="flex min-h-16 w-full items-center gap-3 rounded-xl px-3 text-left"
              style={{
                color: 'var(--cp-text)',
                background: isSelected ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))' : 'transparent',
                border: `1px solid ${isSelected ? 'color-mix(in srgb, var(--cp-accent) 34%, var(--cp-border))' : 'transparent'}`,
              }}
            >
              <FileArchive size={18} aria-hidden="true" style={{ color: isSelected ? 'var(--cp-accent)' : 'var(--cp-muted)' }} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">{file.name}</span>
                <span className="mt-0.5 block text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>{formatBytes(file.sizeBytes)}</span>
              </span>
            </button>
          )
        })}
      </div>

      <footer className="flex flex-wrap justify-end gap-2 border-t p-4" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          onClick={onCancel}
          className="min-h-11 rounded-xl px-4 text-sm font-semibold"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          {t('common.cancel', 'Cancel')}
        </button>
        <button
          type="button"
          disabled={!selected}
          onClick={() => selected && onSelect(selected)}
          className="min-h-11 rounded-xl px-4 text-sm font-semibold disabled:opacity-40"
          style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
        >
          {t('appService.source.choosePackage', 'Choose package')}
        </button>
      </footer>
    </dialog>
  )
}
