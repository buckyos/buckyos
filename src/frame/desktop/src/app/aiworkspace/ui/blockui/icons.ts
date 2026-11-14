/* The icon names an extension may give its actions (`component.json` `actions[].icon`, PRD §10.1): a fixed set of
 * common glyphs, so a control's buttons look like the built-in ones. An unknown name shows the action's title. */

import {
  Bell, Calendar, Check, CirclePlay, CircleStop, Clock, Copy, Download, Eye, Flag, Heart, Image, Link2, List, Lock, MessageSquare, Pause,
  Pencil, Play, Plus, RefreshCw, RotateCcw, Search, Send, Settings, Share2, SkipForward, Sparkles, Square, Star, ThumbsUp, Trash2, Upload, Volume2,
  type LucideIcon,
} from 'lucide-react'

const ICONS: Record<string, LucideIcon> = {
  bell: Bell, calendar: Calendar, check: Check, 'circle-play': CirclePlay, stop: CircleStop, clock: Clock, copy: Copy, download: Download, eye: Eye,
  flag: Flag, heart: Heart, image: Image, link: Link2, list: List, lock: Lock, message: MessageSquare, pause: Pause, edit: Pencil, pencil: Pencil,
  play: Play, plus: Plus, add: Plus, refresh: RefreshCw, reset: RotateCcw, search: Search, send: Send, settings: Settings, share: Share2,
  next: SkipForward, 'skip-forward': SkipForward, sparkles: Sparkles, ai: Sparkles, square: Square, star: Star, like: ThumbsUp, 'thumbs-up': ThumbsUp,
  delete: Trash2, trash: Trash2, upload: Upload, volume: Volume2,
}

export function iconByName(name: string | undefined | null): LucideIcon | undefined {
  return name ? ICONS[name.toLowerCase()] : undefined
}
