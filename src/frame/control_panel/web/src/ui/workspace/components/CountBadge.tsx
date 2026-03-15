import Icon from '../../icons'

type CountBadgeProps = {
  icon: IconName
  label: string
  count: number
  onClick?: () => void
}

const CountBadge = ({ icon, label, count, onClick }: CountBadgeProps) => {
  const Tag = onClick ? 'button' : 'span'
  return (
    <Tag
      type={onClick ? 'button' : undefined}
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-2.5 py-1 text-[11px] font-semibold text-[var(--cp-muted)] transition ${
        onClick ? 'cursor-pointer hover:bg-[var(--cp-primary-soft)] hover:text-[var(--cp-primary-strong)]' : ''
      }`}
      title={label}
    >
      <Icon name={icon} className="size-3" />
      <span>{count}</span>
    </Tag>
  )
}

export default CountBadge
