import Icon from '../../icons'

type FilterOption = {
  value: string
  label: string
}

type FilterDropdownProps = {
  label: string
  value: string
  options: FilterOption[]
  onChange: (value: string) => void
}

const FilterDropdown = ({ label, value, options, onChange }: FilterDropdownProps) => (
  <select
    value={value}
    onChange={(e) => onChange(e.target.value)}
    className="rounded-full border border-[var(--cp-border)] bg-white px-3 py-1.5 text-xs font-medium text-[var(--cp-ink)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
    title={label}
  >
    <option value="">{label}: All</option>
    {options.map((opt) => (
      <option key={opt.value} value={opt.value}>
        {opt.label}
      </option>
    ))}
  </select>
)

type FilterBarProps = {
  keyword: string
  onKeywordChange: (value: string) => void
  dropdowns?: FilterDropdownProps[]
  placeholder?: string
}

const FilterBar = ({
  keyword,
  onKeywordChange,
  dropdowns = [],
  placeholder = 'Search...',
}: FilterBarProps) => (
  <div className="flex flex-wrap items-center gap-2">
    {dropdowns.map((dd) => (
      <FilterDropdown key={dd.label} {...dd} />
    ))}
    <div className="relative min-w-[180px] flex-1">
      <Icon
        name="search"
        className="absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-[var(--cp-muted)]"
      />
      <input
        value={keyword}
        onChange={(e) => onKeywordChange(e.target.value)}
        placeholder={placeholder}
        className="w-full rounded-full border border-[var(--cp-border)] bg-white py-1.5 pl-8 pr-3 text-xs text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
      />
    </div>
  </div>
)

export default FilterBar
export { FilterDropdown }
