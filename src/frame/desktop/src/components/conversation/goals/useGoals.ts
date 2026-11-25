import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { onZoneRecovered } from '../../../api/zone_link'
import { loadConversationBackend } from '../backend'
import type { MessageObject } from '../model/msgobj'
import type { WorkCard } from '../worksessions/model'
import { budgetHold, isLiveGoal, type Goal, type GoalActivation, type GoalPost, type GoalSchedule, type GoalState, type OpenActivation } from './model'
import { goalPosts, rememberGoalPost, subscribeGoalPosts } from './posts'
import { getGoalSource } from './source'

const LIST_POLL_MS = 30_000
const VIEW_POLL_MS = 15_000
/** While an Activation runs, or something said to the Goal was not read yet. */
const BUSY_POLL_MS = 5_000
/** What was said to a Goal and not read for longer than this is not expected any moment. */
const POST_WAIT_MS = 10 * 60_000
const ACTIVATIONS_PAGE = 30

/**
 * Runs `read` at once and again after the time it resolves to (null: nothing is expected to change), while `key` is
 * set and the page is visible; a page shown again reads at once. The returned function reads now.
 */
function usePolled(key: string | null, read: () => Promise<number | null>): () => Promise<void> {
  const reader = useRef(read)
  useEffect(() => { reader.current = read })
  const run = useRef<() => Promise<void>>(async () => undefined)
  useEffect(() => {
    if (key === null) return
    let stopped = false, again = false
    let reading: Promise<void> | null = null
    let timer: ReturnType<typeof setTimeout> | undefined
    const once = async () => {
      clearTimeout(timer)
      const delay = await reader.current().catch(() => VIEW_POLL_MS)
      if (!stopped && delay !== null && !again && document.visibilityState === 'visible') timer = setTimeout(() => void start(), delay)
    }
    const start = (): Promise<void> => {
      if (reading) { again = true; return reading.then(() => reading ?? undefined) }
      reading = once().finally(() => {
        reading = null
        if (again && !stopped) { again = false; void start() }
      })
      return reading
    }
    run.current = start
    const shown = () => { if (document.visibilityState === 'visible') void start(); else clearTimeout(timer) }
    // Deferred, so a mount that is undone at once (React's development double mount) reads nothing.
    timer = setTimeout(() => void start(), 0)
    const stopRecovered = onZoneRecovered(() => void start())
    document.addEventListener('visibilitychange', shown)
    return () => { stopped = true; run.current = async () => undefined; clearTimeout(timer); stopRecovered(); document.removeEventListener('visibilitychange', shown) }
  }, [key])
  return useCallback(() => run.current(), [])
}

export interface GoalListView {
  /** `unavailable`: no read has succeeded (not the owner, the Agent is offline). */
  status: 'loading' | 'ready' | 'unavailable'
  goals: Goal[]
  /** Of the goals that were looked at; null: it has none yet. */
  states: Record<string, GoalState | null>
  switchedOff: Record<string, boolean>
}

/** The Goals of an Agent with their states, kept current while `active`; `ended`: those that ended are looked at too. */
export function useGoalList(agentDid: string, active: boolean, ended: boolean): GoalListView & { refresh: () => void } {
  const [view, setView] = useState<GoalListView>({ status: 'loading', goals: [], states: {}, switchedOff: {} })
  const read = async () => {
    const source = getGoalSource()
    try {
      const goals = await source.list(agentDid)
      const looked = goals.filter(goal => ended || isLiveGoal(goal))
      const found = await Promise.all(looked.map(async goal => [
        goal.goal_id,
        await source.state(agentDid, goal.goal_id).catch(() => undefined),
        goal.builtin && isLiveGoal(goal) ? (await source.schedule(agentDid, goal.goal_id).catch(() => null))?.switched_off === true : false,
      ] as const))
      setView(previous => ({
        status: 'ready',
        goals,
        states: { ...previous.states, ...Object.fromEntries(found.filter(([, state]) => state !== undefined).map(([id, state]) => [id, state ?? null])) },
        switchedOff: Object.fromEntries(found.map(([id, , off]) => [id, off])),
      }))
      return goals.some(isLiveGoal) ? LIST_POLL_MS : null
    } catch {
      setView(previous => previous.status === 'ready' ? previous : { ...previous, status: 'unavailable' })
      return LIST_POLL_MS
    }
  }
  const refresh = usePolled(active ? `${agentDid}\n${ended}` : null, read)
  return { ...view, refresh: () => { void refresh() } }
}

