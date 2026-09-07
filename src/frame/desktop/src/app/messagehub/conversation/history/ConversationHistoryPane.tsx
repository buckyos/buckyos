import { isActionMessage } from '../../sessionModel'
import { useI18n } from '../../../../i18n/provider'
import {
  forwardRef,
  memo,
  startTransition,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { getMessageStableId, type DID } from '../../protocol/msgobj'
import {
  buildConversationProjection,
  extendConversationProjection,
  materializeConversationWindow,
} from './data-source'
import { ConversationListRow } from './renderers'
import type {
  ConversationListItem,
  ConversationMaterializedWindow,
  ConversationMessageReader,
  ConversationStatusDescriptor,
} from './types'

type PaneProjection = Awaited<ReturnType<typeof buildConversationProjection>> & { readerRevision: number }

function getReaderRevision(reader: ConversationMessageReader): number {
  const value = (reader as { revision?: unknown }).revision
  return typeof value === 'number' ? value : 0
}

async function findMessageIndex(reader: ConversationMessageReader, messageId: string): Promise<number> {
  const pageSize = 128
  for (let start = 0; start < reader.totalCount; start += pageSize) {
    const messages = await reader.readRange(start, pageSize)
    const offset = messages.findIndex((message, index) => getMessageStableId(message, start + index) === messageId)
    if (offset >= 0) return start + offset
  }
  return -1
}

const DEFAULT_VISIBLE_ITEM_COUNT = 12
const BOTTOM_ANCHOR_THRESHOLD_PX = 24
const CONTENT_GROWTH_LOCK_MS = 240
const EXPLICIT_SCROLL_LOCK_MS = 1500

type ScrollMode = 'bottom-anchored' | 'free-scroll'

interface ViewportProfile {
  isMobileViewport: boolean
  visibleItemCount: number
}

export interface ConversationHistoryPaneHandle {
  scrollToBottom: () => void
}

const ConversationHistoryPaneInner = forwardRef<ConversationHistoryPaneHandle, {
  reader: ConversationMessageReader
  selfDid: DID
  isGroup: boolean
  showActions?: boolean
  emptyLabel?: string
  statusItems?: readonly ConversationStatusDescriptor[]
  hasOlder?: boolean
  onLoadOlder?: () => Promise<boolean>
  onVisibleMessages?: (recordIds: string[]) => void
}>(function ConversationHistoryPane({
  reader,
  selfDid,
  isGroup,
  statusItems,
  showActions = true,
  emptyLabel,
  hasOlder = false,
  onLoadOlder,
  onVisibleMessages,
}, ref) {
  const { t } = useI18n()
  const filterAnchor = useRef<{ messageIndex: number; messageId?: string; offset: number; targetIndex?: number } | null>(null)
  const loadingOlderRef = useRef(false)
  const visibleReportRef = useRef<string>('')
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const [viewportProfile, setViewportProfile] = useState<ViewportProfile>({
    isMobileViewport: false,
    visibleItemCount: DEFAULT_VISIBLE_ITEM_COUNT,
  })
  const [projection, setProjection] = useState<PaneProjection | null>(null)
  const [windowState, setWindowState] = useState<ConversationMaterializedWindow | null>(null)
  const previousTotalCountRef = useRef(0)
  const projectionRef = useRef<PaneProjection | null>(null)
  const scrollModeRef = useRef<ScrollMode>('bottom-anchored')
  const bottomAnchorLockUntilRef = useRef(0)
  const bottomAnchorRequestIdRef = useRef(0)
  const [showScrollToBottom, setShowScrollToBottom] = useState(false)
  const hasProjection = projection !== null
  const { isMobileViewport, visibleItemCount } = viewportProfile
  const itemsByIndex = useMemo(() => {
    const map = new Map<number, ConversationListItem>()

    windowState?.items.forEach((item) => {
      map.set(item.index, item)
    })

    return map
  }, [windowState])

  const windowItemsRef = useRef(itemsByIndex)
  useEffect(() => { windowItemsRef.current = itemsByIndex }, [itemsByIndex])

  useEffect(() => {
    const element = scrollRef.current
    if (!element) {
      return
    }

    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (entry) {
        setViewportProfile((previous) => {
          const next = getViewportProfile(entry.contentRect.width, entry.contentRect.height)

          return previous.isMobileViewport === next.isMobileViewport
            && previous.visibleItemCount === next.visibleItemCount
            ? previous
            : next
        })
      }
    })

    resizeObserver.observe(element)
    setViewportProfile(getViewportProfile(element.clientWidth, element.clientHeight))

    return () => {
      resizeObserver.disconnect()
    }
  }, [hasProjection])

  useEffect(() => {
    const element = scrollRef.current
    if (!element) {
      return
    }

    const handleScroll = () => {
      if (filterAnchor.current) return
      const isAnchored = isNearBottom(element, BOTTOM_ANCHOR_THRESHOLD_PX)

      if (isAnchored) {
        scrollModeRef.current = 'bottom-anchored'
        setShowScrollToBottom(false)
        return
      }

      if (Date.now() < bottomAnchorLockUntilRef.current) {
        return
      }

      scrollModeRef.current = 'free-scroll'
      setShowScrollToBottom(true)
      cancelBottomAnchorRequest(bottomAnchorRequestIdRef)
    }

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY >= 0) {
        return
      }

      scrollModeRef.current = 'free-scroll'
      bottomAnchorLockUntilRef.current = 0
      cancelBottomAnchorRequest(bottomAnchorRequestIdRef)
      setShowScrollToBottom(true)
    }

    element.addEventListener('scroll', handleScroll, { passive: true })
    element.addEventListener('wheel', handleWheel, { passive: true })
    handleScroll()

    return () => {
      element.removeEventListener('scroll', handleScroll)
      element.removeEventListener('wheel', handleWheel)
    }
  }, [hasProjection])

  useEffect(() => {
    const contentElement = contentRef.current
    if (!contentElement) {
      return
    }

    const resizeObserver = new ResizeObserver(() => {
      if (scrollModeRef.current === 'bottom-anchored') {
        bottomAnchorLockUntilRef.current = Math.max(
          bottomAnchorLockUntilRef.current,
          Date.now() + CONTENT_GROWTH_LOCK_MS,
        )
        stickToBottom(scrollRef.current)
      }
    })

    resizeObserver.observe(contentElement)

    return () => {
      resizeObserver.disconnect()
    }
  }, [hasProjection])

  useEffect(() => {
    projectionRef.current = projection
  }, [projection])

  useEffect(() => {
    let cancelled = false
    const currentProjection = projectionRef.current
    const statusItemsSignature = getStatusItemsSignature(statusItems)
    const readerRevision = getReaderRevision(reader)
    const revisionChanged = Boolean(currentProjection && currentProjection.readerKey === reader.readerKey && currentProjection.readerRevision !== readerRevision)
    const isAppendOnlyUpdate = Boolean(
      currentProjection
      && !revisionChanged
      && currentProjection.showActions === showActions
      && currentProjection.readerKey === reader.readerKey
      && currentProjection.statusItemsSignature === statusItemsSignature
      && reader.totalCount > currentProjection.messageCount
    )

    if (currentProjection
      && !revisionChanged
      && currentProjection.showActions === showActions
      && currentProjection.readerKey === reader.readerKey
      && currentProjection.statusItemsSignature === statusItemsSignature
      && reader.totalCount === currentProjection.messageCount) {
      return () => {
        cancelled = true
      }
    }

    if (isAppendOnlyUpdate && currentProjection) {
      const appendStartIndex = currentProjection.messageCount
      const appendedCount = reader.totalCount - currentProjection.messageCount

      void reader.readRange(appendStartIndex, appendedCount).then((messages) => {
        if (cancelled || messages.length !== appendedCount) {
          return
        }

        startTransition(() => {
          setProjection((activeProjection) => {
            if (!activeProjection) {
              return activeProjection
            }

            if (activeProjection.readerKey !== currentProjection.readerKey
              || activeProjection.statusItemsSignature !== statusItemsSignature) {
              return activeProjection
            }

            const consumedCount = Math.max(0, activeProjection.messageCount - appendStartIndex)
            const remainingMessages = messages.slice(consumedCount)

            if (remainingMessages.length === 0) {
              return activeProjection
            }

            return { ...extendConversationProjection(activeProjection, remainingMessages, statusItems), readerRevision: activeProjection.readerRevision }
          })
        })
      })

      return () => {
        cancelled = true
      }
    }

    const filterChanged = currentProjection?.readerKey === reader.readerKey && currentProjection.showActions !== showActions
    // A same-reader rebuild (filter toggle, prepended older page, record
    // update / removal) keeps the first visible message in place by its
    // stable id instead of resetting to the bottom.
    const keepAnchor = (filterChanged || revisionChanged) && scrollRef.current && scrollModeRef.current === 'free-scroll'
    if (keepAnchor && currentProjection) {
      const container = scrollRef.current!
      const row = [...container.querySelectorAll<HTMLElement>('[data-index]')].find(element => {
        const item = windowItemsRef.current.get(Number(element.dataset.index))
        return element.getBoundingClientRect().bottom > container.getBoundingClientRect().top && item?.kind === 'message' && !isActionMessage(item.data)
      })
      const entry = row ? currentProjection.entries[Number(row.dataset.index)] : undefined
      const item = row ? windowItemsRef.current.get(Number(row.dataset.index)) : undefined
      if (entry?.kind === 'message' && row) filterAnchor.current = { messageIndex: entry.messageIndex, messageId: item?.kind === 'message' ? getMessageStableId(item.data, entry.messageIndex) : undefined, offset: row.getBoundingClientRect().top - container.getBoundingClientRect().top }
    }
    if (!filterChanged && !revisionChanged) { scrollModeRef.current = 'bottom-anchored'; setProjection(null) }
    bottomAnchorLockUntilRef.current = 0
    cancelBottomAnchorRequest(bottomAnchorRequestIdRef)

    void buildConversationProjection(reader, statusItems, showActions).then(async (nextProjection) => {
      if (cancelled) {
        return
      }
      const anchor = filterAnchor.current
      if (anchor?.messageId) {
        // Resolve the anchored message's new raw index (older pages may have
        // been prepended in front of it).
        const index = await findMessageIndex(reader, anchor.messageId)
        if (cancelled) return
        if (index >= 0) anchor.messageIndex = index
      }

      startTransition(() => {
        setWindowState(null)
        setProjection({ ...nextProjection, readerRevision })
      })
    })

    return () => {
      cancelled = true
    }
  }, [reader, statusItems, showActions])

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: projection?.totalCount ?? 0,
    getScrollElement: () => scrollRef.current,
    getItemKey: (index) => projection?.entries[index]?.key ?? index,
    estimateSize: (index) => (
      projection?.entries[index]?.kind === 'message' ? 180 : 52
    ),
    measureElement: (element, entry) => (
      entry?.borderBoxSize[0]?.blockSize ?? element.getBoundingClientRect().height
    ),
    useAnimationFrameWithResizeObserver: true,
    overscan: isMobileViewport
      ? Math.max(visibleItemCount * 5, 72)
      : Math.max(visibleItemCount * 3, 36),
  })

  useEffect(() => {
    virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, instance) => (
      !filterAnchor.current
      && scrollModeRef.current === 'free-scroll'
      && item.end <= (instance.scrollOffset ?? 0) + instance.scrollAdjustments
    )
  }, [virtualizer])

  const virtualItems = virtualizer.getVirtualItems()

  useImperativeHandle(ref, () => ({
    scrollToBottom() {
      requestBottomAnchor(
        scrollModeRef,
        bottomAnchorLockUntilRef,
        bottomAnchorRequestIdRef,
        scrollRef.current,
        EXPLICIT_SCROLL_LOCK_MS,
      )
    },
  }), [])

  useEffect(() => {
    if (!projection || virtualItems.length === 0) {
      return
    }

    const firstVisibleIndex = virtualItems[0].index
    const lastVisibleIndex = virtualItems[virtualItems.length - 1].index
    const buffer = visibleItemCount * 2
    const startIndex = Math.max(0, firstVisibleIndex - buffer)
    const endIndex = Math.min(
      projection.totalCount,
      lastVisibleIndex + buffer + visibleItemCount,
    )

    if (hasWindowCoverage(windowState, startIndex, endIndex)) {
      return
    }

    let cancelled = false

    void materializeConversationWindow(
      projection,
      reader,
      startIndex,
      endIndex,
    ).then((nextWindow) => {
      if (!cancelled) {
        setWindowState(nextWindow)
      }
    })

    return () => {
      cancelled = true
    }
  }, [projection, reader, virtualItems, visibleItemCount, windowState])

  useEffect(() => {
    if (!projection || virtualItems.length === 0) return
    // Older history: request the previous page when the top of the loaded
    // range comes into view. The anchor logic above keeps the viewport still.
    if (hasOlder && onLoadOlder && !loadingOlderRef.current && virtualItems[0].index <= 4 && scrollModeRef.current === 'free-scroll') {
      loadingOlderRef.current = true
      void onLoadOlder().finally(() => { loadingOlderRef.current = false })
    }
    if (onVisibleMessages) {
      const ids = virtualItems
        .map((virtualItem) => itemsByIndex.get(virtualItem.index))
        .filter((item): item is Extract<ConversationListItem, { kind: 'message' }> => item?.kind === 'message')
        .map((item) => getMessageStableId(item.data, item.messageIndex))
      const signature = ids.join('|')
      if (signature && signature !== visibleReportRef.current) {
        visibleReportRef.current = signature
        onVisibleMessages(ids)
      }
    }
  }, [projection, virtualItems, itemsByIndex, hasOlder, onLoadOlder, onVisibleMessages])

  useEffect(() => {
    if (!projection || projection.totalCount === 0) {
      previousTotalCountRef.current = 0
      return
    }

    const previousTotalCount = previousTotalCountRef.current
    const countGrew = projection.totalCount > previousTotalCount

    previousTotalCountRef.current = projection.totalCount

    if (filterAnchor.current) {
      const anchor = filterAnchor.current
      const index = projection.entries.findIndex(entry => entry.kind === 'message' && entry.messageIndex >= anchor.messageIndex)
      if (index >= 0) {
        anchor.targetIndex = index
        const frame = requestAnimationFrame(() => virtualizer.scrollToIndex(index, { align: 'start' }))
        return () => cancelAnimationFrame(frame)
      } else filterAnchor.current = null
      return
    }
    if (previousTotalCount === 0) {
      requestBottomAnchor(
        scrollModeRef,
        bottomAnchorLockUntilRef,
        bottomAnchorRequestIdRef,
        scrollRef.current,
      )
      return
    }

    if (countGrew && scrollModeRef.current === 'bottom-anchored') {
      requestBottomAnchor(
        scrollModeRef,
        bottomAnchorLockUntilRef,
        bottomAnchorRequestIdRef,
        scrollRef.current,
        CONTENT_GROWTH_LOCK_MS,
      )
    }
  }, [projection, virtualizer])

  useEffect(() => {
    const anchor = filterAnchor.current, container = scrollRef.current
    if (!anchor || anchor.targetIndex === undefined || !container) return
    let frame = 0, remaining = 8
    const restore = () => {
      const row = container.querySelector<HTMLElement>(`[data-index="${anchor.targetIndex}"][data-message-index]`)
      if (row) container.scrollTop += row.getBoundingClientRect().top - container.getBoundingClientRect().top - anchor.offset
      remaining--
      if (remaining > 0) frame = requestAnimationFrame(restore)
      else { filterAnchor.current = null; scrollModeRef.current = 'free-scroll' }
    }
    frame = requestAnimationFrame(restore)
    return () => cancelAnimationFrame(frame)
  }, [windowState])

  const handleScrollToBottomClick = () => {
    setShowScrollToBottom(false)
    requestBottomAnchor(
      scrollModeRef,
      bottomAnchorLockUntilRef,
      bottomAnchorRequestIdRef,
      scrollRef.current,
      EXPLICIT_SCROLL_LOCK_MS,
    )
  }

  if (!projection) {
    return (
      <div
        className="h-full min-h-0 flex-1 overflow-hidden px-3 py-2"
        style={{ background: 'var(--cp-bg)' }}
      >
        <div className="h-full animate-pulse rounded-3xl" style={{
          background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)',
        }}
        />
      </div>
    )
  }

  return (
    <div className="relative h-full min-h-0 flex-1" data-testid="conversation-history" data-raw-count={reader.totalCount} data-visible-count={projection.totalCount}>
      {projection.totalCount === 0 && <p className="absolute inset-0 flex items-center justify-center text-sm text-[color:var(--cp-muted)]" role="status">{reader.totalCount ? t('messagehub.filteredMessages') : emptyLabel ?? t('messagehub.noMessages')}</p>}
      <div
        ref={scrollRef}
        className="h-full overflow-y-auto px-3 py-2 shell-scrollbar"
        style={{
          contain: 'strict',
          overflowAnchor: 'none',
          WebkitOverflowScrolling: 'touch',
        }}
      >
        <div
          ref={contentRef}
          style={{
            height: virtualizer.getTotalSize(),
            position: 'relative',
            width: '100%',
          }}
        >
          {virtualItems.map((virtualItem) => {
            const item = itemsByIndex.get(virtualItem.index)

            return (
              <div
                key={virtualItem.key}
                ref={item ? virtualizer.measureElement : undefined}
                data-index={virtualItem.index}
                data-message-index={item?.kind === 'message' ? item.messageIndex : undefined}
                className="flow-root"
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  transform: `translateY(${virtualItem.start}px)`,
                  height: item ? undefined : virtualItem.size,
                }}
              >
                {item ? (
                  <ConversationListRow
                    item={item}
                    isGroup={isGroup}
                    selfDid={selfDid}
                  />
                ) : (
                  <ListItemPlaceholder />
                )}
              </div>
            )
          })}
        </div>
      </div>
      {showScrollToBottom && (
        <button
          type="button"
          onClick={handleScrollToBottomClick}
          className="absolute right-4 bottom-4 flex h-9 w-9 items-center justify-center rounded-full shadow-lg transition-opacity hover:opacity-80 active:scale-95"
          style={{
            background: 'var(--cp-surface)',
            color: 'var(--cp-text)',
            border: '1px solid color-mix(in srgb, var(--cp-text) 12%, transparent)',
            zIndex: 10,
          }}
          aria-label="Scroll to bottom"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </button>
      )}
    </div>
  )
})

