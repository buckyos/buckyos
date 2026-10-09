/* Cross-node objects of HomeStation architecture v0.6 §5.8–§5.11. Field names follow the
   architecture examples; they are not a frozen wire format. */

export type ObjId = string
export type Did = string
export type EntryUrl = string

export type FeedObjectKind = 'post' | 'comment'
export type CommentType = 'text' | 'like' | 'bookmark' | 'repost' | 'quote'
export type ContentType = 'text' | 'image' | 'video' | 'audio' | 'article' | 'link' | 'product'
export type PublicationCategory = 'work' | 'product'

export interface MediaPart {
  object: ObjId
  alt?: string
}

export interface FeedContent {
  type: ContentType
  text?: string
  title?: string
  summary?: string
  cover?: ObjId
  media?: MediaPart[]
}

export interface FeedReference {
  relation: 'comment_on'
  object_id: ObjId
}

export interface FeedSource {
  kind: 'web' | 'rss' | 'platform'
  original_url: string
  original_author?: string
  captured_at_ms: number
}

export interface FeedObject {
  kind: FeedObjectKind
  comment_type?: CommentType
  publisher: Did
  iat: number
  entry?: EntryUrl
  content?: FeedContent
  wraps?: ObjId
  references?: FeedReference[]
  tags?: string[]
  source?: FeedSource
  link?: string
  publication_category?: PublicationCategory
  base_on?: ObjId
}

export type HeadState = 'active' | 'withdrawn'

export interface FeedHead {
  kind: 'feed_head'
  entry: EntryUrl
  seq: number
  state: HeadState
  current?: ObjId
  updated_at_ms: number
}

export interface FollowDeclaration {
  kind: 'follow'
  publisher: Did
  iat: number
  entry: EntryUrl
  target: { publisher: Did; stream: string }
}

export interface FileObject {
  kind: 'file'
  name: string
  meta: {
    mime: string
    size: number
    width?: number
    height?: number
    duration_ms?: number
  }
}

export function commentTarget(object: FeedObject): ObjId | undefined {
  if (object.comment_type === 'repost' || object.comment_type === 'quote') return object.wraps
  return object.references?.find(reference => reference.relation === 'comment_on')?.object_id
}
