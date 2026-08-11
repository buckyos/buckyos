export const panelToneClasses = {
  accent:
    'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,var(--cp-surface))] text-[color:var(--cp-accent)]',
  success:
    'bg-[color:color-mix(in_srgb,var(--cp-success)_14%,var(--cp-surface))] text-[color:var(--cp-success)]',
  warning:
    'bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,var(--cp-surface))] text-[color:var(--cp-warning)]',
  neutral:
    'bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] text-[color:var(--cp-muted)]',
} as const

export function appIconSurfaceStyle(
  accent: string,
  tone: 'tile' | 'window' = 'tile',
) {
  const baseMix = tone === 'tile' ? '52%' : '46%'
  const edgeMix = tone === 'tile' ? '42%' : '36%'

  return {
    background: `linear-gradient(165deg, color-mix(in srgb, ${accent} ${baseMix}, var(--cp-surface-2)), color-mix(in srgb, ${accent} ${edgeMix}, var(--cp-surface-2)))`,
  }
}
