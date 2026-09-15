import { useEffect, useRef, useState } from 'react'

/**
 * Runs the latest `effect` once `key` has stayed unchanged for `delayMs`.
 * A pending run is dropped when `key` changes again or the component unmounts.
 */
export function useDebouncedEffect(key: string, delayMs: number, effect: () => void): void {
  const effectRef = useRef(effect)
  useEffect(() => {
    effectRef.current = effect
  })
  useEffect(() => {
    const timer = window.setTimeout(() => effectRef.current(), delayMs)
    return () => window.clearTimeout(timer)
  }, [delayMs, key])
}

/** Returns a copy of `value` that only updates once it has stayed unchanged for `delayMs`. */
export function useDebouncedValue(value: string, delayMs: number): string {
  const [debounced, setDebounced] = useState(value)
  useDebouncedEffect(value, delayMs, () => setDebounced(value))
  return debounced
}
