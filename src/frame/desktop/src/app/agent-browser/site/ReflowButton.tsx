/* "Reflow" beside the address of the original page. A page the owner already has (collected in the personal site,
 * being added, tidied before) opens in home mode at once. For any other page the owner says what is wanted: the
 * website as a lasting source of the site, the Agent doing something on this page, or this one page tidied.
 * `browser.page_hint` decides which comes first. */

import { BookOpen, FileText, LayoutList, ListChecks } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { PageHint } from '../api/types'
import { HandToAgentSheet } from '../conversation/HandoverSheets'
import { displayUrl } from '../shell/address'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser } from '../store/context'
import { RequestSheet } from './RequestSheet'
import { useSiteController } from './useSite'

type Choice = 'add' | 'task' | 'tidy'
interface Asked { url: string; title: string; hint: PageHint['hint'] }

const ORDER: Record<PageHint['hint'], Choice[]> = { reading: ['add', 'task', 'tidy'], unknown: ['add', 'task', 'tidy'], task: ['task', 'add', 'tidy'] }

function ChoiceSheet({ asked, onChoose, onClose }: { asked: Asked; onChoose: (choice: Choice) => void; onClose: () => void }) {
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  let host = asked.url
  try { host = new URL(asked.url).hostname } catch { /* shown as it is */ }
  const choices: Record<Choice, { icon: ReactNode; title: string; detail: string }> = {
    add: { icon: <BookOpen size={20} aria-hidden />, title: t('agentBrowser.site.reflow.add', 'Add to my reading'), detail: t('agentBrowser.site.reflow.addHint', '{{name}} keeps collecting {{site}}: new articles are ready to read in Home.', { name, site: host }) },
    task: { icon: <ListChecks size={20} aria-hidden />, title: t('agentBrowser.site.reflow.task', 'Ask {{name}} to do something here', { name }), detail: t('agentBrowser.site.reflow.taskHint', 'Hand it this page and say what you want done.') },
    tidy: { icon: <FileText size={20} aria-hidden />, title: t('agentBrowser.site.reflow.tidy', 'Tidy this page only'), detail: t('agentBrowser.site.reflow.tidyHint', 'A version of just this page that is easy to read on your phone.') },
  }
  return (
    <Sheet testId="ab-reflow-sheet" title={t('agentBrowser.site.reflow.title', 'Reflow')} onClose={onClose}>
      <p className="ab-page-ref" data-testid="ab-reflow-page">
        {asked.title ? <span className="ab-page-ref-title">{asked.title}</span> : null}
        <span className="ab-page-ref-url">{displayUrl(asked.url)}</span>
      </p>
      <ul className="ab-list" data-testid="ab-reflow-choices" data-hint={asked.hint}>
        {ORDER[asked.hint].map((choice, index) => (
          <li key={choice}>
            <button type="button" className="ab-list-item ab-choice" data-first={index === 0 ? 'true' : undefined} data-testid={`ab-reflow-choice-${choice}`} onClick={() => onChoose(choice)}>
              {choices[choice].icon}
              <span className="ab-list-text">
                <span className="ab-list-title">{choices[choice].title}</span>
                <span className="ab-list-detail">{choices[choice].detail}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </Sheet>
  )
}

export function ReflowButton() {
  const site = useSiteController()
  const { t } = useI18n()
  const url = useAgentBrowser((snapshot) => snapshot.state?.url ?? '')
  const title = useAgentBrowser((snapshot) => snapshot.state?.title ?? '')
  const [busy, setBusy] = useState(false)
  const [asked, setAsked] = useState<Asked | null>(null)
  const [step, setStep] = useState<'choose' | Choice>('choose')
  const press = async () => {
    setBusy(true)
    const outcome = await site.reflowPage(url, title)
    setBusy(false)
    if (outcome === 'opened') return
    setStep('choose')
    setAsked({ url, title, hint: outcome })
  }
  const close = () => setAsked(null)
  const label = t('agentBrowser.site.reflow.button', 'Reflow')
  return (
    <>
      <button type="button" className="ab-button ab-reflow-button" aria-label={label} aria-haspopup="dialog" title={label} disabled={busy || !/^https?:\/\//i.test(url)} data-testid="ab-reflow-button" onClick={() => void press()}>
        <LayoutList size={18} aria-hidden className={busy ? 'ab-spin' : undefined} /><span className="ab-reflow-button-label">{label}</span>
      </button>
      {!asked ? null
        : step === 'task' ? <HandToAgentSheet onClose={close} />
        : step === 'choose' ? <ChoiceSheet asked={asked} onChoose={setStep} onClose={close} />
        : <RequestSheet kind={step} url={asked.url} title={asked.title} onClose={close} />}
    </>
  )
}
