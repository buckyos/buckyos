/* ── Avatar picker (ported from the OpenDAN WebUI profile dialog) ── */

import { useRef, useState } from 'react'
import { Button } from '@mui/material'
import { ImagePlus, Trash2 } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { AgentAvatar } from './ui'

const AVATAR_SIZE = 192

/** A square, centre-cropped copy small enough to keep in the profile. */
async function toAvatar(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file)
  const side = Math.min(bitmap.width, bitmap.height)
  const canvas = document.createElement('canvas')
  canvas.width = canvas.height = AVATAR_SIZE
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('no canvas')
  ctx.drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, AVATAR_SIZE, AVATAR_SIZE)
  bitmap.close()
  return canvas.toDataURL('image/jpeg', 0.85)
}

export function AvatarPicker({
  name,
  avatar,
  fallbackSrc,
  onChange,
}: {
  name: string
  /** The chosen avatar (data URL); null keeps the template / default one. */
  avatar: string | null
  fallbackSrc?: string
  onChange: (avatar: string | null) => void
}) {
  const { t } = useI18n()
  const file = useRef<HTMLInputElement>(null)
  const [failed, setFailed] = useState(false)

  const pick = async (picked?: File) => {
    if (!picked) return
    try {
      onChange(await toAvatar(picked))
      setFailed(false)
    } catch {
      setFailed(true)
    } finally {
      if (file.current) file.current.value = ''
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-4" data-testid="agent-setup-avatar">
      <AgentAvatar name={name || '?'} src={avatar ?? fallbackSrc} size={72} />
      <div className="flex min-w-0 flex-col items-start gap-1.5">
        <input
          ref={file}
          type="file"
          accept="image/*"
          className="hidden"
          aria-label={t('agentSetup.avatar.choose')}
          data-testid="agent-setup-avatar-input"
          onChange={(event) => void pick(event.target.files?.[0])}
        />
        <div className="flex flex-wrap gap-2">
          <Button size="small" variant="outlined" startIcon={<ImagePlus size={14} />} onClick={() => file.current?.click()}>
            {t('agentSetup.avatar.choose')}
          </Button>
          {avatar ? (
            <Button size="small" variant="text" startIcon={<Trash2 size={14} />} onClick={() => onChange(null)}>
              {t('agentSetup.avatar.remove')}
            </Button>
          ) : null}
        </div>
        <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">
          {failed ? (
            <span role="alert" className="text-[color:var(--cp-danger)]">{t('agentSetup.avatar.failed')}</span>
          ) : avatar ? t('agentSetup.avatar.custom') : t('agentSetup.avatar.default')}
        </p>
      </div>
    </div>
  )
}