export interface GoalView {
  /** `missing`: the Agent has no such Goal. */
  status: 'loading' | 'ready' | 'unavailable' | 'missing'
  goal: Goal | null
  state: GoalState | null
  schedule: GoalSchedule | null
  activations: GoalActivation[]
  hasOlder: boolean
  open: OpenActivation | null
  /** Cards of the Work of the current epoch, by session. */
  works: Record<string, WorkCard>
  notices: MessageObject[] | null
}

const EMPTY_VIEW: GoalView = { status: 'loading', goal: null, state: null, schedule: null, activations: [], hasOlder: false, open: null, works: {}, notices: null }

export interface GoalConversationState extends GoalView {
  posts: readonly GoalPost[]
  loadOlder: () => Promise<boolean>
  post: (text: string) => Promise<void>
  retry: () => void
}

/** One Goal with its Activations, Work and notifications, kept current while `active`. */
export function useGoal(agentDid: string, goalId: string, active: boolean): GoalConversationState {
  const [view, setView] = useState<GoalView>(EMPTY_VIEW)
  const posts = useSyncExternalStore(subscribeGoalPosts, () => goalPosts(agentDid, goalId))
  const limit = useRef(ACTIVATIONS_PAGE)
  const last = useRef<{ activations?: { key: string; items: GoalActivation[] }; works?: { key: string; complete: boolean; cards: Record<string, WorkCard> } }>({})
  const read = async (): Promise<number | null> => {
    const source = getGoalSource()
    let goal: Goal | null
    try {
      goal = await source.get(agentDid, goalId)
    } catch {
      setView(previous => previous.status === 'ready' ? previous : { ...previous, status: 'unavailable' })
      return VIEW_POLL_MS
    }
    if (!goal) { setView({ ...EMPTY_VIEW, status: 'missing' }); return null }
    const live = isLiveGoal(goal)
    const { sid, epoch } = goal.session
    const [state, schedule, open, notices] = await Promise.all([
      source.state(agentDid, goalId).catch(() => undefined),
      source.schedule(agentDid, goalId).catch(() => undefined),
      live ? source.open(agentDid, sid).catch(() => null) : null,
      loadConversationBackend().then(backend => backend.goalNotices(agentDid, goalId)).catch(() => undefined),
    ])
    // The records only change when an Activation ends.
    const ended = state?.last_activation
    const key = `${limit.current}:${ended ? `${ended.epoch}.${ended.seq}` : ''}`
    let activations = last.current.activations
    if (activations?.key !== key) {
      const items = await source.activations(agentDid, goalId, limit.current).catch(() => undefined)
      if (items) {
        activations = { key, items }
        const newest = items.at(-1)
        // The state is published before the record: read again until it is there.
        if (!ended || (newest?.epoch === ended.epoch && newest.seq === ended.seq)) last.current.activations = activations
      }
    }
    const items = activations?.items ?? []
    let works = last.current.works
    if (items.some(item => item.epoch === epoch && item.works_created.length > 0) || (state?.active_works.length ?? 0) > 0) {
      const worksKey = `${sid}:${key}:${state?.active_works.join() ?? ''}`
      if (works?.key !== worksKey || !works.complete) {
        const tree = await source.works(agentDid, sid).catch(() => undefined)
        if (tree) works = last.current.works = { key: worksKey, complete: tree.complete, cards: Object.fromEntries(tree.sessions.map(card => [card.session_id, card])) }
      }
    }
    const running = open && !items.some(item => item.epoch === open.epoch && item.seq === open.seq) ? open : null
    setView(previous => ({
      status: 'ready',
      goal,
      state: state === undefined ? previous.state : state,
      schedule: schedule === undefined ? previous.schedule : schedule,
      activations: items,
      hasOlder: items.length >= limit.current,
      open: running,
      works: works?.cards ?? {},
      notices: notices ?? previous.notices,
    }))
    if (!live) return null
    const seen = new Set([...items, ...(running ? [running] : [])].flatMap(item => item.reasons.map(reason => reason.key)))
    const now = Date.now()
    const waiting = goal.status === 'active' && !budgetHold(state, now) && goalPosts(agentDid, goalId).some(post => post.sid === sid && now - post.at_ms < POST_WAIT_MS && !seen.has(post.key))
    return running || waiting ? BUSY_POLL_MS : VIEW_POLL_MS
  }
  const refresh = usePolled(active ? `${agentDid}\n${goalId}` : null, read)
  return {
    ...view,
    posts,
    loadOlder: async () => {
      limit.current += ACTIVATIONS_PAGE
      await refresh()
      return true
    },
    post: async (text) => {
      const sid = view.goal?.session.sid ?? ''
      const key = await getGoalSource().post(agentDid, goalId, text)
      rememberGoalPost(agentDid, goalId, { key, sid, text, at_ms: Date.now() })
      void refresh()
    },
    retry: () => { void refresh() },
  }
}
