import { useCallback } from 'react'
import { useI18n } from '../../../i18n/provider'

/**
 * Strings of the Session Only pages (guest page, external customer invite).
 * They live with Message Hub until they move into `src/i18n/messagehub.ts`;
 * the global dictionaries win once they carry the same `messagehub.sso.*` key.
 */
const en = {
  loading: 'Opening the conversation…',
  invalidLink: 'This link is incomplete. Open the link you received again.',
  'invited.title': '{{group}} invited you to a conversation',
  'invited.body': 'Join to read the conversation and reply. Signing in alone does not add you to it.',
  'invited.join': 'Join conversation',
  'invited.joining': 'Joining…',
  'invited.failed': 'Could not join: the invitation may have been withdrawn. Contact the person who shared the link.',
  'lobby.title': 'Your conversations with {{group}}',
  'lobby.empty': 'You have no conversation here yet.',
  'lobby.start': 'Start a conversation',
  'lobby.starting': 'Starting…',
  'lobby.startFailed': 'Could not start a conversation. Try again later, or contact the person who shared the link.',
  'denied.title': 'This conversation is not available',
  'denied.body': 'You cannot access this conversation. Contact the person who shared it.',
  'error.title': 'The conversation could not be opened right now.',
  retry: 'Retry',
  reconnecting: 'Reconnecting…',
  archived: 'This conversation is archived and can no longer be answered.',
  conversation: 'Conversation',
  account: 'Account',
  signOut: 'Sign out',
  signingOut: 'Signing out…',
  signOutFailed: 'Could not sign out. Try again.',
  restrictedHub: 'Open a conversation from the link you were given.',
  home: 'Home',
  'invite.open': 'Invite external customer (private link)',
  'invite.intro': 'Creates a restricted account for the customer and a private sign-in link to this session. You send the link yourself, through the channel below.',
  'invite.name': 'Customer name',
  'invite.userId': 'Account ID',
  'invite.userIdHint': 'Lowercase letters, digits, ".", "_" or "-".',
  'invite.reuse': 'Reuse an existing customer account',
  'invite.reuseId': 'Existing account ID',
  'invite.channel': 'Channel',
  'invite.channel.email': 'Email',
  'invite.channel.sms': 'SMS',
  'invite.channel.other': 'Other',
  'invite.target': 'Masked contact (for your records)',
  'invite.targetPlaceholder': 'a***@example.com',
  'invite.create': 'Create link',
  'invite.creating': 'Creating…',
  'invite.cancel': 'Cancel',
  'invite.nameRequired': 'Enter the customer name.',
  'invite.invalidUserId': 'Enter a valid account ID.',
  'invite.targetRequired': 'Enter the masked contact.',
  'invite.failed': 'The link could not be created.',
  'invite.resourcePending': 'The account was created, but adding it to this session has not finished. Retry it in Users & Agents › Exclusive invites.',
} as const

export type GuestTextKey = keyof typeof en

const zh: Record<GuestTextKey, string> = {
  loading: '正在打开会话…',
  invalidLink: '链接不完整，请重新打开收到的链接。',
  'invited.title': '{{group}} 邀请你加入会话',
  'invited.body': '加入后即可查看会话内容并回复。仅登录不会自动加入会话。',
  'invited.join': '加入会话',
  'invited.joining': '正在加入…',
  'invited.failed': '无法加入：邀请可能已失效，请联系分享者。',
  'lobby.title': '你与 {{group}} 的会话',
  'lobby.empty': '你在这里还没有会话。',
  'lobby.start': '开始咨询',
  'lobby.starting': '正在创建…',
  'lobby.startFailed': '无法开始咨询，请稍后重试或联系分享者。',
  'denied.title': '无法访问此会话',
  'denied.body': '无法访问此会话，请联系分享者。',
  'error.title': '暂时无法打开此会话。',
  retry: '重试',
  reconnecting: '正在重新连接…',
  archived: '此会话已归档，不能再回复。',
  conversation: '会话',
  account: '账号',
  signOut: '退出登录',
  signingOut: '正在退出…',
  signOutFailed: '退出失败，请重试。',
  restrictedHub: '请通过收到的链接打开会话。',
  home: '返回首页',
  'invite.open': '邀请外部客户（专属链接）',
  'invite.intro': '为客户创建受限账号，并生成进入此会话的专属登录链接。链接由你通过下面的渠道自行发送。',
  'invite.name': '客户名称',
  'invite.userId': '账号 ID',
  'invite.userIdHint': '小写字母、数字、“.”、“_”或“-”。',
  'invite.reuse': '复用已有客户账号',
  'invite.reuseId': '已有账号 ID',
  'invite.channel': '发送渠道',
  'invite.channel.email': '邮箱',
  'invite.channel.sms': '短信',
  'invite.channel.other': '其它',
  'invite.target': '联系方式（脱敏，仅作记录）',
  'invite.targetPlaceholder': 'a***@example.com',
  'invite.create': '生成链接',
  'invite.creating': '正在生成…',
  'invite.cancel': '取消',
  'invite.nameRequired': '请输入客户名称。',
  'invite.invalidUserId': '请输入有效的账号 ID。',
  'invite.targetRequired': '请输入脱敏后的联系方式。',
  'invite.failed': '生成链接失败。',
  'invite.resourcePending': '账号已创建，但尚未成功加入此会话。请在“用户与 Agent › 专属邀请”中重试。',
}

export type GuestText = (key: GuestTextKey, variables?: Record<string, string | number>) => string

export function useGuestText(): GuestText {
  const { t, locale } = useI18n()
  return useCallback((key, variables) => t(`messagehub.sso.${key}`, locale === 'zh-CN' ? zh[key] : en[key], variables), [t, locale])
}
