import type { GoalPost } from './model'

const STORAGE_KEY = 'buckyos.conversation.goalPosts.v1'
const KEEP_PER_GOAL = 40
const MAX_GOALS = 100

let memory: Record<string, GoalPost[]> | null = null
const listeners = new Set<() => void>()
const EMPTY: readonly GoalPost[] = []

function load(): Record<string, GoalPost[]> {
  if (memory) return memory
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}')
    memory = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, GoalPost[]> : {}
  } catch {
    memory = {}
  }
  return memory
}

const keyOf = (agentDid: string, goalId: string) => `${agentDid}\n${goalId}`

/** What was written to the Goal from this browser, oldest first: a message may wait hours for its Activation, and nothing lists the waiting ones. */
export function goalPosts(agentDid: string, goalId: string): readonly GoalPost[] {
  return load()[keyOf(agentDid, goalId)] ?? EMPTY
}

export function rememberGoalPost(agentDid: string, goalId: string, post: GoalPost) {
  const all = load()
  const key = keyOf(agentDid, goalId)
  const rest = Object.fromEntries(Object.entries(all).filter(([name]) => name !== key).slice(-(MAX_GOALS - 1)))
  memory = { ...rest, [key]: [...(all[key] ?? []).filter(item => item.key !== post.key), post].slice(-KEEP_PER_GOAL) }
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(memory)) } catch { /* kept for this page only */ }
  listeners.forEach(listener => listener())
}

export function subscribeGoalPosts(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
