import { useEffect, useMemo, useState } from 'react'
import { NavLink, useLocation } from 'react-router-dom'

import { fetchChatBootstrap, fetchChatContacts } from '@/api'
import Icon from '../icons'
import ChatWindow from './desktop/ChatWindow'

type MessageHubSection = 'today' | 'chat' | 'people' | 'tasks' | 'agents'

type MessageHubNavItem = {
  id: MessageHubSection
  label: string
  description: string
  icon: IconName
  path: string
}

type TodayInboxItem = {
  id: string
  title: string
  summary: string
  tone: 'urgent' | 'focus' | 'watch'
  lane: 'needs-reply' | 'follow-up' | 'agent'
  ctaLabel: string
  ctaPath: string
  freshnessLabel: string
  meta: string
}

type TaskBlueprint = {
  id: string
  title: string
  owner: string
  dueLabel: string
  sourceLabel: string
  status: 'active' | 'waiting' | 'draft'
}

type AgentBlueprint = {
  id: string
  title: string
  summary: string
  priority: 'urgent' | 'standard'
  threadId: string
}

const NAV_ITEMS: MessageHubNavItem[] = [
  {
    id: 'today',
    label: 'Today',
    description: 'Priority overview and triage queue',
    icon: 'spark',
    path: '/message-hub/today',
  },
  {
    id: 'chat',
    label: 'Chat',
    description: 'Realtime direct messaging',
    icon: 'message',
    path: '/message-hub/chat',
  },
  {
    id: 'people',
    label: 'People',
    description: 'Cross-channel contact view',
    icon: 'users',
    path: '/message-hub/people',
  },
  {
    id: 'tasks',
    label: 'Tasks',
    description: 'Follow-up extraction and TODO queue',
    icon: 'todo',
    path: '/message-hub/tasks',
  },
  {
    id: 'agents',
    label: 'Agents',
    description: 'Agent inbox and RAW records',
    icon: 'agent',
    path: '/message-hub/agents',
  },
]

const inferSection = (pathname: string): MessageHubSection => {
  if (pathname.endsWith('/today')) return 'today'
  if (pathname.endsWith('/people')) return 'people'
  if (pathname.endsWith('/tasks')) return 'tasks'
  if (pathname.endsWith('/agents')) return 'agents'
  return 'chat'
}

const readErrorMessage = (error: unknown, fallback: string) =>
  error instanceof Error ? error.message : typeof error === 'string' ? error : fallback

const formatRelativeFreshness = (timestamp: number) => {
  if (!timestamp) return 'unknown'

  const diffMinutes = Math.max(0, Math.round((Date.now() - timestamp) / 60000))
  if (diffMinutes < 1) return 'just now'
  if (diffMinutes < 60) return `${diffMinutes}m ago`
  const diffHours = Math.round(diffMinutes / 60)
  if (diffHours < 24) return `${diffHours}h ago`
  const diffDays = Math.round(diffHours / 24)
  return `${diffDays}d ago`
}

const summarizeBindings = (contact: ChatContact) => {
  if (!contact.bindings.length) return 'No linked accounts yet'
  return contact.bindings
    .slice(0, 2)
    .map((binding) => binding.platform)
    .join(' + ')
}

