/* Where a personal site starts (and what "add a website" offers later): importing the bookmarks of another browser,
 * opening a website to add it from there, or telling the Agent in the conversation what to read. */

import { BookmarkPlus, Globe, MessageSquare } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { usePickFiles } from '../../../sysdlg/select/pickFiles'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { bookmarkSites, type BookmarkSite } from './bookmarks'
import { sourceOf } from './data'
import { SiteNotes } from './SiteStrip'
import { useSite, useSiteController } from './useSite'

const PICKER_ID = 'agent-browser-bookmarks'

/** The websites of a bookmarks file: the owner ticks the ones to read (none to begin with); they are added one at a time. */
function BookmarkSheet({ sites, onClose }: { sites: BookmarkSite[]; onClose: () => void }) {
  const controller = useSiteController()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const site = useSite((snapshot) => snapshot.site)
  const adds = useSite((snapshot) => snapshot.adds)
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set())
  const taken = (item: BookmarkSite) => (site && sourceOf(site, item.url) ? 'added' : adds.some((add) => add.host === item.host) ? 'pending' : null)
  const toggle = (host: string) => setChosen((before) => {
    const next = new Set(before)
    if (!next.delete(host)) next.add(host)
    return next
  })
  const add = () => {
    controller.queueAdds(sites.filter((item) => chosen.has(item.host)).map(({ url, title }) => ({ url, title })))
    onClose()
  }
  return (
    <Sheet
      testId="ab-bookmarks-sheet"
      title={t('agentBrowser.site.bookmarks.title', 'Websites in your bookmarks')}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" data-testid="ab-bookmarks-cancel" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ab-button ab-button-primary" disabled={chosen.size === 0} data-testid="ab-bookmarks-add" onClick={add}>{t('agentBrowser.site.bookmarks.add', 'Add selected: {{count}}', { count: chosen.size })}</button>
      </>}
    >
      {sites.length === 0 ? <p className="ab-sheet-empty" data-testid="ab-bookmarks-empty">{t('agentBrowser.site.bookmarks.empty', 'This file has no bookmarks of websites. Export your bookmarks from a browser as an HTML file and choose that.')}</p> : (
        <>
          <p className="ab-dialog-message">{t('agentBrowser.site.bookmarks.hint', 'Tick the websites you want to read here. {{name}} adds them one at a time; each takes a few minutes.', { name })}</p>
          <ul className="ab-list" data-testid="ab-bookmarks-list">
            {sites.map((item) => {
              const state = taken(item)
              return (
                <li key={item.host}>
                  <button type="button" role="checkbox" aria-checked={chosen.has(item.host)} disabled={state !== null} className="ab-list-item" data-testid="ab-bookmark" data-host={item.host} data-taken={state ?? undefined} onClick={() => toggle(item.host)}>
                    <span className="ab-check" aria-hidden />
                    <span className="ab-list-text">
                      <span className="ab-list-title">{item.host}</span>
                      <span className="ab-list-detail">
                        {state === 'added' ? t('agentBrowser.site.bookmarks.added', 'Already in your reading')
                          : state === 'pending' ? t('agentBrowser.site.bookmarks.pending', 'Being added')
                          : item.count > 1 ? t('agentBrowser.site.bookmarks.count', 'Bookmarks: {{count}}', { count: item.count }) : item.title}
                      </span>
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        </>
      )}
    </Sheet>
  )
}

/**
 * The three starting points: `list` shows them, `over` is what one of them put over the App (the bookmarks of the
 * file that was chosen; null with nothing), `away` is true while the system file picker is up. `onChosen` runs when
 * the owner is done here.
 */
function useStartPoints(onChosen?: () => void): { list: ReactNode; over: ReactNode; away: boolean } {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const pickFiles = usePickFiles(PICKER_ID)
  const [bookmarks, setBookmarks] = useState<BookmarkSite[] | null>(null)
  const [picking, setPicking] = useState(false)
  const importBookmarks = async () => {
    setPicking(true)
    try {
      const picked = await pickFiles({ accept: '.html,.htm,text/html', title: t('agentBrowser.site.start.pickBookmarks', 'Choose a bookmarks file (HTML)') })
      if (picked?.length) setBookmarks(bookmarkSites(await picked[0].text()))
    } catch {
      setBookmarks([])
    } finally {
      setPicking(false)
    }
  }
  const points = [
    { id: 'bookmarks', icon: <BookmarkPlus size={20} aria-hidden />, title: t('agentBrowser.site.start.bookmarks', 'Import bookmarks'), detail: t('agentBrowser.site.start.bookmarksHint', 'Pick the websites to read from the bookmarks file of another browser.'), run: () => void importBookmarks() },
    { id: 'open', icon: <Globe size={20} aria-hidden />, title: t('agentBrowser.site.start.open', 'Open a website'), detail: t('agentBrowser.site.start.openHint', 'Go to a website you read, then press Reflow beside its address.'), run: () => { onChosen?.(); store.setMode('page') } },
    { id: 'chat', icon: <MessageSquare size={20} aria-hidden />, title: t('agentBrowser.site.start.chat', 'Tell {{name}}', { name }), detail: t('agentBrowser.site.start.chatHint', 'Say in the conversation which websites you want to read.'), run: () => { onChosen?.(); store.setChatOpen(true) } },
  ]
  const list = (
    <ul className="ab-list ab-start-points" data-testid="ab-start-points">
      {points.map((point) => (
        <li key={point.id}>
          <button type="button" className="ab-list-item ab-start-point" disabled={picking} data-testid={`ab-start-${point.id}`} onClick={point.run}>
            {point.icon}
            <span className="ab-list-text">
              <span className="ab-list-title">{point.title}</span>
              <span className="ab-list-detail">{point.detail}</span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  )
  const over = bookmarks && !picking ? <BookmarkSheet sites={bookmarks} onClose={() => { setBookmarks(null); onChosen?.() }} /> : null
  return { list, over, away: picking }
}

/** Home before the site has a website: what the site is, and the three ways to start it. */
export function SiteStart() {
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const { list, over } = useStartPoints()
  return (
    <div className="ab-start" data-testid="ab-site-start">
      <h2 className="ab-start-title">{t('agentBrowser.site.start.title', 'Your reading site')}</h2>
      <p className="ab-start-hint">{t('agentBrowser.site.start.hint', '{{name}} reads the websites you choose and keeps their articles here, in your language and laid out for your phone. Start with a few websites:', { name })}</p>
      {list}
      <div className="ab-start-notes"><SiteNotes /></div>
      {over}
    </div>
  )
}

/** "Add a website" from the site's own page: the same three ways. The file picker and the bookmarks show instead of this panel, not over it. */
export function StartSheet({ onClose }: { onClose: () => void }) {
  const { t } = useI18n()
  const { list, over, away } = useStartPoints(onClose)
  if (away) return null
  return over ?? <Sheet testId="ab-start-sheet" title={t('agentBrowser.site.start.sheetTitle', 'Add a website')} onClose={onClose}>{list}</Sheet>
}
