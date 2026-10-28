/* Node text measured with the browser's own fonts (脑图方案 §5.4): the shared measurer with canvas metrics, one per
 * window, so the layout and the node Blocks agree on the lines and sizes. */

import { approxMetrics, NodeMeasurer, type TextMetrics } from '@controls/measure'

function canvasMetrics(): TextMetrics {
  let ctx: OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D | null | undefined
  return {
    width(text, font) {
      if (ctx === undefined) {
        try { ctx = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(1, 1).getContext('2d') : document.createElement('canvas').getContext('2d') } catch { ctx = null }
      }
      if (!ctx) return approxMetrics.width(text, font)
      if (ctx.font !== font) ctx.font = font
      return ctx.measureText(text).width
    },
  }
}

export const nodeMeasurer = new NodeMeasurer(canvasMetrics())
