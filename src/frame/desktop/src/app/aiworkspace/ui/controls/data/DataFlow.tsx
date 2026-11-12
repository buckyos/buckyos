/* A data control on a flow page or on a page of its own (控件机制 TODO G14): its body at the page's width, always taking
 * input (there is no canvas selection to go through first). */

import { useContext, useLayoutEffect, useRef, useState } from 'react'
import { num } from '@controls/placement'
import type { CanvasMode } from '../../blocks/registry'
import { usePointerType } from '../../blocks/useBlockContext'
import { useEntity } from '../../../state/hooks'
import { ShellContext } from '../../shell/shellContext'
import { controlRegistry } from '../registry'

export function DataFlow({ containerId, mode, height, host = 'flow' }: { containerId: string; mode: CanvasMode; height?: number; host?: 'flow' | 'page' }) {
  const container = useEntity(containerId)
  const def = controlRegistry.get(container?.kind)
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 640, h: height ?? num(container?.placement?.h) ?? 480 })
  const pointerType = usePointerType()
  const phone = useContext(ShellContext)?.phone ?? false
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const update = () => setSize({ w: el.clientWidth, h: height ?? el.clientHeight })
    update()
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => observer.disconnect()
  }, [height])
  const Body = def?.Body
  return (
    <div ref={ref} className={`aiws-data-flow is-${host}`} style={height ? { height } : undefined} data-testid={`aiws-data-flow-${containerId}`}>
      {Body && <Body containerId={containerId} mode={mode} active host={host} size={size} zoom={1} pointerType={pointerType} phone={phone} />}
    </div>
  )
}
