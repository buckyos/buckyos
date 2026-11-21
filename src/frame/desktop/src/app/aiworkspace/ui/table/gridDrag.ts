import type { PointerEvent as ReactPointerEvent } from 'react'

export function trackGridDrag(event: ReactPointerEvent<HTMLElement>, move: (event: PointerEvent) => void, end: (cancelled: boolean) => void) {
  if (!event.isPrimary || event.button !== 0) return
  const target = event.currentTarget
  const pointer = event.pointerId
  event.stopPropagation()
  event.preventDefault()
  target.setPointerCapture(pointer)
  const update = (next: PointerEvent) => { if (next.pointerId === pointer) move(next) }
  const finish = (next: PointerEvent) => {
    if (next.pointerId !== pointer) return
    cleanup()
    end(next.type !== 'pointerup')
  }
  const interrupt = (next: PointerEvent) => {
    if (next.pointerId === pointer || next.pointerType !== 'touch') return
    cancel()
  }
  const cancel = () => { cleanup(); end(true) }
  const cleanup = () => {
    window.removeEventListener('pointermove', update, true)
    window.removeEventListener('pointerup', finish, true)
    window.removeEventListener('pointercancel', finish, true)
    window.removeEventListener('lostpointercapture', finish, true)
    window.removeEventListener('pointerdown', interrupt, true)
    window.removeEventListener('blur', cancel)
    if (target.hasPointerCapture(pointer)) target.releasePointerCapture(pointer)
  }
  window.addEventListener('pointermove', update, true)
  window.addEventListener('pointerup', finish, true)
  window.addEventListener('pointercancel', finish, true)
  window.addEventListener('lostpointercapture', finish, true)
  window.addEventListener('pointerdown', interrupt, true)
  window.addEventListener('blur', cancel)
}
