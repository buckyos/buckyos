import type { Camera } from '../domain/types'

export interface CanvasController {
  viewport(): { width: number; height: number }
  camera(): Camera
  animateTo(camera: Camera, ms?: number): void
  fitAll(): void
  fitBlocks(ids: string[], padding?: number): void
  /** canvas coordinates of the viewport centre */
  center(): { x: number; y: number }
  /** client (screen) coordinates → canvas coordinates */
  toCanvas(clientX: number, clientY: number): { x: number; y: number }
  zoomAtCenter(factor: number): void
}
