import { useMediaQuery } from '@mui/material'
import { Eye } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import './homestation.css'
import { previewReaderLabel } from './card/labels'
import { ItemDetail } from './detail/ItemDetail'
import { installHomeStationPreviewSource } from './media'
import { HsNavContext, type HsNav, type HsPage } from './navContext'
import { PublicProfileView } from './PublicProfileView'
import { useHomeStationStore, usePreviewReaders, type PreviewReaderKey } from './store/context'
import { HomeStationStoreProvider } from './store/HomeStationStoreProvider'
import { ToastHost } from './ui/ToastHost'

installHomeStationPreviewSource()

const READER_KEYS: PreviewReaderKey[] = ['anonymous', 'follower', 'friend']

function VisitorView() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const readers = usePreviewReaders()
  const { did = 'did:bns:leo' } = useParams()
  const [params, setParams] = useSearchParams()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const [pages, setPages] = useState<HsPage[]>([{ name: 'profile' }])
  const requested = params.get('reader') as PreviewReaderKey | null
  const readerKey: PreviewReaderKey = requested && READER_KEYS.includes(requested) && readers[requested] ? requested : 'anonymous'
  const reader = readers[readerKey] ?? readers.anonymous
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
  const nameOf = (target: string) => store.peekIdentity(target).name

  return (
    <HsNavContext.Provider value={nav}>
      <main className="hs-root relative mx-auto flex h-dvh w-full max-w-3xl flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }} data-testid="hs-visitor" data-reader={readerKey}>
        <ToastHost>
          <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
            <div className="flex flex-wrap items-center gap-1.5 px-4 py-2 text-[11px]" style={{ borderBottom: '1px solid var(--hs-divider)', background: 'var(--hs-subtle-bg)' }}>
              <Eye size={12} />
              <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.visitor.readingAs', 'Reading as')}</span>
              {READER_KEYS.filter(key => readers[key]).map(key => (
                <button key={key} type="button" className="hs-chip" style={{ padding: '3px 10px' }} aria-pressed={readerKey === key} data-testid={`hs-visitor-reader-${key}`} onClick={() => { setParams({ reader: key }); setPages([{ name: 'profile' }]) }}>
                  {previewReaderLabel(t, key, readers[key], nameOf)}
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
  )
}

// 待确认（TODO §13）：公开门户最终由 Zone 根 $ 上的 HomeStation 服务提供，原型阶段在 Desktop 内用路由模拟访客视图
export function HomeStationVisitorRoute() {
  return (
    <HomeStationStoreProvider>
      <VisitorView />
    </HomeStationStoreProvider>
  )
}
