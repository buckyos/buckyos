import {
  BellOff,
  Copy,
  ExternalLink,
  Eye,
  EyeOff,
  Globe,
  Lock,
  MoreHorizontal,
  Pencil,
  Share2,
  Tags,
  ThumbsDown,
  Trash2,
  UsersRound,
} from 'lucide-react'
import { forwardRef, useImperativeHandle, useMemo, useRef, useState } from 'react'
import { useWindowDialog } from '../../../desktop/windows/dialogs'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import type { CardView } from '../datamodel/types'
import { useHomeStationStore } from '../store/context'
import { Menu, type MenuItemSpec } from '../ui/Popover'
import { ReasonDetail } from './ReasonDetail'
import { TagDetail } from './parts'

export interface ItemMenuHandle {
  open: () => void
}

export const ItemMenu = forwardRef<ItemMenuHandle, { view: CardView; allowOwnerActions?: boolean }>(function ItemMenu({ view, allowOwnerActions = true }, ref) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const actions = useItemActions()
  const dialog = useWindowDialog()
  const buttonRef = useRef<HTMLButtonElement>(null)
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)

  useImperativeHandle(ref, () => ({ open: () => setAnchor(buttonRef.current) }), [])

  const items = useMemo(() => {
    const { item, reading, personal } = view
    const list: MenuItemSpec[] = []
    const entry = item.entry
    if (item.isOwn && allowOwnerActions && entry && entry.state !== 'withdrawn' && !item.isPrivateCapture) {
      const text = item.object.content?.text
      if (text !== undefined && item.contentType !== 'reaction') {
        list.push({ id: 'edit', label: t('homestation.menu.edit', 'Edit (publish a new version)'), icon: <Pencil size={14} />, onSelect: () => void actions.edit(entry.entry, text) })
      }
      list.push({ id: 'audience', label: t('homestation.menu.audience', 'Change audience'), icon: <UsersRound size={14} />, onSelect: () => void actions.changeAudience(entry.entry, item.audience.spec) })
      list.push({ id: 'withdraw', label: t('homestation.menu.withdraw', 'Withdraw'), hint: t('homestation.menu.withdrawHint', 'Copies already delivered can’t be recalled'), icon: <Trash2 size={14} />, danger: true, onSelect: () => void actions.withdraw(entry.entry) })
    }
    if (item.isPrivateCapture) {
      list.push(view.sharedAs
        ? { id: 'shared', label: t('homestation.menu.alreadyShared', 'Already shared to your homepage'), icon: <Share2 size={14} />, disabled: true, onSelect: () => {} }
        : { id: 'share', label: t('homestation.menu.share', 'Share to my homepage'), hint: t('homestation.menu.shareHint', 'Private captures are not published until you share them'), icon: <Share2 size={14} />, onSelect: () => void actions.shareCapture(view) })
    }
    if (reading) {
      list.push({
        id: 'why',
        label: t('homestation.menu.why', 'Why am I seeing this?'),
        icon: <Eye size={14} />,
        onSelect: () => void dialog.open({ title: t('homestation.reason.title', 'Why you’re seeing this'), size: 'sm', renderBody: () => <div className="hs-root"><ReasonDetail view={view} /></div> }),
      })
    }
    if (!item.isOwn && item.publisher.kind === 'person' && item.publisher.did) {
      const did = item.publisher.did
      list.push({ id: 'mute', label: t('homestation.menu.mutePerson', 'Don’t show {{name}}', { name: item.publisher.name }), hint: t('homestation.menu.muteHint', 'Keeps following and friendship; nobody is notified'), icon: <BellOff size={14} />, onSelect: () => void actions.mute({ kind: 'person', did, name: item.publisher.name }) })
      for (const group of store.peekGroups().filter(entry => entry.members.includes(did))) {
        list.push({ id: `mute-${group.id}`, label: t('homestation.menu.muteGroup', 'Don’t show the group “{{name}}”', { name: group.name }), icon: <EyeOff size={14} />, onSelect: () => void actions.mute({ kind: 'group', groupId: group.id, name: group.name }) })
      }
    }
    if (reading) {
      list.push({ id: 'less', label: t('homestation.menu.lessLike', 'Show less like this'), icon: <ThumbsDown size={14} />, onSelect: () => void actions.lessLike(view) })
      if (reading.effectiveTags.length > 0) {
        list.push({
          id: 'tags',
          label: t('homestation.menu.correctTags', 'Correct tags…'),
          icon: <Tags size={14} />,
          onSelect: () => void dialog.open({
            title: t('homestation.menu.correctTags', 'Correct tags…'),
            size: 'md',
            renderBody: controls => (
              <div className="hs-root divide-y" style={{ borderColor: 'var(--hs-divider)' }}>
                {reading.effectiveTags.map(tag => <TagDetail key={`${tag.tag}-${tag.source}`} tag={tag} objId={item.objId} onClose={() => controls.dismiss()} />)}
              </div>
            ),
          }),
        })
      }
    }
    if (personal?.bookmark.on) {
      list.push(personal.bookmark.visibility === 'private'
        ? { id: 'bookmark-public', label: t('homestation.menu.bookmarkPublic', 'Make bookmark public'), hint: t('homestation.menu.bookmarkPublicHint', 'Publishes a public bookmark signed by you'), icon: <Globe size={14} />, disabled: item.isPrivateCapture, onSelect: () => void actions.setBookmarkPublic(view, true) }
        : { id: 'bookmark-private', label: t('homestation.menu.bookmarkPrivate', 'Make bookmark private'), icon: <Lock size={14} />, onSelect: () => void actions.setBookmarkPublic(view, false) })
    }
    const original = item.object.link ?? item.object.source?.original_url
    if (original) list.push({ id: 'original', label: t('homestation.menu.original', 'View original'), hint: original, icon: <ExternalLink size={14} />, onSelect: () => actions.openLink(original) })
    list.push({ id: 'copy', label: t('homestation.menu.copyId', 'Copy object ID'), hint: item.objId, icon: <Copy size={14} />, onSelect: () => void actions.copyObjId(item.objId) })
    return list
  }, [actions, allowOwnerActions, dialog, store, t, view])

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="hs-icon-btn -mr-2 -mt-1"
        aria-label={t('homestation.menu.more', 'More actions')}
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        data-testid="hs-card-more"
        onClick={event => {
          event.stopPropagation()
          setAnchor(anchor ? null : event.currentTarget)
        }}
      >
        <MoreHorizontal size={16} />
      </button>
      <Menu anchor={anchor} open={!!anchor} onClose={() => setAnchor(null)} items={items} label={t('homestation.menu.more', 'More actions')} testId="hs-card-menu" />
    </>
  )
})
