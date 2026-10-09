import { useMemo } from 'react'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { AudienceBody, ConfirmBody, EditBody, QuoteBody } from './actionDialogs'
import { audienceLabel, isNarrowerAudience } from './datamodel/format'
import type { AudienceSpec, CardView, MuteRule } from './datamodel/types'
import type { HomeStationStore } from './mock/store'
import { useHsNav } from './navContext'
import type { EntryUrl, ObjId } from './protocol/feed'
import { useHomeStationStore } from './store/context'
import { useToast } from './ui/toastContext'

export function useItemActions() {
  const { t } = useI18n()
  const store: HomeStationStore = useHomeStationStore()
  const dialog = useWindowDialog()
  const toast = useToast()
  const nav = useHsNav()

  return useMemo(() => {
    const nameOf = (did: string) => store.peekIdentity(did).name
    const labelOf = (spec: AudienceSpec) => audienceLabel(t, spec, store.peekGroups(), nameOf)

    const confirm = (title: string, body: string, confirmLabel: string, options: { notes?: string[]; danger?: boolean; testId?: string } = {}) =>
      dialog.open<boolean>({ title, size: 'sm', renderBody: controls => <ConfirmBody controls={controls} body={body} notes={options.notes} confirmLabel={confirmLabel} danger={options.danger} testId={options.testId} /> }).then(result => result === true)

    const openPublished = { label: t('homestation.toast.viewPublished', 'View'), onClick: () => nav.navigate({ name: 'published' }, { reset: true }) }

    return {
      async toggleLike(view: CardView) {
        const on = !view.personal?.like.on
        if (on && !view.item.audience.restricted && !store.peekSettings().likeNoticeShown) {
          const ok = await confirm(
            t('homestation.like.noticeTitle', 'Likes are public'),
            t('homestation.like.noticeBody', 'Your like is published from your own HomeStation under a public audience. The author, your followers and collectors can see it and count it.'),
            t('homestation.like.noticeConfirm', 'Like publicly'),
            { notes: [t('homestation.like.noticeUndo', 'You can unlike later; that publishes a withdrawn state for the same like.'), t('homestation.like.noticeBookmark', 'Bookmarks stay private unless you choose to make one public.')], testId: 'hs-like-confirm' },
          )
          if (!ok) return
        }
        await store.setLike(view.item.objId, on)
        if (on && view.item.audience.restricted) toast({ text: t('homestation.like.restricted', 'This post has a restricted audience, so your like is delivered only to its author.') })
      },
      async toggleBookmark(view: CardView) {
        const on = !view.personal?.bookmark.on
        await store.setBookmark(view.item.objId, { on, public: false })
        toast({ text: on ? t('homestation.bookmark.addedPrivate', 'Bookmarked privately. Only you can see it.') : t('homestation.bookmark.removed', 'Removed from bookmarks.'), tone: on ? 'success' : 'info' })
      },
      async setBookmarkPublic(view: CardView, makePublic: boolean) {
        await store.setBookmark(view.item.objId, { on: true, public: makePublic })
        toast({ text: makePublic ? t('homestation.bookmark.public', 'Your bookmark is now public and shows up in your published interactions.') : t('homestation.bookmark.private', 'Your bookmark is private again. A withdrawn state was published for the public one.') })
      },
      async toggleReadLater(view: CardView) {
        const on = !view.personal?.readLater
        await store.setReadLater(view.item.objId, on)
        toast({ text: on ? t('homestation.readLater.added', 'Saved to Read later.') : t('homestation.readLater.removed', 'Removed from Read later.') })
      },
      async toggleDislike(view: CardView) {
        const on = !view.personal?.dislike
        await store.setDislike(view.item.objId, on)
        if (on) toast({ text: t('homestation.dislike.note', 'Noted locally. Dislikes are not published (pending decision).') })
      },
      async repost(view: CardView) {
        if (!view.canRepost) return
        const ok = await confirm(
          t('homestation.repost.confirmTitle', 'Repost to your homepage?'),
          t('homestation.repost.confirmBody', 'Your repost is a new post signed by you that wraps this exact version. It appears on your homepage and is delivered to your followers.'),
          t('homestation.repost.confirm', 'Repost'),
          { notes: [t('homestation.repost.confirmNote', 'Undoing a repost withdraws only your repost; copies already delivered can’t be recalled.')], testId: 'hs-repost-confirm' },
        )
        if (!ok) return
        await store.repost(view.item.objId, true)
        toast({ text: t('homestation.repost.done', 'Reposted. It’s in My publications.'), tone: 'success', action: openPublished })
      },
      async undoRepost(view: CardView) {
        await store.repost(view.item.objId, false)
        toast({ text: t('homestation.repost.undone', 'Repost withdrawn. The original is unchanged.') })
      },
      async quote(view: CardView) {
        if (!view.canRepost) return
        const result = await dialog.open<{ text: string; audience: AudienceSpec }>({
          title: t('homestation.quote.title', 'Quote repost'),
          size: 'md',
          renderBody: controls => <QuoteBody controls={controls} defaultAudience={store.peekSettings().defaultAudience} />,
        })
        if (!result) return
        await store.quote(view.item.objId, result.text, result.audience)
        toast({ text: t('homestation.quote.done', 'Quote published. It’s in My publications.'), tone: 'success', action: openPublished })
      },
      async mute(rule: MuteRule) {
        await store.setMuteRule(rule, true)
        toast({
          text: rule.kind === 'person'
            ? t('homestation.mute.person', 'You won’t see {{name}}’s posts in your feed or catch-up. Following and friendship are unchanged.', { name: rule.name })
            : t('homestation.mute.group', 'You won’t see posts from the group {{name}}. Following and friendship are unchanged.', { name: rule.name }),
          action: { label: t('homestation.toast.undo', 'Undo'), onClick: () => void store.setMuteRule(rule, false) },
        })
      },
      async lessLike(view: CardView) {
        await store.markLessLike(view.item.objId)
        toast({ text: t('homestation.lessLike', 'Got it — this only tunes your local recommendations.') })
      },
      async shareCapture(view: CardView) {
        const audience = store.peekSettings().defaultAudience
        const ok = await confirm(
          t('homestation.share.title', 'Share to your homepage?'),
          t('homestation.share.body', 'Your Spider’s private copy becomes a post signed by you that wraps the snapshot. The original author did not sign it, and the link to the source stays as provenance.'),
          t('homestation.share.confirm', 'Share'),
          { notes: [t('homestation.share.audience', 'Audience: {{label}}', { label: labelOf(audience) })], testId: 'hs-share-confirm' },
        )
        if (!ok) return
        const task = await store.shareCapture(view.item.objId)
        if (task.stage === 'failed') toast({ text: t('homestation.publish.failedToast', 'Publishing failed. Nothing was published.'), tone: 'warning' })
        else toast({ text: t('homestation.share.done', 'Shared to your homepage.'), tone: 'success', action: openPublished })
      },
      async copyObjId(objId: ObjId) {
        try {
          await navigator.clipboard.writeText(objId)
          toast({ text: t('homestation.copy.done', 'Object ID copied.'), tone: 'success' })
        } catch {
          toast({ text: objId })
        }
      },
      openLink(url: string) {
        window.open(url, '_blank', 'noopener,noreferrer')
      },
      async withdraw(entry: EntryUrl) {
        const ok = await confirm(
          t('homestation.withdraw.title', 'Withdraw this post?'),
          t('homestation.withdraw.body', 'Your HomeStation signs a withdrawn state for this entry and tells known receivers. HomeStations that follow the protocol stop showing it as current.'),
          t('homestation.withdraw.confirm', 'Withdraw'),
          { danger: true, notes: [t('homestation.withdraw.note', 'Copies others already fetched can’t be recalled. Comments and likes by others stay theirs.')], testId: 'hs-withdraw-confirm' },
        )
        if (!ok) return
        await store.withdraw(entry)
        toast({ text: t('homestation.withdraw.done', 'Withdrawn.') })
      },
      async edit(entry: EntryUrl, text: string) {
        const next = await dialog.open<string>({ title: t('homestation.edit.title', 'Edit post'), size: 'md', renderBody: controls => <EditBody controls={controls} initialText={text} /> })
        if (!next || next === text) return
        await store.editPost(entry, next)
        toast({ text: t('homestation.edit.done', 'New version published. Readers holding the old one will see “Updated”.'), tone: 'success' })
      },
      async changeAudience(entry: EntryUrl, current: AudienceSpec) {
        const next = await dialog.open<AudienceSpec>({ title: t('homestation.audience.dialogTitle', 'Change audience'), size: 'md', renderBody: controls => <AudienceBody controls={controls} current={current} t={t} groupsLabel={labelOf} /> })
        if (!next) return
        await store.setAudience(entry, next)
        toast({ text: isNarrowerAudience(next, current) ? t('homestation.audience.narrowed', 'Audience narrowed. Existing copies are not recalled.') : t('homestation.audience.changed', 'Audience updated.') })
      },
      async retryResources(objId: ObjId) {
        const state = await store.retryResources(objId)
        toast({ text: state === 'unavailable' ? t('homestation.resources.stillUnavailable', 'Still unavailable: none of the known holders answered.') : t('homestation.resources.ready', 'Ready to play.'), tone: state === 'unavailable' ? 'warning' : 'success' })
      },
    }
  }, [dialog, nav, store, t, toast])
}

export type ItemActions = ReturnType<typeof useItemActions>