ConversationHistoryPaneInner.displayName = 'ConversationHistoryPane'

export const ConversationHistoryPane = memo(ConversationHistoryPaneInner)

function getStatusItemsSignature(
  statusItems: readonly ConversationStatusDescriptor[] = [],
) {
  return statusItems.map((item) => (
    `${item.id}:${item.position ?? 'tail'}:${item.status}:${item.label}:${item.createdAtMs ?? ''}`
  )).join('|')
}

function getViewportProfile(width: number, height: number): ViewportProfile {
  const isMobileViewport = width > 0 && width < 769
  const effectiveViewportHeight = isMobileViewport
    ? Math.round(height * 7)
    : height

  return {
    isMobileViewport,
    visibleItemCount: Math.max(
      DEFAULT_VISIBLE_ITEM_COUNT,
      Math.ceil(effectiveViewportHeight / 88),
    ),
  }
}

function stickToBottom(scrollElement: HTMLDivElement | null) {
  if (!scrollElement) {
    return
  }

  scrollElement.scrollTop = scrollElement.scrollHeight
}

function requestBottomAnchor(
  scrollModeRef: { current: ScrollMode },
  bottomAnchorLockUntilRef: { current: number },
  bottomAnchorRequestIdRef: { current: number },
  scrollElement: HTMLDivElement | null,
  lockMs = EXPLICIT_SCROLL_LOCK_MS,
) {
  scrollModeRef.current = 'bottom-anchored'
  bottomAnchorLockUntilRef.current = lockMs > 0 ? Date.now() + lockMs : 0
  const requestId = bottomAnchorRequestIdRef.current + 1
  bottomAnchorRequestIdRef.current = requestId
  scheduleBottomAnchor(
    scrollModeRef,
    bottomAnchorRequestIdRef,
    requestId,
    scrollElement,
    [0, 32, 80, 160, 320, 520],
  )
}

