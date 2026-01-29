import type { LineSeries, LineSvgProps, SliceTooltipProps } from '@nivo/line'
import { ResponsiveLine } from '@nivo/line'

import chartTheme from './theme'
import usePrefersReducedMotion from './usePrefersReducedMotion'

type ChartSeries = LineSeries

type LineAreaChartProps = {
  data: ChartSeries[]
  height?: number
  colors?: string[]
  axisBottom?: LineSvgProps<ChartSeries>['axisBottom']
  axisLeft?: LineSvgProps<ChartSeries>['axisLeft']
  yScaleMin?: number | 'auto'
  yScaleMax?: number | 'auto'
  enablePoints?: boolean
  enableArea?: boolean
  areaOpacity?: number
  defs?: LineSvgProps<ChartSeries>['defs']
  fill?: LineSvgProps<ChartSeries>['fill']
  valueFormatter?: (value: number) => string
}

const LineAreaChart = ({
  data,
  height = 240,
  colors,
  axisBottom,
  axisLeft,
  yScaleMin = 0,
  yScaleMax = 'auto',
  enablePoints = false,
  enableArea = true,
  areaOpacity = 0.12,
  defs,
  fill,
  valueFormatter,
}: LineAreaChartProps) => {
  const prefersReducedMotion = usePrefersReducedMotion()
  const formatValue = (value: number) => {
    if (!Number.isFinite(value)) {
      return '—'
    }
    return valueFormatter ? valueFormatter(value) : Math.round(value).toString()
  }

  const sliceTooltip = ({ slice }: SliceTooltipProps<ChartSeries>) => (
    <div>
      <div className="text-xs font-semibold text-[var(--cp-ink)]">
        {slice.points[0]?.data.xFormatted ?? slice.points[0]?.data.x}
      </div>
      <div className="mt-1 space-y-1">
        {slice.points.map((point) => (
          <div key={point.id} className="flex items-center justify-between gap-3 text-xs">
            <span className="flex items-center gap-2">
              <span
                className="inline-flex size-2 rounded-full"
              style={{ backgroundColor: point.seriesColor }}
            />
            <span>{point.seriesId}</span>
            </span>
            <span className="font-semibold text-[var(--cp-ink)]">
              {formatValue(Number(point.data.y))}
            </span>
          </div>
        ))}
      </div>
    </div>
  )

  return (
    <div style={{ height }}>
      <ResponsiveLine
        data={data}
        theme={chartTheme}
        colors={colors}
        margin={{ top: 16, right: 16, bottom: 28, left: 44 }}
        xScale={{ type: 'point' }}
        yScale={{ type: 'linear', min: yScaleMin, max: yScaleMax, stacked: false }}
        curve="monotoneX"
        enableArea={enableArea}
        areaOpacity={areaOpacity}
        enablePoints={enablePoints}
        pointSize={6}
        pointBorderWidth={2}
        pointBorderColor={{ from: 'serieColor' }}
        enableSlices="x"
        axisBottom={axisBottom}
        axisLeft={axisLeft}
        enableGridX={false}
        gridYValues={5}
        useMesh
        animate={!prefersReducedMotion}
        motionConfig="gentle"
        defs={defs}
        fill={fill}
        sliceTooltip={sliceTooltip}
      />
    </div>
  )
}

export default LineAreaChart
