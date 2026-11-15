import { useSyncExternalStore } from 'react'
import { useAgentBrowserStore } from '../store/context'
import { reflowControllerOf, type ReflowController, type ReflowSnapshot } from './controller'

export function useReflowController(): ReflowController {
  return reflowControllerOf(useAgentBrowserStore())
}

/** The part of the reflow state a component shows; it re-renders when that part changes. */
export function useReflow<T>(select: (snapshot: ReflowSnapshot) => T): T {
  const controller = useReflowController()
  return useSyncExternalStore(controller.subscribe, () => select(controller.getSnapshot()))
}

/** A moment the way it is shown beside a page: date and time to the minute. */
export function formatMoment(ms: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(ms))
}
