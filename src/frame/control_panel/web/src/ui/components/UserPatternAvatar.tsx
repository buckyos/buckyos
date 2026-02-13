type UserPatternAvatarProps = {
  name: string
  className?: string
}

const palettes = [
  { bg: '#dbeafe', fg: '#1d4ed8', accent: '#93c5fd' },
  { bg: '#dcfce7', fg: '#166534', accent: '#86efac' },
  { bg: '#fef3c7', fg: '#92400e', accent: '#fcd34d' },
  { bg: '#fae8ff', fg: '#7e22ce', accent: '#d8b4fe' },
  { bg: '#ffedd5', fg: '#9a3412', accent: '#fdba74' },
  { bg: '#cffafe', fg: '#155e75', accent: '#67e8f9' },
  { bg: '#e0f2fe', fg: '#0369a1', accent: '#7dd3fc' },
  { bg: '#e2e8f0', fg: '#1e293b', accent: '#94a3b8' },
]

const hashString = (value: string) => {
  let hash = 2166136261
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

const buildPattern = (seed: number) => {
  const grid: boolean[][] = []
  for (let row = 0; row < 5; row += 1) {
    const left: boolean[] = []
    for (let col = 0; col < 3; col += 1) {
      const bitIndex = row * 3 + col
      left.push(((seed >> bitIndex) & 1) === 1)
    }
    grid.push([left[0], left[1], left[2], left[1], left[0]])
  }
  return grid
}

const getNameSlice = (name: string) => {
  const trimmed = name.trim()
  if (!trimmed) {
    return 'U'
  }

  const words = trimmed.split(/\s+/).filter(Boolean)
  if (words.length >= 2) {
    const first = Array.from(words[0])[0] ?? ''
    const second = Array.from(words[1])[0] ?? ''
    return `${first}${second}`.toUpperCase()
  }

  return Array.from(trimmed).slice(0, 2).join('').toUpperCase()
}

const UserPatternAvatar = ({ name, className }: UserPatternAvatarProps) => {
  const normalizedName = name.trim().toLowerCase() || 'user'
  const seed = hashString(normalizedName)
  const palette = palettes[seed % palettes.length]
  const pattern = buildPattern(seed)
  const shapeMode = seed % 3
  const nameSlice = getNameSlice(name)
  const classes = ['overflow-hidden rounded-full border border-white/15', className].filter(Boolean).join(' ')

  return (
    <div className={classes} aria-hidden>
      <svg viewBox="0 0 100 100" className="size-full">
        <rect x="0" y="0" width="100" height="100" fill={palette.bg} />
        <circle cx="82" cy="18" r="20" fill={palette.accent} opacity="0.35" />
        <g transform="translate(10 10)">
          {pattern.map((row, rowIndex) =>
            row.map((enabled, colIndex) => {
              if (!enabled) {
                return null
              }

              const x = colIndex * 16
              const y = rowIndex * 16
              if (shapeMode === 0) {
                return <rect key={`${rowIndex}-${colIndex}`} x={x} y={y} width="12" height="12" rx="2" fill={palette.fg} />
              }

              if (shapeMode === 1) {
                return <circle key={`${rowIndex}-${colIndex}`} cx={x + 6} cy={y + 6} r="5.5" fill={palette.fg} />
              }

              return (
                <rect
                  key={`${rowIndex}-${colIndex}`}
                  x={x}
                  y={y}
                  width="12"
                  height="12"
                  rx="4"
                  fill={palette.fg}
                />
              )
            }),
          )}
        </g>
        <text
          x="50"
          y="89"
          textAnchor="middle"
          fontSize="23"
          fontWeight="700"
          fill={palette.fg}
          opacity="0.88"
          fontFamily="'Space Grotesk', 'Work Sans', sans-serif"
        >
          {nameSlice}
        </text>
      </svg>
    </div>
  )
}

export default UserPatternAvatar