const buildTodayInbox = (contacts: ChatContact[], sendEnabled: boolean): TodayInboxItem[] => {
  const recentContacts = [...contacts].sort((left, right) => right.updated_at - left.updated_at).slice(0, 6)

  const replyItems: TodayInboxItem[] = recentContacts.slice(0, 2).map((contact, index) => ({
    id: `reply-${contact.did}`,
    title: `${contact.name || 'Recent contact'} is waiting on a response`,
    summary: sendEnabled
      ? `Recent activity was detected ${formatRelativeFreshness(contact.updated_at)}. Jump back into the direct thread before this turns into a missed follow-up.`
      : `Recent activity was detected ${formatRelativeFreshness(contact.updated_at)}. This account is read-only now, so treat this as awareness instead of a reply task.`,
    tone: index === 0 ? 'urgent' : 'focus',
    lane: 'needs-reply' as const,
    ctaLabel: 'Open chat',
    ctaPath: '/message-hub/chat',
    freshnessLabel: formatRelativeFreshness(contact.updated_at),
    meta: contact.did,
  }))

  const taskItems: TodayInboxItem[] = recentContacts.slice(2, 4).map((contact) => ({
    id: `task-${contact.did}`,
    title: `Turn ${contact.name || 'this contact'} into a follow-up object`,
    summary: `Message Hub already knows this person and their bindings (${summarizeBindings(contact)}). The next step is extracting commitments, owners, and due dates from the thread.`,
    tone: 'focus' as const,
    lane: 'follow-up' as const,
    ctaLabel: 'Review tasks',
    ctaPath: '/message-hub/tasks',
    freshnessLabel: formatRelativeFreshness(contact.updated_at),
    meta: `${contact.bindings.length} bindings`,
  }))

  const agentItems: TodayInboxItem[] = [
    {
      id: 'agent-approval-nightly',
      title: 'Nightly sync agent wants a human approval lane',
      summary:
        'Agent-originated work should surface here with a short summary, confidence level, and a direct path to approve or inspect the raw thread.',
      tone: 'watch',
      lane: 'agent',
      ctaLabel: 'Open agents',
      ctaPath: '/message-hub/agents',
      freshnessLabel: 'planned surface',
      meta: 'thread agent-thread-42',
    },
  ]

  const fallbackItem: TodayInboxItem = {
    id: 'today-bootstrap',
    title: 'Seed the unified inbox from chat-first activity',
    summary:
      'Today should stay useful even before mail, calendar, and notifications are integrated. The current source of truth is contact freshness plus message workflow capability.',
    tone: 'watch',
    lane: 'follow-up',
    ctaLabel: 'Explore people',
    ctaPath: '/message-hub/people',
    freshnessLabel: 'foundation',
    meta: `${contacts.length} known contacts`,
  }

  return [...replyItems, ...taskItems, ...agentItems, fallbackItem].slice(0, 6)
}

const buildTaskBlueprints = (contacts: ChatContact[]): TaskBlueprint[] =>
  [...contacts]
    .sort((left, right) => right.updated_at - left.updated_at)
    .slice(0, 4)
    .map((contact, index) => ({
      id: `task-blueprint-${contact.did}`,
      title:
        index === 0
          ? `Reply to ${contact.name || 'recent contact'} and confirm next step`
          : index === 1
            ? `Capture TODOs from ${contact.name || 'recent contact'} thread`
            : `Merge ${contact.name || 'contact'} into cross-channel timeline`,
      owner: contact.name || contact.did,
      dueLabel: index === 0 ? 'Today' : index === 1 ? 'This afternoon' : 'This week',
      sourceLabel: summarizeBindings(contact),
      status: index === 0 ? 'active' : index === 1 ? 'waiting' : 'draft',
    }))

const buildAgentBlueprints = (contacts: ChatContact[]): AgentBlueprint[] => [
  {
    id: 'agent-blueprint-1',
    title: 'Daily digest planner requests inbox ranking review',
    summary: `Use recent people movement${contacts[0] ? ` around ${contacts[0].name || contacts[0].did}` : ''} to decide what should be high signal tomorrow morning.`,
    priority: 'urgent',
    threadId: 'agent-thread-42',
  },
  {
    id: 'agent-blueprint-2',
    title: 'Assistant agent proposes follow-up extraction rules',
    summary:
      'Promote phrases like "let me know", "please confirm", and explicit due dates into task candidates connected to the originating conversation.',
    priority: 'standard',
    threadId: 'agent-thread-43',
  },
]

