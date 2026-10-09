import { useMediaQuery } from '@mui/material'
import { Eye } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import './homestation.css'
import type { ReaderIdentity } from './datamodel/types'
import { ItemDetail } from './detail/ItemDetail'
import { installHomeStationPreviewSource } from './mock/media'
import { createHomeStationStore, parseScenario } from './mock/store'
import { HsNavContext, type HsNav, type HsPage } from './navContext'
import { PublicProfileView } from './PublicProfileView'
import { HomeStationStoreContext } from './store/context'
import { ToastHost } from './ui/ToastHost'

installHomeStationPreviewSource()

const READERS: Record<string, ReaderIdentity> = {
  anonymous: { kind: 'anonymous' },
  follower: { kind: 'did', did: 'did:bns:sarah' },
  friend: { kind: 'did', did: 'did:bns:bob' },
}

// 待确认（TODO §13）：公开门户最终由 Zone 根 $ 上的 HomeStation 服务提供，原型阶段在 Desktop 内用路由模拟访客视图
export function HomeStationVisitorRoute() {
  const { t } = useI18n()
  const { did = 'did:bns:leo' } = useParams()
  const [params, setParams] = useSearchParams()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const [store] = useState(() => createHomeStationStore(parseScenario(window.location.search)))
  const [pages, setPages] = useState<HsPage[]>([{ name: 'profile' }])
  const readerKey = params.get('reader') && READERS[params.get('reader')!] ? params.get('reader')! : 'anonymous'
  const reader = READERS[readerKey]
  const page = pages[pages.length - 1]
  const nav = useMemo<HsNav>(() => ({
    page,
    perspective: 'visitor',
    reader,
    isDesktop,
    navigate: next => setPages(current => [...current, next]),
    back: () => setPages(current => (current.length > 1 ? current.slice(0, -1) : current)),
    openDetail: objId => setPages(current => [...current, { name: 'detail', objId }]),
    showFilteredInFeed: () => {},
  }), [isDesktop, page, reader])
  const readerLabels: Record<string, string> = {
    anonymous: t('homestation.profile.asAnonymous', 'Anonymous'),
    follower: t('homestation.profile.asFollower', 'Follower (Sarah Kim)'),
    friend: t('homestation.profile.asFriend', 'Friend (Bob Zhang)'),
  }

  return (
    <HomeStationStoreContext.Provider value={store}>
      <HsNavContext.Provider value={nav}>
        <main className="hs-root relative mx-auto flex h-dvh w-full max-w-3xl flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }} data-testid="hs-visitor" data-reader={readerKey}>
          <ToastHost>
            <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
              <div className="flex flex-wrap items-center gap-1.5 px-4 py-2 text-[11px]" style={{ borderBottom: '1px solid var(--hs-divider)', background: 'var(--hs-subtle-bg)' }}>
                <Eye size={12} />
                <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.visitor.readingAs', 'Reading as')}</span>
                {Object.keys(READERS).map(key => (
                  <button key={key} type="button" className="hs-chip" style={{ padding: '3px 10px' }} aria-pressed={readerKey === key} data-testid={`hs-visitor-reader-${key}`} onClick={() => { setParams({ reader: key }); setPages([{ name: 'profile' }]) }}>
                    {readerLabels[key]}
                  </button>
                ))}
                <span className="hs-badge">{t('homestation.profile.devOnly', 'Dev tool')}</span>
                <span className="w-full" style={{ color: 'var(--cp-muted)' }}>{t('homestation.visitor.note', 'Prototype of the public portal. In production it is served by the HomeStation service at your Zone root, not by the Desktop (pending decision).')}</span>
              </div>
              <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto">
                {page.name === 'detail' ? <ItemDetail key={page.objId} objId={page.objId} /> : <PublicProfileView key={`${did}-${readerKey}`} owner={did} />}
              </div>
            </WindowDialogProvider>
          </ToastHost>
        </main>
      </HsNavContext.Provider>
    </HomeStationStoreContext.Provider>
  )
}
