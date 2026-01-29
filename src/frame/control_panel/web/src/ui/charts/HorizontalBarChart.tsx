import type { BarDatum, BarSvgProps } from '@nivo/bar'
import { ResponsiveBar } from '@nivo/bar'

import chartTheme from './theme'
import usePrefersReducedMotion from './usePrefersReducedMotion'

type HorizontalBarChartProps<T extends BarDatum> = {
  data: T[]
  keys: string[]
  indexBy: string
  height?: number
  maxValue?: number
  colors?: string[]
  axisBottom?: BarSvgProps<T>['axisBottom']
  axisLeft?: BarSvgProps<T>['axisLeft']
}

const HorizontalBarChart = <T extends BarDatum>({
  data,
  keys,
  indexBy,
  height = 220,
  maxValue,
  colors,
  axisBottom,
  axisLeft,
}: HorizontalBarChartProps<T>) => {
  const prefersReducedMotion = usePrefersReducedMotion()

  return (
    <div style={{ height }}>
      <ResponsiveBar
        data={data}
        keys={keys}
        indexBy={indexBy}
        layout="horizontal"
        margin={{ top: 8, right: 16, bottom: 28, left: 120 }}
        padding={0.3}
        valueScale={{ type: 'linear', min: 0, max: maxValue ?? 'auto' }}
        indexScale={{ type: 'band', round: true }}
        theme={chartTheme}
        colors={colors}
        enableLabel={false}
        axisTop={null}
        axisRight={null}
        axisBottom={axisBottom}
        axisLeft={axisLeft}
        enableGridX={false}
        enableGridY={false}
        animate={!prefersReducedMotion}
        motionConfig="gentle"
      />
    </div>
  )
}

export default HorizontalBarChart
