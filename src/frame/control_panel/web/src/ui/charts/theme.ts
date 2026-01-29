const chartTheme = {
  background: 'transparent',
  text: {
    fontFamily: 'var(--cp-font-body)',
    fontSize: 11,
    fill: 'var(--cp-muted)',
  },
  axis: {
    domain: {
      line: {
        stroke: 'var(--cp-border)',
        strokeWidth: 1,
      },
    },
    ticks: {
      line: {
        stroke: 'var(--cp-border)',
        strokeWidth: 1,
      },
      text: {
        fill: 'var(--cp-muted)',
        fontSize: 11,
      },
    },
    legend: {
      text: {
        fill: 'var(--cp-muted)',
        fontSize: 12,
      },
    },
  },
  grid: {
    line: {
      stroke: 'var(--cp-border)',
      strokeWidth: 1,
      strokeDasharray: '4 4',
    },
  },
  tooltip: {
    container: {
      background: 'var(--cp-surface)',
      color: 'var(--cp-ink)',
      fontSize: 12,
      borderRadius: 12,
      boxShadow: '0 20px 40px -30px rgba(15, 23, 42, 0.4)',
      border: '1px solid var(--cp-border)',
      padding: '8px 10px',
    },
  },
}

export default chartTheme
