/** An on/off switch with the look of the wizard's auto-sync toggle. */
export function Switch({ checked, onChange, label, disabled = false, testId }: {
  checked: boolean
  onChange: (checked: boolean) => void
  label: string
  disabled?: boolean
  testId?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      data-testid={testId}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50"
      style={{ background: checked ? 'var(--cp-accent)' : 'var(--cp-border)' }}
    >
      <span
        className="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform"
        style={{ transform: checked ? 'translateX(20px)' : 'translateX(0)' }}
      />
    </button>
  )
}
