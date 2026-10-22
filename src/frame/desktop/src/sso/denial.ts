/* ── What a denial (D-4) tells the visitor, and for admins where it is adjusted ──
 *
 * Branches only on `reason` / `detail` / `adjust` codes, never on error text. Visitors see the
 * category and whom to contact; a non-restricted viewer (an admin) also sees the adjust location.
 */

import type { AccessDenial, AdjustLocation } from 'buckyos'

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

function credentialText(t: Translate, detail: string | undefined): string {
  switch (detail) {
    case 'expired':
      return t('sso.denial.credential.expired', 'This link has expired.')
    case 'password_required':
      return t('sso.denial.credential.passwordRequired', 'This account has a password now. Sign in with it.')
    case 'link_replaced':
      return t('sso.denial.credential.linkReplaced', 'This link was replaced by a newer one. Use the latest link you received.')
    case 'password_unset':
      return t('sso.denial.credential.passwordUnset', 'This account has no password yet. Open the invitation link you received to sign in.')
    default:
      return t('sso.denial.credential', 'Your sign-in credential cannot be used. Follow the instructions on this page.')
  }
}

/** The visitor-side sentence of a denial. */
export function denialText(t: Translate, denial: AccessDenial): string {
  switch (denial.reason) {
    case 'session_kind_not_allowed':
      return t('sso.denial.sessionKindNotAllowed', 'This kind of account cannot use this feature.')
    case 'session_ceiling_exceeded':
      return t('sso.denial.sessionCeilingExceeded', 'Sign in again and retry.')
    case 'resource_grant_missing':
      return t('sso.denial.resourceGrantMissing', 'You do not have access to this. Contact the person who shared it.')
    case 'entry_unavailable':
      return t('sso.denial.entryUnavailable', 'This link is not available. Contact the person who shared it.')
    case 'zone_method_disabled':
      return t('sso.denial.zoneMethodDisabled', 'This sign-in method is not available right now. Contact the administrator.')
    case 'account_state':
      return t('sso.denial.accountState', 'This account is not available. Contact the administrator.')
    case 'credential_state':
      return credentialText(t, denial.detail)
    case 'source_revoked':
      return t('sso.denial.sourceRevoked', 'This invitation is no longer valid. Contact the person who shared it.')
    case 'authorization_unavailable':
      return t('sso.denial.authorizationUnavailable', 'Access cannot be confirmed right now. Try again later.')
  }
}

/** The Control Panel page or panel an adjust location names. */
export function adjustLocationLabel(t: Translate, location: AdjustLocation): string {
  switch (location) {
    case 'cp:sso_settings':
      return t('sso.adjust.location.ssoSettings', 'Settings › Sign-in & Sharing')
    case 'cp:users':
      return t('sso.adjust.location.users', 'Users & Agents')
    case 'cp:exclusive_invites':
      return t('sso.adjust.location.exclusiveInvites', 'Users & Agents › Exclusive invites')
    case 'share_panel':
      return t('sso.adjust.location.sharePanel', 'the share panel of the resource')
    case 'reauth':
      return t('sso.adjust.location.reauth', 'signing in again')
    case 'credential':
      return t('sso.adjust.location.credential', 'the sign-in credential')
  }
}

/** For admins: who adjusts this and where; `null` when nothing can be adjusted. */
export function denialAdjustText(t: Translate, denial: AccessDenial): string | null {
  const { party, location } = denial.adjust
  switch (party) {
    case 'product':
      return t('sso.adjust.product', 'Product boundary: this kind of account does not support the feature; there is no setting for it.')
    case 'none':
      return t('sso.adjust.none', 'A service it depends on is unavailable; nothing needs to be changed.')
    case 'self':
      return location
        ? t('sso.adjust.self', 'The visitor resolves this by {{location}}.', { location: adjustLocationLabel(t, location) })
        : null
    case 'sharer':
      return location
        ? t('sso.adjust.sharer', 'The sharer adjusts this in {{location}}.', { location: adjustLocationLabel(t, location) })
        : t('sso.adjust.sharerAnywhere', 'The sharer adjusts this.')
    case 'admin':
      return location
        ? t('sso.adjust.admin', 'An administrator adjusts this in {{location}}.', { location: adjustLocationLabel(t, location) })
        : t('sso.adjust.adminAnywhere', 'An administrator adjusts this.')
  }
}
