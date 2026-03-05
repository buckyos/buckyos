type ProgressRingProps = {
  progressPercent: number | null
  size?: number
  strokeWidth?: number
}

const ProgressRing = ({ progressPercent, size = 68, strokeWidth = 6 }: ProgressRingProps) => {
  const radius = (size - strokeWidth) / 2
  const circumference = 2 * Math.PI * radius
  const clampedPercent = Math.max(0, Math.min(progressPercent ?? 0, 100))
  const dashOffset = circumference - (clampedPercent / 100) * circumference

  return (
    <div className="relative inline-flex items-center justify-center" style={{ width: size, height: size }}>
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className="-rotate-90"
        aria-hidden
      >
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="rgb(226 232 240)"
          strokeWidth={strokeWidth}
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="rgb(15 118 110)"
          strokeLinecap="round"
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={dashOffset}
          style={{ transition: 'stroke-dashoffset 180ms ease' }}
        />
      </svg>
      <span className="absolute text-[11px] font-semibold text-slate-700">
        {`${clampedPercent}%`}
      </span>
    </div>
  )
}

export default ProgressRing
