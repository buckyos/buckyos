/* DOM events → the protocol's `input` fields (§3.2). */

import { MODIFIER, type MouseButton } from '../api/types'

interface ModifierKeys { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }

export function modifiersOf(event: ModifierKeys): number {
  return (event.altKey ? MODIFIER.alt : 0) | (event.ctrlKey ? MODIFIER.ctrl : 0) | (event.metaKey ? MODIFIER.meta : 0) | (event.shiftKey ? MODIFIER.shift : 0)
}

/** `MouseEvent.button` of a press or release. */
export function buttonOf(button: number): MouseButton {
  return button === 1 ? 'middle' : button === 2 ? 'right' : 'left'
}

/** `MouseEvent.buttons` while moving: the button held, if any. */
export function heldButton(buttons: number): MouseButton {
  return buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none'
}

const LINE_PX = 40

/** A wheel event's deltas in CSS pixels of the page. */
export function wheelPixels(event: Pick<WheelEvent, 'deltaX' | 'deltaY' | 'deltaMode'>, pageHeight: number): { x: number; y: number } {
  const unit = event.deltaMode === 1 ? LINE_PX : event.deltaMode === 2 ? pageHeight : 1
  return { x: event.deltaX * unit, y: event.deltaY * unit }
}

const IME_KEYS = new Set(['Unidentified', 'Process', 'Dead'])

/** A key event the input method owns: its text arrives through composition and input events instead. */
export function isImeKey(event: Pick<KeyboardEvent, 'key' | 'keyCode' | 'isComposing'>): boolean {
  return event.isComposing || event.keyCode === 229 || IME_KEYS.has(event.key)
}

/** The character a key press types, for `key.text`; empty for keys and shortcuts that type nothing. */
export function keyText(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'getModifierState'>): string {
  if ([...event.key].length !== 1 || event.metaKey) return ''
  if (event.ctrlKey && !(event.altKey || event.getModifierState('AltGraph'))) return ''
  return event.key
}

/** AltGr reports itself as Ctrl+Alt: a character typed with it is sent without those two. */
export function keyModifiers(event: ModifierKeys & Pick<KeyboardEvent, 'getModifierState'>): number {
  const modifiers = modifiersOf(event)
  return event.getModifierState('AltGraph') ? modifiers & ~(MODIFIER.alt | MODIFIER.ctrl) : modifiers
}

/** The `inputmode` whose on-screen keyboard suits the field the page focused (`focus.input_type`). */
export function inputModeFor(inputType: string | undefined): string {
  switch (inputType) {
    case 'number': return 'decimal'
    case 'tel': return 'tel'
    case 'email': return 'email'
    case 'url': return 'url'
    case 'search': return 'search'
    default: return 'text'
  }
}

/**
 * iOS raises the on-screen keyboard only for a focus made inside the user's gesture; other browsers also for one
 * made a moment later, while the gesture's activation lasts.
 */
export function keyboardNeedsGesture(): boolean {
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
}

/**
 * What the user did to the hidden input since `before`: how many characters were removed from its end and the
 * text typed after what remains. Counted in code points: one Backspace removes one of them.
 */
export function diffTyped(before: string, after: string): { deleted: number; inserted: string } {
  const a = [...before]
  const b = [...after]
  let common = 0
  while (common < a.length && common < b.length && a[common] === b[common]) common += 1
  return { deleted: a.length - common, inserted: b.slice(common).join('') }
}
