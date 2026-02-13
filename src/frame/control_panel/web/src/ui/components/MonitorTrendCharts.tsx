import LineAreaChart from '../charts/LineAreaChart'

type ResourceTrendChartProps = {
  timeline: ResourcePoint[]
  height?: number
}

type NetworkTrendChartProps = {
  timeline: NetworkPoint[]
  height?: number
}

export const ResourceTrendChart = ({ timeline, height = 220 }: ResourceTrendChartProps) => {
  const points = timeline.length
    ? timeline
    : [{ time: 'now', cpu: 0, memory: 0 }]

  const data = [
    {
      id: 'CPU',
      data: points.map((point) => ({ x: point.time, y: point.cpu })),
    },
    {
      id: 'Memory',
      data: points.map((point) => ({ x: point.time, y: point.memory })),
    },
  ]

  return (
    <LineAreaChart
      data={data}
      height={height}
      colors={['var(--cp-primary)', 'var(--cp-accent)']}
      axisBottom={{ tickSize: 0, tickPadding: 8 }}
      axisLeft={{ tickSize: 0, tickPadding: 8, tickValues: [0, 25, 50, 75, 100] }}
      yScaleMin={0}
      yScaleMax={100}
      valueFormatter={(value) => `${Math.round(value)}%`}
    />
  )
}

export const NetworkTrendChart = ({ timeline, height = 200 }: NetworkTrendChartProps) => {
  const points = timeline.length
    ? timeline
    : [{ time: 'now', rx: 0, tx: 0 }]

  const maxValue = Math.max(
    1,
    ...points.map((point) => Math.max(point.rx, point.tx)),
  )
  const scaleMax = Math.max(5, Math.ceil(maxValue / 1024 / 1024 / 5) * 5)

  const data = [
    {
      id: 'Download',
      data: points.map((point) => ({ x: point.time, y: point.rx / 1024 / 1024 })),
    },
    {
      id: 'Upload',
      data: points.map((point) => ({ x: point.time, y: point.tx / 1024 / 1024 })),
    },
  ]

  return (
    <LineAreaChart
      data={data}
      height={height}
      colors={['var(--cp-primary)', 'var(--cp-accent)']}
      axisBottom={{ tickSize: 0, tickPadding: 8 }}
      axisLeft={{ tickSize: 0, tickPadding: 8, tickValues: [0, scaleMax / 2, scaleMax] }}
      yScaleMin={0}
      yScaleMax={scaleMax}
      valueFormatter={(value) => `${value.toFixed(2)} MB/s`}
    />
  )
}
