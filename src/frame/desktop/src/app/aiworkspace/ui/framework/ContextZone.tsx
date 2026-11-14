/* The context zone (UI 框架 PRD §4.4): the bottom-centre place of the bars that come with a situation — writing (the
 * ink bar), a show or a public session (their controls). One bar at a time; nothing when no situation is active.
 * Every bar in it gets the same position, size and avoidance: centred at the bottom, never wrapping (a bar too wide
 * scrolls sideways), clear of the left tool zone, and lifted above the status bar when the two would meet — and above
 * the soft keyboard while one is up (a phone, §4.4: the visual viewport ends above it). */

import { useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { useAiwsText } from '../../useAiwsText'
import './framework.css'

/** The gap kept between the zone and the status bar it avoids. */
const GAP = 8

export function ContextZone({ children, testId = 'aiws-context-zone' }: { children: ReactNode; testId?: string }) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [lift, setLift] = useState(0)
  const [keyboard, setKeyboard] = useState(0)
  useLayoutEffect(() => {
    const vv = window.visualViewport
    const area = ref.current?.parentElement
    if (!vv || !area) return
    // a soft keyboard comes with touch (on a computer a smaller visual viewport is a zoom, not a keyboard)
    const touch = window.matchMedia('(pointer: coarse)')
    const follow = () => setKeyboard(touch.matches ? Math.max(0, Math.round(area.getBoundingClientRect().bottom - (vv.offsetTop + vv.height))) : 0)
    follow()
    vv.addEventListener('resize', follow)
    vv.addEventListener('scroll', follow)
    return () => { vv.removeEventListener('resize', follow); vv.removeEventListener('scroll', follow) }
  }, [])
  useLayoutEffect(() => {
    const el = ref.current
    const area = el?.parentElement
    if (!el || !area) return
    const place = () => {
      const status = area.querySelector<HTMLElement>(':scope > .aiws-status-dock > .aiws-status-row, :scope > .aiws-status-dock > .aiws-status')
      if (!status) { setLift(0); return }
      const zone = el.getBoundingClientRect()
      const bar = status.getBoundingClientRect()
      const meets = zone.right + GAP > bar.left && zone.left - GAP < bar.right
      setLift(meets ? Math.max(0, area.getBoundingClientRect().bottom - bar.top) : 0)
    }
    place()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(place)
    observer.observe(el)
    observer.observe(area)
    const status = area.querySelector(':scope > .aiws-status-dock')
    if (status) observer.observe(status)
    return () => observer.disconnect()
  }, [])
  return (
    <div ref={ref} className="aiws-context-zone" role="region" aria-label={t('aiworkspace.framework.zone.context')} data-testid={testId} data-lifted={lift > 0 || keyboard > 0 ? 'true' : undefined}
      data-keyboard={keyboard > 0 ? 'true' : undefined} style={lift > 0 || keyboard > 0 ? { bottom: Math.max(lift, keyboard) + GAP } : undefined}>
      {children}
    </div>
  )
}
