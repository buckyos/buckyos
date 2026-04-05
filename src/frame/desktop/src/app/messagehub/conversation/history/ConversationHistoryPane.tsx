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
import type { DID } from '../../protocol/msgobj'
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
  statusItems?: readonly ConversationStatusDescriptor[]
}>(function ConversationHistoryPane({
  reader,
  selfDid,
  isGroup,
  statusItems,
}, ref) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const [viewportProfile, setViewportProfile] = useState<ViewportProfile>({
    isMobileViewport: false,
    visibleItemCount: DEFAULT_VISIBLE_ITEM_COUNT,
  })
  const [projection, setProjection] = useState<Awaited<ReturnType<typeof buildConversationProjection>> | null>(null)
  const [windowState, setWindowState] = useState<ConversationMaterializedWindow | null>(null)
  const previousTotalCountRef = useRef(0)
  const projectionRef = useRef<Awaited<ReturnType<typeof buildConversationProjection>> | null>(null)
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

    element.addEventListener('scroll', handleScroll, { passive: true })
    handleScroll()

    return () => {
      element.removeEventListener('scroll', handleScroll)
    }
  }, [hasProjection])

  useEffect(() => {
    const contentElement = contentRef.current
    if (!contentElement) {
      return
    }

    const resizeObserver = new ResizeObserver(() => {
      if (scrollModeRef.current === 'bottom-anchored') {
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
    const isAppendOnlyUpdate = Boolean(
      currentProjection
      && currentProjection.readerKey === reader.readerKey
      && currentProjection.statusItemsSignature === statusItemsSignature
      && reader.totalCount > currentProjection.messageCount
    )

    if (currentProjection
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

            return extendConversationProjection(activeProjection, remainingMessages, statusItems)
          })
        })
      })

      return () => {
        cancelled = true
      }
    }

    scrollModeRef.current = 'bottom-anchored'
    bottomAnchorLockUntilRef.current = 0
    cancelBottomAnchorRequest(bottomAnchorRequestIdRef)
    setProjection(null)
    setWindowState(null)

    void buildConversationProjection(reader, statusItems).then((nextProjection) => {
      if (cancelled) {
        return
      }

      startTransition(() => {
        setProjection(nextProjection)
      })
    })

    return () => {
      cancelled = true
    }
  }, [reader, statusItems])

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: projection?.totalCount ?? 0,
    getScrollElement: () => scrollRef.current,
    getItemKey: (index) => projection?.entries[index]?.key ?? index,
    estimateSize: (index) => {
      const entry = projection?.entries[index]
      return getListEntryEstimate(entry?.kind, itemsByIndex.get(index))
    },
    overscan: isMobileViewport
      ? Math.max(visibleItemCount * 5, 72)
      : Math.max(visibleItemCount * 3, 36),
    useFlushSync: false,
  })

  useEffect(() => {
    virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (_item, delta, instance) => {
      if (Math.abs(delta) < 4) {
        return false
      }

      if (instance.isScrolling) {
        return false
      }

      return instance.scrollDirection === 'backward'
    }
  }, [virtualizer])

  const virtualItems = virtualizer.getVirtualItems()
  const firstVirtualItem = virtualItems[0]

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
    if (!projection || projection.totalCount === 0) {
      previousTotalCountRef.current = 0
      return
    }

    const previousTotalCount = previousTotalCountRef.current
    const countGrew = projection.totalCount > previousTotalCount

    previousTotalCountRef.current = projection.totalCount

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
  }, [projection])

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
    <div className="relative h-full min-h-0 flex-1">
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
          <div
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100%',
              transform: `translateY(${firstVirtualItem?.start ?? 0}px)`,
            }}
          >
            {virtualItems.map((virtualItem) => {
              const item = itemsByIndex.get(virtualItem.index)

              return (
                <div
                  key={item?.key ?? `placeholder:${virtualItem.index}`}
                  ref={virtualizer.measureElement}
                  data-index={virtualItem.index}
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
  lockMs = 0,
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

function getListEntryEstimate(
  kind: ConversationListItem['kind'] | undefined,
  item?: ConversationListItem,
) {
  switch (kind) {
    case 'timestamp':
      return 52
    case 'status': {
      if (item?.kind === 'status') {
        return Math.min(72, 40 + Math.ceil(item.label.length / 32) * 12)
      }
      return 52
    }
    case 'message': {
      if (item?.kind === 'message') {
        const format = item.data.content.format ?? 'text/plain'
        if (format.startsWith('image/')) {
          return 280
        }

        const contentLength = item.data.content.content.length
        return Math.min(420, Math.max(132, 96 + Math.ceil(contentLength / 90) * 24))
      }

      return 180
    }
    default:
      return 180
  }
}
