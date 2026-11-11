/* ── Choose / replace / remove an image (avatars, Zone icons) with a local preview ── */

import { useState, type ReactNode } from 'react'
import { Button } from '@mui/material'
import { ImagePlus, Trash2 } from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { usePickFiles } from '../sysdlg/select/pickFiles'
import { ObjectAvatar } from './ObjectIcon'
import {
  ACCEPTED_IMAGE_TYPES,
  ImageFileError,
  imageFileToDataUrl,
  type ImageFileErrorReason,
  type ImageProcessOptions,
} from './imageFile'

export interface ImagePickerLabels {
  choose: string
  remove: string
  /** Shown while no image of the user's own is chosen. */
  usingDefault: string
  /** Shown while the user's image is chosen. */
  usingCustom: string
}

export function ImagePicker({
  title,
  value,
  fallbackIcons = [],
  fallbackIcon,
  shape = 'circle',
  options,
  labels,
  disabled = false,
  previewSize = 72,
  onChange,
  testId,
  chooseTestId,
  onPickingChange,
}: {
  title: string
  /** The chosen image (a data URL); null means "no image of our own". */
  value: string | null
  /** Previewed when `value` is null, e.g. the template or document default. */
  fallbackIcons?: readonly string[]
  fallbackIcon?: ReactNode
  shape?: 'circle' | 'square'
  options: ImageProcessOptions
  labels: ImagePickerLabels
  disabled?: boolean
  previewSize?: number
  onChange: (value: string | null) => void
  testId?: string
  chooseTestId?: string
  /** While the system file picker is open; a caller inside a page-level modal hides it meanwhile. */
  onPickingChange?: (picking: boolean) => void
}) {
  const { t } = useI18n()
  const pickFiles = usePickFiles('image-picker')
  const [error, setError] = useState<ImageFileErrorReason | null>(null)
  const [picking, setPicking] = useState(false)

  const pick = async () => {
    setPicking(true)
    onPickingChange?.(true)
    try {
      const picked = (await pickFiles({ accept: ACCEPTED_IMAGE_TYPES.join(','), title: labels.choose }))?.[0]
      if (!picked) return
      onChange(await imageFileToDataUrl(picked, options))
      setError(null)
    } catch (caught) {
      setError(caught instanceof ImageFileError ? caught.reason : 'unreadable')
    } finally {
      setPicking(false)
      onPickingChange?.(false)
    }
  }

  const errorText = error === 'type'
    ? t('presentation.image.errorType', 'Choose a PNG, JPEG, WebP, GIF or BMP image.')
    : error === 'too_large'
      ? t('presentation.image.errorSize', 'That image is too large.')
      : t('presentation.image.errorUnreadable', 'That image could not be read. The previous one is kept.')

  return (
    <div className="flex flex-wrap items-center gap-4" data-testid={testId}>
      <ObjectAvatar
        title={title || '?'}
        icons={value ? [value] : fallbackIcons}
        size={previewSize}
        shape={shape}
        fallbackIcon={fallbackIcon}
        decorative
      />
      <div className="flex min-w-0 flex-col items-start gap-1.5">
        <div className="flex flex-wrap gap-2">
          <Button size="small" variant="outlined" startIcon={<ImagePlus size={14} />} disabled={disabled || picking} onClick={() => void pick()} data-testid={chooseTestId}>
            {labels.choose}
          </Button>
          {value ? (
            <Button size="small" variant="text" startIcon={<Trash2 size={14} />} disabled={disabled || picking} onClick={() => { setError(null); onChange(null) }}>
              {labels.remove}
            </Button>
          ) : null}
        </div>
        <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">
          {error ? (
            <span role="alert" className="text-[color:var(--cp-danger)]">{errorText}</span>
          ) : value ? labels.usingCustom : labels.usingDefault}
        </p>
      </div>
    </div>
  )
}
