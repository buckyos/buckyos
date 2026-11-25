import type { CDPSession, Page } from '@playwright/test'

interface Point { x: number; y: number }

/** Fingers on the screen, in page coordinates (one touch event per call, like a real digitizer). */
export class Fingers {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) { return new Fingers(await page.context().newCDPSession(page)) }
  down(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  move(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  up() { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] }) }
  async tap(p: Point) { await this.down(p); await this.up() }
  async drag(page: Page, from: Point[], to: Point[], steps = 10) {
    await this.down(...from)
    for (let i = 1; i <= steps; i += 1) {
      await this.move(...from.map((f, n) => ({ x: f.x + ((to[n].x - f.x) * i) / steps, y: f.y + ((to[n].y - f.y) * i) / steps })))
      await page.waitForTimeout(16)
    }
    await page.waitForTimeout(150)
    await this.up()
  }
}
