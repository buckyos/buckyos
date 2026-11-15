/* What the App writes to the Agent in the default conversation when a page has nothing to show, its refresh or an
 * action on the original page failed, the user wants a fallback page made again, or changed the preference (RF4). */

export type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

export type AskTopic = 'fallback' | 'refresh_failed' | 'redo' | 'action' | 'preference'

export interface AskDetail {
  /** The normalized address of the page. */
  url: string
  site: string
  host: string
  /** The user's preference for the site; empty when there is none. */
  preference: string
  /** Why the refresh or the action failed; empty when unknown. */
  reason: string
  /** The element of the original page an action was for. */
  selector: string
}

export function askText(t: Translate, topic: AskTopic, detail: AskDetail): string {
  const variables = { ...detail }
  const preference = detail.preference ? t('agentBrowser.reflow.ask.preferenceLine', 'My preference for this site: {{preference}}', variables) : ''
  const reason = detail.reason ? t('agentBrowser.reflow.ask.reasonLine', 'Reason: {{reason}}', variables) : ''
  const lines = topic === 'fallback' ? [
    t('agentBrowser.reflow.ask.fallback', 'Please make a reflow page for this address (a fallback page): {{url}}', variables),
    preference,
    t('agentBrowser.reflow.ask.fallbackDone', 'Keep it in my reflow library when it is done (reflow result put). I am waiting for it in the reflow view of the Agent browser.'),
  ] : topic === 'refresh_failed' ? [
    t('agentBrowser.reflow.ask.refreshFailed', 'The reflow refresh of this page failed: {{url}}', variables),
    reason,
    preference,
    t('agentBrowser.reflow.ask.refreshFailedDone', 'Please make a fallback page for it first so I can read it, then fix the reflow package of this site ({{site}}).', variables),
  ] : topic === 'redo' ? [
    t('agentBrowser.reflow.ask.redo', 'Please make the reflow page of this address again, from the site as it is now: {{url}}', variables),
    preference,
    t('agentBrowser.reflow.ask.redoDone', 'Keep it in my reflow library when it is done (reflow result put).'),
  ] : topic === 'action' ? [
    t('agentBrowser.reflow.ask.action', 'On the reflow page of {{url}} I pressed a control that has to be carried out on the original page, and it did not work.', variables),
    t('agentBrowser.reflow.ask.actionElement', 'The element on the original page: {{selector}}', variables),
    reason,
    t('agentBrowser.reflow.ask.actionDone', 'Please do it on the original page, then update the reflow result of this page.'),
  ] : [
    t('agentBrowser.reflow.ask.preference', 'I changed my reflow preference for {{host}}: {{preference}}', variables),
    t('agentBrowser.reflow.ask.preferenceDone', 'Please follow it from now on: if the site has a reflow package, change the package (the preference in its manifest and its code), and make this page again: {{url}}', variables),
  ]
  return lines.filter(Boolean).join('\n')
}
