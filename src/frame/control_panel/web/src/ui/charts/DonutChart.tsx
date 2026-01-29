import type { PieSvgProps } from '@nivo/pie'
import { ResponsivePie } from '@nivo/pie'

import chartTheme from './theme'
import usePrefersReducedMotion from './usePrefersReducedMotion'

type DonutDatum = {
  id: string
  label?: string
  value: number
  color?: string
}

type DonutChartProps = {
  data: DonutDatum[]
  height?: number
  innerRadius?: number
}

const DonutChart = ({ data, height = 220, innerRadius = 0.7 }: DonutChartProps) => {
  const prefersReducedMotion = usePrefersReducedMotion()

  const tooltip: PieSvgProps<DonutDatum>['tooltip'] = ({ datum }) => (
    <div className="text-xs">
      <div className="text-[var(--cp-ink)]">{datum.label ?? datum.id}</div>
      <div className="mt-1 font-semibold text-[var(--cp-ink)]">
        {Math.round(datum.value)}%
      </div>
    </div>
  )

  return (
    <div style={{ height }}>
      <ResponsivePie
        data={data}
        theme={chartTheme}
        margin={{ top: 12, right: 12, bottom: 12, left: 12 }}
        innerRadius={innerRadius}
        padAngle={1.5}
        cornerRadius={6}
        colors={{ datum: 'data.color' }}
        activeOuterRadiusOffset={6}
        activeInnerRadiusOffset={2}
        enableArcLabels={false}
        enableArcLinkLabels={false}
        tooltip={tooltip}
        animate={!prefersReducedMotion}
        motionConfig="gentle"
      />
    </div>
  )
}

export type { DonutDatum }
export default DonutChart
