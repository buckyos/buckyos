/* ── Danger variants of the Desktop MUI buttons (the theme's variant overrides ignore `color`) ── */

export const dangerOutlinedSx = {
  color: 'var(--cp-danger)',
  borderColor: 'color-mix(in srgb, var(--cp-danger) 36%, var(--cp-border))',
  '&:hover': {
    borderColor: 'color-mix(in srgb, var(--cp-danger) 56%, var(--cp-border))',
    backgroundColor: 'color-mix(in srgb, var(--cp-danger) 8%, transparent)',
  },
} as const

export const dangerContainedSx = {
  background: 'var(--cp-danger)',
  color: 'var(--cp-bg)',
  boxShadow: 'none',
  '&:hover': { background: 'color-mix(in srgb, var(--cp-danger) 86%, var(--cp-text))', boxShadow: 'none' },
} as const

export const dangerTextSx = { color: 'var(--cp-danger)' } as const