function scheduleBottomAnchor(
  scrollModeRef: { current: ScrollMode },
  bottomAnchorRequestIdRef: { current: number },
  requestId: number,
  scrollElement: HTMLDivElement | null,
  delaysMs: readonly number[],
) {
  delaysMs.forEach((delayMs) => {
    window.setTimeout(() => {
      if (
        scrollModeRef.current !== 'bottom-anchored'
        || bottomAnchorRequestIdRef.current !== requestId
      ) {
        return
      }

      stickToBottom(scrollElement)
    }, delayMs)
  })
}

function cancelBottomAnchorRequest(
  bottomAnchorRequestIdRef: { current: number },
) {
  bottomAnchorRequestIdRef.current += 1
}

function isNearBottom(
  scrollElement: HTMLDivElement | null,
  thresholdPx: number,
) {
  if (!scrollElement) {
    return false
  }

  return getDistanceToBottom(scrollElement) <= thresholdPx
}

function getDistanceToBottom(scrollElement: HTMLDivElement) {
  return scrollElement.scrollHeight - scrollElement.clientHeight - scrollElement.scrollTop
}

function hasWindowCoverage(
  windowState: ConversationMaterializedWindow | null,
  startIndex: number,
  endIndex: number,
) {
  if (!windowState) {
    return false
  }

  return (
    windowState.startIndex <= startIndex
    && windowState.endIndex >= endIndex
  )
}

function ListItemPlaceholder() {
  return (
    <div className="py-2">
      <div
        className="h-14 rounded-3xl"
        style={{
          background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)',
        }}
      />
    </div>
  )
}