const MessageHubPage = () => {
  const location = useLocation()
  const section = useMemo(() => inferSection(location.pathname), [location.pathname])

  const [bootstrap, setBootstrap] = useState<ChatBootstrapResponse | null>(null)
  const [contacts, setContacts] = useState<ChatContact[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    document.title = 'Message Hub'
  }, [])

  useEffect(() => {
    let cancelled = false

    const load = async () => {
      setLoading(true)
      const [{ data: bootstrapData, error: bootstrapError }, { data: contactsData, error: contactsError }] = await Promise.all([
        fetchChatBootstrap(),
        fetchChatContacts({ limit: 24 }),
      ])

      if (cancelled) return

      setBootstrap(bootstrapData)
      setContacts(contactsData?.items ?? [])

      const nextError = bootstrapError ?? contactsError
      setError(nextError ? readErrorMessage(nextError, 'Unable to load Message Hub overview.') : null)
      setLoading(false)
    }

    void load()

    return () => {
      cancelled = true
    }
  }, [])

  const activeContacts = contacts.filter((contact) => Date.now() - contact.updated_at < 1000 * 60 * 60 * 24)
  const verifiedContacts = contacts.filter((contact) => contact.is_verified)
  const sendEnabled = bootstrap?.capabilities.message_send ?? false
  const todayInbox = useMemo(() => buildTodayInbox(contacts, sendEnabled), [contacts, sendEnabled])
  const taskBlueprints = useMemo(() => buildTaskBlueprints(contacts), [contacts])
  const agentBlueprints = useMemo(() => buildAgentBlueprints(contacts), [contacts])

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top_left,_rgba(15,118,110,0.22),_transparent_34%),radial-gradient(circle_at_top_right,_rgba(245,158,11,0.12),_transparent_30%),linear-gradient(180deg,_#eef4f3_0%,_#e3ece9_100%)]">
      <div className="mx-auto grid min-h-screen max-w-[1440px] gap-6 px-4 py-5 md:grid-cols-[280px_minmax(0,1fr)] md:px-6">
        <aside className="cp-panel h-fit overflow-hidden">
          <div className="border-b border-[var(--cp-border)] px-5 py-5">
            <div className="flex items-center gap-3">
              <span className="inline-flex size-11 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-white shadow-sm">
                <Icon name="message" className="size-5" />
              </span>
              <div>
                <p className="text-xs font-semibold uppercase tracking-[0.22em] text-[var(--cp-primary-strong)]">
                  Message Hub
                </p>
                <h1 className="mt-1 text-xl font-semibold text-[var(--cp-ink)]">Communication center</h1>
              </div>
            </div>
            <p className="mt-4 text-sm leading-6 text-[var(--cp-muted)]">
              Build a unified inbox for people, agents, notifications, schedules, and follow-up work.
            </p>
          </div>

          <nav className="space-y-2 px-3 py-4">
            {NAV_ITEMS.map((item) => (
              <NavLink
                key={item.id}
                to={item.path}
                className={({ isActive }) =>
                  `flex items-start gap-3 rounded-2xl px-4 py-3 transition ${
                    isActive
                      ? 'bg-[var(--cp-primary)] text-white shadow-sm'
                      : 'text-[var(--cp-ink)] hover:bg-[var(--cp-surface-muted)]'
                  }`
                }
              >
                {({ isActive }) => (
                  <>
                    <span
                      className={`mt-0.5 inline-flex size-9 items-center justify-center rounded-2xl ${
                        isActive ? 'bg-white/15 text-white' : 'bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]'
                      }`}
                    >
                      <Icon name={item.icon} className="size-4" />
                    </span>
                    <span className="min-w-0">
                      <span className="block text-sm font-semibold">{item.label}</span>
                      <span className={`mt-1 block text-xs leading-5 ${isActive ? 'text-white/75' : 'text-[var(--cp-muted)]'}`}>
                        {item.description}
                      </span>
                    </span>
                  </>
                )}
              </NavLink>
            ))}
          </nav>

          <div className="border-t border-[var(--cp-border)] px-5 py-4">
            <div className="rounded-2xl bg-[var(--cp-surface-muted)] p-4">
              <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Current scope</p>
              <div className="mt-3 space-y-2 text-sm text-[var(--cp-ink)]">
                <p>Realtime chat is live now.</p>
                <p>Email, calendar, TODO extraction, and RAW agent records are next.</p>
              </div>
            </div>
          </div>
        </aside>

        <main className="min-w-0">
          <section className="cp-panel overflow-hidden">
            <div className="border-b border-[var(--cp-border)] px-6 py-5">
              <div className="flex flex-wrap items-start justify-between gap-4">
                <div>
                  <p className="text-sm font-semibold uppercase tracking-[0.22em] text-[var(--cp-primary-strong)]">
                    {NAV_ITEMS.find((item) => item.id === section)?.label}
                  </p>
                  <h2 className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">
                    {section === 'today' && 'What needs attention today'}
                    {section === 'chat' && 'Realtime communication workspace'}
                    {section === 'people' && 'People across channels'}
                    {section === 'tasks' && 'Follow-up and extracted action queue'}
                    {section === 'agents' && 'Agent inbox and raw traces'}
                  </h2>
                  <p className="mt-3 max-w-3xl text-sm leading-6 text-[var(--cp-muted)]">
                    {section === 'today' &&
                      'Use this page as the launchpad for high-signal work, unresolved replies, and the next surfaces Message Hub should absorb.'}
                    {section === 'chat' &&
                      'The current live slice stays chat-first while the surrounding app grows into the larger information center.'}
                    {section === 'people' &&
                      'Contacts become the stable entity that unifies messages, email, notifications, and schedule context.'}
                    {section === 'tasks' &&
                      'Message Hub should turn conversations into follow-up objects, deadlines, and lightweight task queues.'}
                    {section === 'agents' &&
                      'This surface is where human-agent and agent-agent communication eventually lands for approval, replay, and debugging.'}
                  </p>
                </div>

                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <MetricCard label="Contacts" value={String(contacts.length)} note={loading ? 'Loading' : `${verifiedContacts.length} verified`} />
                  <MetricCard label="Active today" value={String(activeContacts.length)} note={loading ? 'Loading' : 'Recent contact updates'} />
                  <MetricCard label="Replies" value={sendEnabled ? 'Enabled' : 'Read only'} note={bootstrap?.scope.access_mode ?? 'scope'} />
                  <MetricCard label="Owner" value={bootstrap?.scope.username ?? 'Loading'} note={bootstrap?.scope.owner_did ?? 'Resolving scope'} />
                </div>
              </div>

              {error ? (
                <div className="mt-4 rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700">
                  {error}
                </div>
              ) : null}
            </div>

            <div className="px-6 py-6">
              {section === 'today' ? (
                <TodaySection
                  bootstrap={bootstrap}
                  contacts={contacts}
                  loading={loading}
                  inboxItems={todayInbox}
                  taskBlueprints={taskBlueprints}
                  agentBlueprints={agentBlueprints}
                  sendEnabled={sendEnabled}
                />
              ) : null}
              {section === 'chat' ? <ChatWindow /> : null}
              {section === 'people' ? <PeopleSection contacts={contacts} loading={loading} /> : null}
              {section === 'tasks' ? <TasksSection taskBlueprints={taskBlueprints} /> : null}
              {section === 'agents' ? <AgentsSection agentBlueprints={agentBlueprints} /> : null}
            </div>
          </section>
        </main>
      </div>
    </div>
  )
}

