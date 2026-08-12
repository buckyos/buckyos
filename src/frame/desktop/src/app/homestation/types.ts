/* ── Content Types ── */

export type FeedContentType = 'text' | 'image' | 'video' | 'link' | 'article' | 'product'
export type ReadingMode = 'standard' | 'image' | 'longform' | 'immersive-video'
export type ViewPerspective = 'owner' | 'visitor'

/* ── FeedObject ── */

export interface FeedAuthor {
  id: string
  name: string
  avatar?: string
  sourceType: 'did' | 'platform' | 'rss' | 'agent'
  isVerified?: boolean
}

export interface FeedMedia {
  type: 'image' | 'video' | 'audio'
  url: string
  thumbnailUrl?: string
  width?: number
  height?: number
  durationMs?: number
  alt?: string
}

export interface FeedInteractions {
  likeCount: number
  commentCount: number
  repostCount: number
  isLiked: boolean
  isBookmarked: boolean
  isReposted: boolean
}

export interface FeedObject {
  id: string
  author: FeedAuthor
  contentType: FeedContentType
  title?: string
  text?: string
  body?: string
  media: FeedMedia[]
  originalUrl?: string
  recommendReason?: string
  topics: string[]
  interactions: FeedInteractions
  createdAt: number
  sourceId: string
}

/* ── Source ── */

export type SourceType = 'person' | 'channel' | 'rss' | 'website' | 'topic' | 'agent-curated'

export interface Source {
  id: string
  name: string
  type: SourceType
  avatar?: string
  url?: string
  description?: string
  isFollowing: boolean
  feedCount: number
  lastUpdatedAt: number
}

/* ── Topic ── */

export interface Topic {
  id: string
  name: string
  icon?: string
  feedCount: number
  isSubscribed: boolean
  trendScore?: number
}

/* ── Filter ── */

export type FeedFilter = 'all' | 'following' | 'images' | 'videos' | 'longform' | 'news'

export interface ActiveFilterState {
  filter: FeedFilter
  topicId: string | null
  readingMode: ReadingMode
}

/* ── Public Profile ── */

export type ProfileTab = 'posts' | 'works' | 'products' | 'featured'

export interface UserProfile {
  id: string
  name: string
  avatar?: string
  coverImage?: string
  bio?: string
  followerCount: number
  followingCount: number
  postCount: number
}

/* ── View State ── */

export type MobileView = 'feed' | 'topics' | 'publish' | 'sources' | 'profile' | 'detail' | 'immersive'
export type MobileBottomTab = 'home' | 'topics' | 'publish' | 'sources' | 'me'

export interface HomeStationState {
  perspective: ViewPerspective
  activeFilter: ActiveFilterState
  mobileView: MobileView
  mobileBottomTab: MobileBottomTab
  selectedFeedId: string | null
  readingMode: ReadingMode
  showSidebar: boolean
  showInfoPanel: boolean
  searchQuery: string
}
