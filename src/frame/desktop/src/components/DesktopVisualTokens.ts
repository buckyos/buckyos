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

export function appIconSurfaceStyle(accent: string, iconKey?: string) {
  if (iconKey === 'aiworkspace') return { background: 'transparent', boxShadow: 'none' }
  return {
    background: `linear-gradient(180deg, color-mix(in srgb, ${accent} 90%, white), ${accent})`,
    boxShadow: 'inset 0 1px 0 rgb(255 255 255 / 0.16), 0 1px 2px rgb(0 0 0 / 0.12)',
  }
}
