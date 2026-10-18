/* ── My Network layout: relation navigation | entries | details (PRD §5.2); a stack on phones ── */

import { useMediaQuery } from '@mui/material'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useMobileBackHandler } from '../../../desktop/windows/MobileNavContext'
import { useI18n } from '../../../i18n/provider'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { useNetworkResource, useRequests } from '../../../components/network/hooks'
import { actionableCount, tagKind } from '../../../components/network/relation'
import type { MyNetworkLaunch } from '../launch'
import { collectionsOf } from '../datamodel/collections'
import type { MyNetworkView } from '../datamodel/types'
import { useCollectionActions } from './collectionActions'
import { CollectionList } from './CollectionList'
import { ContactDetailPage } from './ContactDetailPage'
import { JoinedGroupDetail, JoinedGroupList } from './GroupsView'
import { HomeOverview } from './HomeOverview'
import { NavSidebar } from './NavSidebar'
import { RequestsView } from './RequestsView'

type MobileStage = 'nav' | 'view' | 'detail'

function initialView(launch?: MyNetworkLaunch): MyNetworkView {
  if (!launch) return { kind: 'home' }
  if ('view' in launch) return { kind: 'requests' }
  if ('entity' in launch) return { kind: 'contacts' }
  return { kind: 'home' }
}

function DetailPlaceholder() {
  const { t } = useI18n()
  return <div className="flex h-full items-center justify-center px-8 text-center text-sm text-[color:var(--cp-muted)]">{t('myNetwork.detail.placeholder')}</div>
}

export function MyNetworkShell({ launch }: { launch?: MyNetworkLaunch }) {
  const isMobile = useMediaQuery('(max-width: 767px)')
  const dialogs = useNetworkDialogs()
  const collectionActions = useCollectionActions()
  const contacts = useNetworkResource('contacts')
  const groups = useNetworkResource('groups')
  const requests = useRequests()
  const [view, setView] = useState<MyNetworkView>(() => initialView(launch))
  const [selected, setSelected] = useState<string | null>(() => (launch && 'entity' in launch ? launch.entity : null))
  const [stage, setStage] = useState<MobileStage>(() => (!launch ? 'nav' : 'entity' in launch ? 'detail' : 'view'))
  const launchHandled = useRef(false)

  const collections = useMemo(() => collectionsOf(contacts.data ?? []), [contacts.data])
  const counts = {
    contacts: contacts.data?.length,
    friends: contacts.data?.filter(contact => contact.friend_state === 'friend').length,
    groups: groups.data?.filter(group => group.state === 'active').length,
    requests: actionableCount(requests.items),
  }

  const navigate = useCallback((next: MyNetworkView) => {
    setView(next)
    setSelected(null)
    setStage('view')
  }, [])
  const select = useCallback((did: string) => {
    setSelected(did)
    setStage('detail')
  }, [])
  const openPerson = useCallback((did: string) => {
    setView({ kind: 'contacts' })
    setSelected(did)
    setStage('detail')
  }, [])

  const addFriend = useCallback(async (input?: string) => {
    const result = await dialogs.addFriend(input ? { input } : {})
    if (result.action === 'done') openPerson(result.person.did)
  }, [dialogs, openPerson])
  const joinGroup = useCallback(async (input?: string) => {
    const result = await dialogs.joinGroup(input ? { input } : {})
    if (result.action === 'joined' && result.state === 'active') {
      setView({ kind: 'groups' })
      setSelected(result.groupDid)
      setStage('detail')
    }
  }, [dialogs])
  const createCollection = async () => {
    const tag = await collectionActions.create(contacts.data ?? []).catch(() => undefined)
    if (tag) navigate({ kind: 'collection', tag })
  }

  useEffect(() => {
    if (!launch || !('action' in launch) || launchHandled.current) return
    const timer = window.setTimeout(() => {
      launchHandled.current = true
      if (launch.action === 'add-friend') void addFriend(launch.input)
      else void joinGroup(launch.input)
    }, 0)
    return () => window.clearTimeout(timer)
  }, [launch, addFriend, joinGroup])

  const back = useCallback(() => {
    setStage(current => (current === 'detail' && view.kind !== 'home' && view.kind !== 'requests' ? 'view' : 'nav'))
    setSelected(null)
  }, [view.kind])
  useMobileBackHandler(isMobile && stage !== 'nav' ? back : null)

  const nav = (
    <NavSidebar
      view={isMobile ? null : view}
      counts={counts}
      collections={collections}
      onSelect={navigate}
      onAddFriend={() => void addFriend()}
      onJoinGroup={() => void joinGroup()}
      onCreateCollection={() => void createCollection()}
      wide={isMobile}
    />
  )

  const collectionTag = view.kind === 'collection' && tagKind(view.tag) === 'user' ? view.tag : undefined
  const detail = selected ? (
    view.kind === 'groups' || groups.data?.some(group => group.did === selected)
      ? <JoinedGroupDetail key={selected} did={selected} onLeft={() => setSelected(null)} />
      : <ContactDetailPage key={selected} did={selected} collectionTag={collectionTag} onRemoved={() => setSelected(null)} />
  ) : null

  const list = view.kind === 'groups'
    ? <JoinedGroupList selectedDid={selected} onSelect={select} onJoinGroup={() => void joinGroup()} wide={isMobile} />
    : view.kind === 'contacts' || view.kind === 'friends' || view.kind === 'collection'
      ? (
        <CollectionList
          key={view.kind === 'collection' ? `c:${view.tag}` : view.kind}
          view={view}
          selectedDid={selected}
          onSelect={select}
          onCollectionGone={(renamed) => navigate(renamed ? { kind: 'collection', tag: renamed } : { kind: 'contacts' })}
          wide={isMobile}
        />
      )
      : null

  const single = view.kind === 'home'
    ? <HomeOverview collections={collections} onNavigate={navigate} onOpenPerson={openPerson} onAddFriend={() => void addFriend()} onJoinGroup={() => void joinGroup()} />
    : view.kind === 'requests' ? <RequestsView /> : null

  if (isMobile) {
    return (
      <div className="flex h-full w-full flex-col overflow-hidden bg-[color:var(--cp-bg)]" data-testid="my-network" data-stage={stage}>
        {stage === 'nav' ? nav : stage === 'detail' && detail ? (
          <main className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto px-4 py-4">{detail}</main>
        ) : (
          <div className="flex min-h-0 flex-1">{single ?? list}</div>
        )}
      </div>
    )
  }

  return (
    <div className="flex h-full w-full overflow-hidden bg-[color:var(--cp-bg)]" data-testid="my-network">
      {nav}
      {single ?? (
        <>
          {list}
          <main className="desktop-scrollbar min-w-0 flex-1 overflow-y-auto" data-testid="my-network-detail">
            {detail ? <div className="max-w-3xl px-6 py-5">{detail}</div> : <DetailPlaceholder />}
          </main>
        </>
      )}
    </div>
  )
}
