import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Car, Clock, Coffee, Flag, Heart, Lightbulb, Loader2, PawPrint, Search, Smile, Trophy } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { loadEmojiGroups, recentEmoji, rememberEmoji, searchEmoji, type EmojiGroup, type EmojiGroupId, type EmojiItem } from './emoji'

const GROUP_ICONS: Record<EmojiGroupId, typeof Smile> = {
  recent: Clock,
  people: Smile,
  nature: PawPrint,
  food: Coffee,
  activity: Trophy,
  travel: Car,
  objects: Lightbulb,
  symbols: Heart,
  flags: Flag,
}

/** `tabbable`: the first emoji is the grid's tab stop; arrow keys move on from there. */
function EmojiGrid({ items, label, tabbable }: { items: readonly EmojiItem[]; label: string; tabbable: boolean }) {
  return (
    <div role="group" aria-label={label} className="mh-emoji-grid">
      {items.map((item, index) => (
        <button key={`${index}:${item.emoji}`} type="button" tabIndex={tabbable && index === 0 ? 0 : -1} data-emoji={item.emoji} aria-label={item.name} title={item.name}>{item.emoji}</button>
      ))}
    </div>
  )
}

/**
 * Telegram-style emoji panel above the composer: recently used first, the
 * categories below with a tab bar to jump between them, and a search over
 * the English emoji names. Picking keeps the panel open and the caret in the
 * input, so several emoji can be added in a row.
 */
export function EmojiPanel({ onPick, onClose }: { onPick: (emoji: string) => void; onClose: () => void }) {
  const { t } = useI18n()
  const [groups, setGroups] = useState<EmojiGroup[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [query, setQuery] = useState('')
  const [recent, setRecent] = useState(recentEmoji)
  const [active, setActive] = useState<EmojiGroupId>('recent')
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    loadEmojiGroups().then(loaded => { if (!cancelled) setGroups(loaded) }, () => { if (!cancelled) setFailed(true) })
    return () => { cancelled = true }
  }, [])

  const names = useMemo(() => {
    const map = new Map<string, string>()
    groups?.forEach(group => group.items.forEach(item => map.set(item.emoji, item.name)))
    return map
  }, [groups])
  const sections = useMemo<EmojiGroup[]>(() => [
    { id: 'recent', items: recent.map(emoji => ({ emoji, name: names.get(emoji) ?? emoji })) },
    ...(groups ?? []),
  ], [groups, names, recent])
  const results = useMemo(() => (query.trim() && groups ? searchEmoji(groups, query) : null), [groups, query])

  const pick = useCallback((emoji: string) => {
    onPick(emoji)
    setRecent(rememberEmoji(emoji))
  }, [onPick])

  const groupLabel = (id: EmojiGroupId) => t(`messagehub.emoji.group.${id}`)

  const syncActive = () => {
    const container = scrollRef.current
    if (!container || results) return
    let current: EmojiGroupId = 'recent'
    for (const section of container.querySelectorAll<HTMLElement>('[data-emoji-group]')) {
      if (section.offsetTop - container.offsetTop <= container.scrollTop + 8) current = section.dataset.emojiGroup as EmojiGroupId
    }
    setActive(current)
  }

  const jumpTo = (id: EmojiGroupId) => {
    const container = scrollRef.current
    const section = container?.querySelector<HTMLElement>(`[data-emoji-group="${id}"]`)
    if (!container || !section) return
    container.scrollTop = section.offsetTop - container.offsetTop
    setActive(id)
  }

  /** Arrow keys move between the emoji of the grid; Enter / Space pick (as buttons). */
  const handleGridKeys = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    if (!target.dataset.emoji) return
    const buttons = [...(scrollRef.current?.querySelectorAll<HTMLButtonElement>('button[data-emoji]') ?? [])]
    const index = buttons.indexOf(target as HTMLButtonElement)
    if (index < 0) return
    const columns = Math.max(1, buttons.findIndex(button => button.offsetTop !== buttons[0].offsetTop))
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowDown' ? columns : event.key === 'ArrowUp' ? -columns : 0
    if (!step) return
    event.preventDefault()
    buttons[Math.min(buttons.length - 1, Math.max(0, index + step))]?.focus()
  }

  return (
    <div
      role="dialog"
      aria-label={t('messagehub.emoji.open')}
      className="mh-emoji-panel"
      data-testid="emoji-panel"
      onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}
    >
      <label className="mh-emoji-search">
        <Search size={15} aria-hidden />
        <input type="search" value={query} onChange={(event) => { setQuery(event.target.value); if (scrollRef.current) scrollRef.current.scrollTop = 0 }} placeholder={t('messagehub.emoji.search')} aria-label={t('messagehub.emoji.search')} data-testid="emoji-search" />
      </label>
      <div
        ref={scrollRef}
        className="mh-emoji-scroll shell-scrollbar"
        onScroll={syncActive}
        // Keep the caret (and focus) in the message input while picking.
        onMouseDown={(event) => event.preventDefault()}
        onClick={(event) => {
          const emoji = (event.target as HTMLElement).closest<HTMLElement>('button[data-emoji]')?.dataset.emoji
          if (emoji) pick(emoji)
        }}
        onKeyDown={handleGridKeys}
      >
        {failed ? <p className="mh-emoji-empty" role="alert">{t('messagehub.emoji.loadFailed')}</p>
          : results ? (results.length > 0 ? <EmojiGrid items={results} label={t('messagehub.emoji.results')} tabbable /> : <p className="mh-emoji-empty">{t('messagehub.emoji.noResults')}</p>)
          : sections.map((section, index) => (
            <section key={section.id} data-emoji-group={section.id}>
              <h3 className="mh-emoji-heading">{groupLabel(section.id)}</h3>
              <EmojiGrid items={section.items} label={groupLabel(section.id)} tabbable={index === 0} />
            </section>
          ))}
        {!groups && !failed && !results ? <div className="mh-emoji-empty"><Loader2 size={18} className="animate-spin" aria-hidden /></div> : null}
      </div>
      {!results ? (
        <div className="mh-emoji-tabs" role="toolbar" aria-label={t('messagehub.emoji.categories')} onMouseDown={(event) => event.preventDefault()}>
          {sections.map(section => {
            const Icon = GROUP_ICONS[section.id]
            return (
              <button key={section.id} type="button" aria-label={groupLabel(section.id)} title={groupLabel(section.id)} aria-pressed={active === section.id} onClick={() => jumpTo(section.id)} data-testid={`emoji-tab-${section.id}`}>
                <Icon size={17} aria-hidden />
              </button>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}
