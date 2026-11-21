/* Appearance properties shared by a label Block and a Block whose content is a label.
 * Each host supplies its natural typography so removing an override restores its own look. */

import { AlignCenter, AlignLeft, AlignRight, Bold, Type } from 'lucide-react'
import { LABEL_ICONS, labelBackgroundImage, readLabelLook } from '@controls/label'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { SURFACE_ICONS } from '../canvas/icons'
import type { ConfigField, RenderContext } from './registry'

const SIZES = [11, 12, 13, 14, 15, 16, 18, 20, 24, 28, 32, 40, 48]

type BaseTypography = (context: RenderContext, store: WorkspaceStore) => { size: number; weight: number }

export function labelAppearanceFields(base: BaseTypography = () => ({ size: 14, weight: 400 })): ConfigField[] {
  const t = aiwsT
  const automatic = t('aiworkspace.label.colorAuto')
  return [
    { key: 'font_size', id: 'label-size', label: t('aiworkspace.label.fontSize'), kind: 'number', quick: true, icon: Type, min: 1, presets: SIZES,
      get: (context, store) => readLabelLook(context.payload.config).font_size ?? base(context, store).size,
      patch: (context, value, store) => ({ font_size: value === base(context, store).size ? null : value }) },
    { key: 'font_color', id: 'label-color', label: t('aiworkspace.label.color'), kind: 'color', quick: true, palette: 'ink', default: 'auto',
      extras: [{ value: 'auto', label: automatic }] },
    { key: 'weight', id: 'label-bold', label: t('aiworkspace.label.bold'), kind: 'boolean', quick: true, icon: Bold,
      get: (context, store) => {
        const weight = context.payload.config?.weight
        return weight ? weight === 'bold' : base(context, store).weight >= 700
      },
      patch: (context, value, store) => {
        const bold = value === true
        const defaultBold = base(context, store).weight >= 700
        return { weight: bold === defaultBold ? null : bold ? 'bold' : 'normal' }
      },
      editLabel: () => t('aiworkspace.label.bold') },
    { key: 'align', id: 'label-align', label: t('aiworkspace.label.align'), kind: 'select', quick: true, default: 'left',
      options: [{ value: 'left', label: t('aiworkspace.label.alignLeft'), icon: AlignLeft }, { value: 'center', label: t('aiworkspace.label.alignCenter'), icon: AlignCenter }, { value: 'right', label: t('aiworkspace.label.alignRight'), icon: AlignRight }] },
    { key: 'font_family', label: t('aiworkspace.blocks.shape.fontFamily'), kind: 'select', default: '',
      options: [{ value: '', label: t('aiworkspace.blocks.shape.fontFamilyTheme') }, { value: 'sans', label: t('aiworkspace.blocks.shape.fontFamily.sans') },
        { value: 'serif', label: t('aiworkspace.blocks.shape.fontFamily.serif') }, { value: 'mono', label: t('aiworkspace.blocks.shape.fontFamily.mono') }],
      get: (context) => context.payload.config?.font_family ?? (context.payload.config?.mono ? 'mono' : ''),
      patch: (context, value) => ({ font_family: value || null, ...(context.payload.config?.mono !== undefined ? { mono: null } : {}) }) },
    { key: 'italic', label: t('aiworkspace.blocks.shape.italic'), kind: 'boolean' },
    { key: 'icon', label: t('aiworkspace.shell.new.canvas.icon'), kind: 'select', default: '',
      options: [{ value: '', label: t('aiworkspace.common.none') }, ...SURFACE_ICONS.filter(({ id }) => Object.hasOwn(LABEL_ICONS, id)).map(({ id, label }) => ({ value: id, label: LABEL_ICONS[id] + ' ' + label }))] },
    { key: 'icon_position', label: t('aiworkspace.label.iconPosition'), kind: 'select', default: 'left',
      options: [{ value: 'left', label: t('aiworkspace.label.alignLeft') }, { value: 'right', label: t('aiworkspace.label.alignRight') }] },
    { key: 'background_color', label: t('aiworkspace.label.backgroundColor'), kind: 'color', palette: 'fill', default: 'auto', extras: [{ value: 'auto', label: automatic }] },
    { key: 'background_image', label: t('aiworkspace.label.backgroundImage'), kind: 'text',
      patch: (_context, value, store): Record<string, string | null> => {
        const input = typeof value === 'string' ? value.trim() : ''
        if (!input) return { background_image: null }
        const url = labelBackgroundImage(input)
        if (url) return { background_image: url }
        store.notify('error', t('aiworkspace.label.imageUrlInvalid'))
        return {}
      } },
  ]
}