const MetricCard = (props: { label: string; value: string; note: string }) => {
  const { label, value, note } = props

  return (
    <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
      <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{label}</p>
      <p className="mt-2 text-sm font-semibold text-[var(--cp-ink)]">{value}</p>
      <p className="mt-1 text-xs text-[var(--cp-muted)]">{note}</p>
    </div>
  )
}

const TodaySection = (props: {
  bootstrap: ChatBootstrapResponse | null
  contacts: ChatContact[]
  loading: boolean
  inboxItems: TodayInboxItem[]
  taskBlueprints: TaskBlueprint[]
  agentBlueprints: AgentBlueprint[]
  sendEnabled: boolean
}) => {
  const { bootstrap, contacts, loading, inboxItems, taskBlueprints, agentBlueprints, sendEnabled } = props
  const mostRecentContacts = [...contacts].sort((left, right) => right.updated_at - left.updated_at).slice(0, 4)
  const needsReply = inboxItems.filter((item) => item.lane === 'needs-reply')
  const followUps = inboxItems.filter((item) => item.lane === 'follow-up')
  const agentSignals = inboxItems.filter((item) => item.lane === 'agent')

  return (
    <div className="space-y-6">
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.4fr)_360px]">
        <div className="rounded-[28px] border border-[var(--cp-border)] bg-[linear-gradient(135deg,_rgba(15,118,110,0.96),_rgba(11,95,89,0.96))] px-6 py-6 text-white shadow-[0_24px_60px_-36px_rgba(11,95,89,0.55)]">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="max-w-2xl">
              <p className="text-xs font-semibold uppercase tracking-[0.24em] text-white/75">Today inbox</p>
              <h3 className="mt-2 text-3xl font-semibold leading-tight">
                Keep the high-signal loop small enough to act on immediately.
              </h3>
              <p className="mt-3 text-sm leading-6 text-white/80">
                Message Hub should tell you what needs a reply, what should become a task, and what agent work needs human review.
              </p>
            </div>

            <div className="grid gap-3 sm:grid-cols-3">
              <HeroStat label="Need reply" value={String(needsReply.length)} />
              <HeroStat label="Follow-ups" value={String(followUps.length)} />
              <HeroStat label="Agent items" value={String(agentSignals.length)} />
            </div>
          </div>

          <div className="mt-5 flex flex-wrap gap-2">
            <span className="cp-pill bg-white/12 text-white">{sendEnabled ? 'Replies enabled' : 'Read-only inbox'}</span>
            <span className="cp-pill bg-white/12 text-white">{contacts.length} known people</span>
            <span className="cp-pill bg-white/12 text-white">{bootstrap?.scope.username ?? 'Loading scope'}</span>
          </div>
        </div>

        <div className="rounded-[28px] border border-[var(--cp-border)] bg-white px-5 py-5 shadow-sm">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Triage policy</h3>
              <p className="mt-1 text-sm text-[var(--cp-muted)]">How Today decides what deserves front-row attention.</p>
            </div>
            <span className="cp-pill bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">Live shell</span>
          </div>

          <div className="mt-4 space-y-3">
            {[
              'Surface recent people activity that likely needs a reply first',
              'Promote unresolved threads into follow-up candidates',
              'Reserve a dedicated lane for agent approvals and raw records',
            ].map((item) => (
              <div key={item} className="rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3 text-sm leading-6 text-[var(--cp-muted)]">
                {item}
              </div>
            ))}
          </div>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <TodayLane title="Needs Reply" tone="urgent" icon="message" items={needsReply} emptyLabel="No urgent reply candidates right now." />
        <TodayLane title="Follow-up Queue" tone="focus" icon="todo" items={followUps} emptyLabel="No follow-up candidates yet." />
        <TodayLane title="Agent Signals" tone="watch" icon="agent" items={agentSignals} emptyLabel="No agent escalations yet." />
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <FocusCard
          icon="message"
          title="Keep the live chat loop sharp"
          description="Realtime contact threads are working now and remain the operational heartbeat of Message Hub."
        />
        <FocusCard
          icon="todo"
          title="Extract follow-ups"
          description="Turn commitments in conversations into small TODO objects with owners, due dates, and reminders."
        />
        <FocusCard
          icon="agent"
          title="Prepare agent escalation"
          description="Reserve a first-class place for approval requests, summaries, and raw inter-agent records."
        />
      </div>

      <div className="cp-panel p-5">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Upcoming extraction targets</h3>
            <p className="mt-1 text-sm text-[var(--cp-muted)]">
              These are the next real workflows that should feed Today instead of staying as blueprints.
            </p>
          </div>
          <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">
            Product build
          </span>
        </div>

        <div className="mt-4 grid gap-3 md:grid-cols-2">
          {[
            'Priority inbox for high-signal items and spam filtering',
            'Cross-channel people graph to merge chat, mail, and schedule context',
            'Notification center that feeds directly into TODO extraction',
            'Agent inbox with human approvals and raw communication replay',
          ].map((item) => (
            <div key={item} className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-ink)]">
              {item}
            </div>
          ))}
        </div>
      </div>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1.35fr)_360px]">
        <div className="cp-panel p-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Suggested tasks from today</h3>
              <p className="mt-1 text-sm text-[var(--cp-muted)]">A first pass at what Message Hub should extract automatically.</p>
            </div>
            <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
              {taskBlueprints.length} candidates
            </span>
          </div>

          <div className="mt-4 space-y-3">
            {taskBlueprints.length ? (
              taskBlueprints.map((task) => (
                <div key={task.id} className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="text-sm font-semibold text-[var(--cp-ink)]">{task.title}</p>
                      <p className="mt-1 text-xs text-[var(--cp-muted)]">{task.owner}</p>
                    </div>
                    <span className={`cp-pill ${task.status === 'active' ? 'bg-emerald-100 text-emerald-700' : task.status === 'waiting' ? 'bg-amber-100 text-amber-700' : 'bg-slate-100 text-slate-600'}`}>
                      {task.status}
                    </span>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2 text-xs text-[var(--cp-muted)]">
                    <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">due {task.dueLabel}</span>
                    <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">{task.sourceLabel}</span>
                  </div>
                </div>
              ))
            ) : (
              <p className="text-sm text-[var(--cp-muted)]">No task candidates yet.</p>
            )}
          </div>
        </div>

        <div className="space-y-6">
          <div className="cp-panel p-5">
            <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Scope notes</h3>
            <div className="mt-4 space-y-2 text-sm leading-6 text-[var(--cp-muted)]">
              {(bootstrap?.notes ?? []).map((note) => (
                <p key={note}>{note}</p>
              ))}
              {!bootstrap && loading ? <p>Loading Message Hub notes...</p> : null}
            </div>
          </div>

          <div className="cp-panel p-5">
            <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Recent people movement</h3>
            <div className="mt-4 space-y-3">
              {loading ? (
                <p className="text-sm text-[var(--cp-muted)]">Loading contacts...</p>
              ) : mostRecentContacts.length ? (
                mostRecentContacts.map((contact) => (
                  <div key={contact.did} className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                    <div className="flex items-center justify-between gap-3">
                      <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{contact.name || contact.did}</p>
                      <span className="text-xs text-[var(--cp-muted)]">{formatRelativeFreshness(contact.updated_at)}</span>
                    </div>
                    <p className="mt-1 truncate text-xs text-[var(--cp-muted)]">{contact.did}</p>
                    <p className="mt-2 text-xs text-[var(--cp-muted)]">{summarizeBindings(contact)}</p>
                  </div>
                ))
              ) : (
                <p className="text-sm text-[var(--cp-muted)]">No recent contact movement yet.</p>
              )}
            </div>
          </div>

          <div className="cp-panel p-5">
            <div className="flex items-center justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Agent queue preview</h3>
                <p className="mt-1 text-sm text-[var(--cp-muted)]">The kinds of agent-originated items that should surface in Today.</p>
              </div>
              <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
                {agentBlueprints.length} items
              </span>
            </div>

            <div className="mt-4 space-y-3">
              {agentBlueprints.map((agentItem) => (
                <div key={agentItem.id} className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="text-sm font-semibold text-[var(--cp-ink)]">{agentItem.title}</p>
                      <p className="mt-2 text-sm leading-6 text-[var(--cp-muted)]">{agentItem.summary}</p>
                    </div>
                    <span className={`cp-pill ${agentItem.priority === 'urgent' ? 'bg-rose-100 text-rose-700' : 'bg-slate-100 text-slate-600'}`}>
                      {agentItem.priority}
                    </span>
                  </div>
                  <p className="mt-3 text-xs text-[var(--cp-muted)]">{agentItem.threadId}</p>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

const FocusCard = (props: { icon: IconName; title: string; description: string }) => {
  const { icon, title, description } = props

  return (
    <div className="rounded-3xl border border-[var(--cp-border)] bg-white px-5 py-5 shadow-sm">
      <span className="inline-flex size-11 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
        <Icon name={icon} className="size-5" />
      </span>
      <h3 className="mt-4 text-lg font-semibold text-[var(--cp-ink)]">{title}</h3>
      <p className="mt-2 text-sm leading-6 text-[var(--cp-muted)]">{description}</p>
    </div>
  )
}

const HeroStat = (props: { label: string; value: string }) => {
  const { label, value } = props

  return (
    <div className="rounded-2xl border border-white/12 bg-white/10 px-4 py-3 backdrop-blur-sm">
      <p className="text-xs uppercase tracking-wide text-white/70">{label}</p>
      <p className="mt-2 text-lg font-semibold text-white">{value}</p>
    </div>
  )
}

const TodayLane = (props: {
  title: string
  tone: TodayInboxItem['tone']
  icon: IconName
  items: TodayInboxItem[]
  emptyLabel: string
}) => {
  const { title, tone, icon, items, emptyLabel } = props
  const toneClass =
    tone === 'urgent'
      ? 'bg-rose-50 border-rose-200'
      : tone === 'focus'
        ? 'bg-amber-50 border-amber-200'
        : 'bg-sky-50 border-sky-200'

  return (
    <section className={`rounded-[28px] border ${toneClass} p-4`}>
      <div className="flex items-center gap-3">
        <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-white text-[var(--cp-primary-strong)] shadow-sm">
          <Icon name={icon} className="size-5" />
        </span>
        <div>
          <h3 className="text-lg font-semibold text-[var(--cp-ink)]">{title}</h3>
          <p className="text-sm text-[var(--cp-muted)]">{items.length} queued signals</p>
        </div>
      </div>

      <div className="mt-4 space-y-3">
        {items.length ? (
          items.map((item) => <InboxCard key={item.id} item={item} />)
        ) : (
          <div className="rounded-2xl border border-white/70 bg-white/70 px-4 py-4 text-sm text-[var(--cp-muted)]">
            {emptyLabel}
          </div>
        )}
      </div>
    </section>
  )
}

const InboxCard = (props: { item: TodayInboxItem }) => {
  const { item } = props
  const pillClass =
    item.tone === 'urgent'
      ? 'bg-rose-100 text-rose-700'
      : item.tone === 'focus'
        ? 'bg-amber-100 text-amber-700'
        : 'bg-sky-100 text-sky-700'

  return (
    <NavLink to={item.ctaPath} className="block rounded-2xl border border-white/70 bg-white/90 px-4 py-4 shadow-sm transition hover:-translate-y-0.5 hover:shadow-md">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-semibold text-[var(--cp-ink)]">{item.title}</p>
          <p className="mt-2 text-sm leading-6 text-[var(--cp-muted)]">{item.summary}</p>
        </div>
        <span className={`cp-pill ${pillClass}`}>{item.freshnessLabel}</span>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-[var(--cp-muted)]">
        <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">{item.meta}</span>
        <span className="cp-pill bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">{item.ctaLabel}</span>
      </div>
    </NavLink>
  )
}

const PeopleSection = (props: { contacts: ChatContact[]; loading: boolean }) => {
  const { contacts, loading } = props

  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {loading ? (
        <div className="rounded-3xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-8 text-sm text-[var(--cp-muted)]">
          Loading people graph...
        </div>
      ) : contacts.length ? (
        contacts.map((contact) => (
          <article key={contact.did} className="rounded-3xl border border-[var(--cp-border)] bg-white p-5 shadow-sm">
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-[var(--cp-ink)]">{contact.name || 'Unnamed contact'}</h3>
                <p className="mt-1 break-all text-xs text-[var(--cp-muted)]">{contact.did}</p>
              </div>
              <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
                {contact.access_level}
              </span>
            </div>

            <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
              <div className="rounded-2xl bg-[var(--cp-surface-muted)] px-3 py-3">
                <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Bindings</p>
                <p className="mt-2 font-semibold text-[var(--cp-ink)]">{contact.bindings.length}</p>
              </div>
              <div className="rounded-2xl bg-[var(--cp-surface-muted)] px-3 py-3">
                <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Updated</p>
                <p className="mt-2 font-semibold text-[var(--cp-ink)]">{formatRelativeFreshness(contact.updated_at)}</p>
              </div>
            </div>

            <p className="mt-4 text-sm leading-6 text-[var(--cp-muted)]">
              {contact.note || 'This contact can later merge direct messages, mail addresses, calendars, and notifications into one timeline.'}
            </p>

            <div className="mt-4 flex flex-wrap gap-2">
              {(contact.tags.length ? contact.tags : ['chat']).slice(0, 4).map((tag) => (
                <span key={`${contact.did}-${tag}`} className="cp-pill bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                  {tag}
                </span>
              ))}
            </div>
          </article>
        ))
      ) : (
        <div className="rounded-3xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-8 text-sm text-[var(--cp-muted)]">
          No contacts yet. This page will become the cross-channel people graph once Message Hub ingests more sources.
        </div>
      )}
    </div>
  )
}

const TasksSection = (props: { taskBlueprints: TaskBlueprint[] }) => {
  const { taskBlueprints } = props
  const lanes = [
    {
      title: 'Extract from conversations',
      items: [
        'Detect promises, asks, and deadlines from messages',
        'Create lightweight TODOs with source links',
        'Mark tasks as waiting, active, or blocked',
      ],
    },
    {
      title: 'Escalate what matters',
      items: [
        'Bubble up missed replies and approaching deadlines',
        'Keep high-signal work visible in Today',
        'Let AI suggest the next follow-up action',
      ],
    },
    {
      title: 'Close the loop',
      items: [
        'Reply directly from the task context',
        'Convert tasks into calendar blocks when needed',
        'Track when the originating thread is resolved',
      ],
    },
  ]

  return (
    <div className="space-y-6">
      <div className="rounded-[28px] border border-[var(--cp-border)] bg-white px-6 py-6 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h3 className="text-xl font-semibold text-[var(--cp-ink)]">Task extraction queue</h3>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--cp-muted)]">
              These are the concrete follow-up objects the app should learn to generate automatically from communication context.
            </p>
          </div>
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            {taskBlueprints.length} seeded tasks
          </span>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-2">
          {taskBlueprints.map((task) => (
            <div key={task.id} className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-semibold text-[var(--cp-ink)]">{task.title}</p>
                  <p className="mt-1 text-xs text-[var(--cp-muted)]">{task.owner}</p>
                </div>
                <span className={`cp-pill ${task.status === 'active' ? 'bg-emerald-100 text-emerald-700' : task.status === 'waiting' ? 'bg-amber-100 text-amber-700' : 'bg-slate-100 text-slate-600'}`}>
                  {task.status}
                </span>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">due {task.dueLabel}</span>
                <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">{task.sourceLabel}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        {lanes.map((lane) => (
          <div key={lane.title} className="rounded-3xl border border-[var(--cp-border)] bg-white p-5 shadow-sm">
            <h3 className="text-lg font-semibold text-[var(--cp-ink)]">{lane.title}</h3>
            <div className="mt-4 space-y-3">
              {lane.items.map((item) => (
                <div key={item} className="rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3 text-sm leading-6 text-[var(--cp-muted)]">
                  {item}
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

const AgentsSection = (props: { agentBlueprints: AgentBlueprint[] }) => {
  const { agentBlueprints } = props

  return (
    <div className="grid gap-6 xl:grid-cols-[minmax(0,1.3fr)_360px]">
      <div className="space-y-4">
        {agentBlueprints.map((agentItem) => (
          <div key={agentItem.id} className="rounded-3xl border border-[var(--cp-border)] bg-white p-5 shadow-sm">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-3">
                <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                  <Icon name="agent" className="size-5" />
                </span>
                <div>
                  <h3 className="text-lg font-semibold text-[var(--cp-ink)]">{agentItem.title}</h3>
                  <p className="mt-1 text-xs text-[var(--cp-muted)]">{agentItem.threadId}</p>
                </div>
              </div>
              <span className={`cp-pill ${agentItem.priority === 'urgent' ? 'bg-rose-100 text-rose-700' : 'bg-slate-100 text-slate-600'}`}>
                {agentItem.priority}
              </span>
            </div>
            <p className="mt-3 text-sm leading-6 text-[var(--cp-muted)]">{agentItem.summary}</p>
          </div>
        ))}

        {[
          {
            title: 'Agent inbox',
            description: 'Collect approval requests, blocked tasks, and agent summaries that need a human decision.',
          },
          {
            title: 'RAW communication records',
            description: 'Browse agent-to-agent records with thread IDs, operation names, timestamps, and replay links.',
          },
          {
            title: 'Execution context bridge',
            description: 'Jump from a message to the related task log, result object, or runtime trace when debugging.',
          },
        ].map((panel) => (
          <div key={panel.title} className="rounded-3xl border border-[var(--cp-border)] bg-white p-5 shadow-sm">
            <div className="flex items-center gap-3">
              <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="agent" className="size-5" />
              </span>
              <h3 className="text-lg font-semibold text-[var(--cp-ink)]">{panel.title}</h3>
            </div>
            <p className="mt-3 text-sm leading-6 text-[var(--cp-muted)]">{panel.description}</p>
          </div>
        ))}
      </div>

      <div className="rounded-3xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-5">
        <h3 className="text-lg font-semibold text-[var(--cp-ink)]">First raw record shape</h3>
        <div className="mt-4 space-y-3 font-mono text-[11px] leading-5 text-[var(--cp-ink)]">
          <pre className="whitespace-pre-wrap rounded-2xl bg-white px-4 py-4">{`{
  "thread_id": "agent-thread-42",
  "from": "planner-agent",
  "to": "executor-agent",
  "kind": "action_request",
  "summary": "Prepare nightly sync report",
  "ts": 1773373373000
}`}</pre>
        </div>
      </div>
    </div>
  )
}

export default MessageHubPage
