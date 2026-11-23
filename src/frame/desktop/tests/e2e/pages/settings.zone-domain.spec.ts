import { expect, test, type Locator, type Page } from '@playwright/test'
import { pickLocal, pickZone, putZoneFile, selectDialog } from '../fixtures/selectDialog'

/** The mock node's state (api/zone_domain_mock.ts); left out, the Zone is on its default hostname. */
const STATE_KEY = 'buckyos.mock.zone-domain.v1'
const DOMAIN = 'home.example.com'
const soon = () => Math.floor(Date.now() / 1000) + 40 * 86_400

const certificates = (manager: 'acme' | 'manual', extra: object = {}) => [DOMAIN, `*.${DOMAIN}`].map((host) => ({
  host,
  loaded: true,
  not_after: soon(),
  issuer: manager === 'acme' ? "Let's Encrypt E6" : 'Example CA DV G2',
  fingerprint: 'sha256:aa',
  manager,
  acme: manager === 'acme' ? { state: 'ready', last_error: null, next_retry_at: null } : null,
  ...extra,
}))

const hosted = (mode: string, certificate: 'acme' | 'upload', extra: object = {}) => ({
  hostname: DOMAIN,
  hosting: { mode, certificate, domain: DOMAIN, ...(mode === 'dns_api' ? { provider: 'cloudflare', credential_id: 'home-example-com-1' } : {}) },
  credential: mode === 'dns_api' ? { id: 'home-example-com-1', provider: 'cloudflare' } : null,
  certificates: certificates(certificate === 'acme' ? 'acme' : 'manual'),
  ...extra,
})

async function openSection(page: Page, state?: object, account?: object): Promise<Locator> {
  await page.addInitScript(([key, value, zone]) => {
    if (value) localStorage.setItem(key as string, JSON.stringify(value))
    if (zone) localStorage.setItem('buckyos.mock.control-panel.v1', JSON.stringify(zone))
  }, [STATE_KEY, state, account] as const)
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await win.getByRole('button', { name: 'Zone Config', exact: true }).click()
  const section = win.getByTestId('settings-zone-domain')
  await expect(section).toBeVisible()
  return section
}

const file = (name: string, text: string) => ({ name, mimeType: 'application/octet-stream', buffer: Buffer.from(text) })

test.describe('settings: Zone domain and certificate', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 })
  })

  test('the default hostname shows only the name and its certificates', async ({ page }) => {
    const section = await openSection(page)
    await expect(section).toContainText('alice.web3.buckyos.ai')
    await expect(section.getByTestId('domain-cert-*.alice.web3.buckyos.ai')).toContainText('In use')
    await expect(section).toContainText('need no setup')
    for (const id of ['domain-change', 'domain-check', 'domain-replace-token', 'domain-replace-certificate']) {
      await expect(section.getByTestId(id)).toHaveCount(0)
    }
  })

  test('an administrator who is not the owner sees nothing to change', async ({ page }) => {
    const section = await openSection(page, hosted('sn', 'acme'), { zone_owner: 'bob' })
    await expect(section).toHaveText(/Only the Zone owner can see and change these settings\.$/)
  })

  test('status: hosting, certificate errors, problems and the checks with what is expected and found', async ({ page }) => {
    const section = await openSection(page, hosted('dns_api', 'acme', {
      problems: ['credential_rejected', 'something_new'],
      certificates: certificates('acme', { acme: { state: 'renewing', last_error: { message: 'urn:ietf:params:acme:error:dns', at: soon() }, next_retry_at: soon() } }),
    }))
    await expect(section).toContainText('Automatic, through your DNS provider')
    await expect(section).toContainText('Cloudflare')
    await expect(section.getByTestId('domain-problems')).toContainText('no longer accepts the saved token')
    await expect(section.getByTestId('domain-problems')).toContainText('something_new')
    const cert = section.getByTestId(`domain-cert-${DOMAIN}`)
    await expect(cert).toContainText('Being renewed')
    await expect(cert.getByText('urn:ietf:params:acme:error:dns')).toBeHidden()
    await cert.getByText('Details').click()
    await expect(cert.getByText('urn:ietf:params:acme:error:dns')).toBeVisible()

    await expect(section).toContainText('Not checked yet.')
    await section.getByTestId('domain-check').click()
    const entry = section.getByTestId('domain-check-entry')
    await expect(entry).toContainText('Action needed')
    await expect(entry).toContainText('ood1.home.example.com')
    await expect(entry).toContainText('An older record for this name overrides the * record.')
    await expect(entry).toContainText('CNAME alice.web3.buckyos.ai')
    await expect(entry).toContainText('A 198.51.100.4')
    await expect(section.getByTestId('domain-check-ownership')).toContainText('Passed')
  })

  test('status: a delegated domain, an order under way and a failed one are explained, raw messages are folded away', async ({ page }) => {
    const item = (id: string, status: string, code: string, detail: string | null = null) => ({ id, status, code, detail, expected: [], actual: [], targets: [] })
    const check = (...items: object[]) => ({ ready: false, bound: null, checked_at: soon(), items })
    const section = await openSection(page, hosted('sn', 'acme', {
      problems: ['credential_missing', 'renewal_failed'],
      check: check(
        item('ownership', 'waiting', 'txt_missing', 'delegated_to_sn'),
        item('delegation', 'waiting', 'bind_unavailable', 'connect error'),
        item('challenge', 'waiting', 'challenge_busy'),
        item('caa', 'fail', 'renewal_failed', 'home.example.com: urn:ietf:params:acme:error:dns.'),
      ),
    }))
    const ownership = section.getByTestId('domain-check-ownership')
    await expect(ownership).toContainText('Delete the NS record for now')
    await expect(ownership).not.toContainText('delegated_to_sn')
    await expect(ownership.getByText('Details')).toHaveCount(0)
    await expect(section.getByTestId('domain-check-delegation')).toContainText('The SN could not be reached.')
    await expect(section.getByTestId('domain-check-challenge')).toContainText('The gateway is getting a certificate right now.')
    const failed = section.getByTestId('domain-check-caa')
    await expect(failed).toContainText('The certificate could not be issued or renewed.')
    await expect(failed.getByText('urn:ietf:params:acme:error:dns')).toBeHidden()
    await failed.getByText('Details').click()
    await expect(failed.getByText('urn:ietf:params:acme:error:dns')).toBeVisible()
    await expect(section.getByTestId('domain-problems')).toContainText('The saved token is missing on the node.')
  })

  test('to the DNS provider: a refused token, a conflict to confirm, checks, then saving', async ({ page }) => {
    const section = await openSection(page, hosted('sn', 'acme', { taken: [DOMAIN] }))
    await section.getByTestId('domain-change').click()
    await section.getByTestId('domain-mode-dns_api').click()
    await section.getByTestId('domain-continue').click()

    await section.getByTestId('domain-provider').selectOption('hetzner')
    await expect(section).toContainText('reaches the whole account or project')
    await section.getByTestId('domain-provider').selectOption('cloudflare')
    await expect(section).not.toContainText('reaches the whole account or project')
    await expect(section.getByTestId('domain-token-link')).toHaveAttribute('href', 'https://dash.cloudflare.com/profile/api-tokens')
    await expect(section.getByTestId('domain-plan-wildcard')).toContainText('*.home.example.com')
    await expect(section.getByTestId('domain-write')).toBeDisabled()

    const token = section.getByTestId('domain-field-api_token')
    await expect(token).toHaveAttribute('type', 'password')
    await token.fill('REJECT-me')
    await section.getByTestId('domain-write').click()
    await expect(section.getByTestId('domain-results')).toContainText('The DNS provider did not accept the token.')
    await expect(token).toHaveValue('REJECT-me')
    await expect(section.getByTestId('domain-save')).toBeDisabled()

    await token.fill('good-token')
    await section.getByTestId('domain-write').click()
    await expect(section).toContainText('The DNS provider accepted the token.')
    await expect(section.getByTestId('domain-results')).toContainText('Not replaced')
    const conflict = section.getByTestId(`domain-conflict-${DOMAIN}`)
    await expect(conflict).toContainText('A 198.51.100.4')
    await conflict.getByRole('checkbox').check()
    await section.getByTestId('domain-write').click()
    await expect(section.getByTestId('domain-conflicts')).toHaveCount(0)
    await expect(section.getByTestId('domain-results')).not.toContainText('Not replaced')

    await expect(section.getByTestId('domain-check-entry')).toBeVisible()
    await section.getByTestId('domain-verify').click()
    await expect(section.getByTestId('domain-check-entry')).toContainText('Passed')
    await section.getByTestId('domain-save').click()
    await expect(section.getByTestId('domain-saved')).toContainText('The node now gets the certificate')
    await expect(section).toContainText('Automatic, through your DNS provider')
    expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain('good-token')
  })

  test('SN hosting shows the NS record only after the ownership has passed', async ({ page }) => {
    const section = await openSection(page, hosted('dns_api', 'acme', { verify_waits: 2 }))
    await section.getByTestId('domain-change').click()
    await section.getByTestId('domain-mode-sn').click()
    await section.getByTestId('domain-continue').click()
    await expect(section.getByTestId('domain-record-pkx')).toContainText('_pkx.home.example.com')
    await expect(section.getByTestId('domain-check-ownership')).toContainText('The TXT record is not visible yet.')
    await expect(section.getByTestId('domain-record-ns')).toHaveCount(0)
    await expect(section).toContainText('The NS record is shown here once the domain ownership has passed.')
    await section.getByTestId('domain-verify').click()
    await expect(section.getByTestId('domain-record-ns')).toContainText('dns.web3.buckyos.ai')
    const challenge = section.getByTestId('domain-check-challenge')
    await expect(challenge).toContainText('This is checked when you save.')
    await section.getByTestId('domain-verify').click()
    await expect(section.getByTestId('domain-save')).toBeEnabled()
  })

  test('manual: records with copy buttons, a certificate pasted and checked, leaving and coming back', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const section = await openSection(page, hosted('sn', 'acme'))
    await section.getByTestId('domain-change').click()
    await section.getByTestId('domain-mode-manual').click()
    await expect(section.getByTestId('domain-source-acme').getByRole('radio')).toBeDisabled()
    await expect(section.getByTestId('domain-source-upload').getByRole('radio')).toBeChecked()
    await section.getByTestId('domain-continue').click()

    await expect(section.getByTestId('domain-record-entry')).toContainText('alice.web3.buckyos.ai')
    await section.getByTestId('domain-copy-name-wildcard').click()
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('*.home.example.com')
    await section.getByTestId('domain-copy-value-wildcard').click()
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('alice.web3.buckyos.ai')

    await section.getByTestId('domain-certificate-paste').click()
    const text = section.getByTestId('domain-certificate-text')
    await text.fill('PARENT')
    await section.getByTestId('domain-certificate-check').click()
    await expect(section).toContainText('This certificate cannot be used.')
    await expect(section.getByTestId('domain-certificate-covers_wildcard')).toContainText('not names below it such as notes.home.example.com')
    await expect(section.getByTestId('domain-certificate-result')).toContainText('not covered')

    await text.fill('SELF')
    await expect(section.getByText('This certificate cannot be used.')).toHaveCount(0)
    await section.getByTestId('domain-certificate-check').click()
    await expect(section.getByTestId('domain-certificate-public_trust')).toContainText('self-signed')
    await section.getByTestId('domain-verify').click()
    await expect(section.getByTestId('domain-check-certificate')).toContainText('Passed')
    await expect(section.getByTestId('domain-check-entry')).toContainText('Passed')
    await expect(section.getByTestId('domain-save')).toBeDisabled()
    await section.getByTestId('domain-certificate-accept').click()
    await expect(section).toContainText('other Zones cannot resolve this Zone’s did:web')
    await expect(section.getByTestId('domain-save')).toBeEnabled()

    const win = page.getByTestId('window-settings')
    await win.getByRole('button', { name: 'General', exact: true }).click()
    await win.getByRole('button', { name: 'Zone Config', exact: true }).click()
    await expect(section.getByTestId('domain-record-entry')).toBeVisible()
    await expect(section.getByTestId('domain-save')).toBeEnabled()
    await section.getByTestId('domain-save').click()
    await expect(section.getByTestId('domain-saved')).toHaveText('Saved.')
    await expect(section).toContainText('Uploaded by you')
    await expect(section.getByTestId('domain-replace-certificate')).toBeVisible()
  })

  test('replacing the certificate: files from this device and from the Zone, and a PFX password', async ({ page }) => {
    const section = await openSection(page, hosted('manual', 'upload', { verify_waits: 9 }))
    await section.getByTestId('domain-replace-certificate').click()
    await expect(section.getByTestId('domain-checks')).toHaveCount(0)
    await expect(section.getByTestId('domain-save')).toBeDisabled()

    await section.getByTestId('domain-certificate-choose').click()
    await pickLocal(page, file('home.pfx', 'PFX'))
    await expect(section).toContainText('home.pfx')
    await section.getByTestId('domain-certificate-check').click()
    await expect(section.getByTestId('domain-certificate-readable')).toContainText('The PFX password is missing or wrong.')
    await section.getByTestId('domain-certificate-password').fill('secret')
    await section.getByTestId('domain-certificate-check').click()
    await expect(section).toContainText('This certificate can be used.')
    await expect(section).toContainText('Valid until')

    await section.getByRole('button', { name: 'Use another certificate' }).click()
    await putZoneFile(page, '/home/Documents/chain.pem', Buffer.from('chain'), 'application/x-pem-file')
    await putZoneFile(page, '/home/Documents/home.key', Buffer.from('key'), 'application/octet-stream')
    await section.getByTestId('domain-certificate-choose').click()
    await pickZone(selectDialog(page), ['/home/Documents/chain.pem', '/home/Documents/home.key'])
    await expect(section).toContainText('chain.pem')
    await section.getByRole('button', { name: 'Remove chain.pem' }).click()
    await expect(section.getByText('chain.pem')).toHaveCount(0)
    await section.getByTestId('domain-certificate-check').click()
    await section.getByTestId('domain-save').click()
    await expect(section.getByTestId('domain-saved')).toBeVisible()
  })

  test('a certificate over 1 MB is not read', async ({ page }) => {
    const section = await openSection(page, hosted('manual', 'upload'))
    await section.getByTestId('domain-replace-certificate').click()
    await section.getByTestId('domain-certificate-choose').click()
    await pickLocal(page, file('big.pem', 'x'.repeat(1024 * 1024 + 1)))
    await expect(section).toContainText('The files are larger than 1 MB in total.')
    await expect(section.getByTestId('domain-certificate-check')).toBeDisabled()
  })

  test('replacing the token needs the provider to accept it, not the other checks', async ({ page }) => {
    const section = await openSection(page, hosted('dns_api', 'acme', { problems: ['credential_rejected'], verify_waits: 9 }))
    await section.getByTestId('domain-replace-token').click()
    await expect(section.getByTestId('domain-provider')).toHaveValue('cloudflare')
    await section.getByTestId('domain-field-api_token').fill('REJECT')
    await section.getByTestId('domain-write').click()
    await expect(section.getByTestId('domain-save')).toBeDisabled()
    await section.getByTestId('domain-field-api_token').fill('fresh')
    await section.getByTestId('domain-write').click()
    await expect(section.getByTestId('domain-checks')).toHaveCount(0)
    await section.getByTestId('domain-save').click()
    await expect(section.getByTestId('domain-saved')).toBeVisible()
    await expect(section.getByTestId('domain-problems')).toHaveCount(0)
  })

  test('changing only the certificate source keeps the saved token', async ({ page }) => {
    const section = await openSection(page, hosted('dns_api', 'acme', { verify_waits: 0 }))
    await section.getByTestId('domain-change').click()
    await section.getByTestId('domain-source-upload').click()
    await section.getByTestId('domain-continue').click()
    await expect(section.getByTestId('domain-token-keep').getByRole('radio')).toBeChecked()
    await section.getByTestId('domain-write').click()
    await section.getByTestId('domain-certificate-paste').click()
    await section.getByTestId('domain-certificate-text').fill('a certificate')
    await section.getByTestId('domain-certificate-check').click()
    await expect(section.getByTestId('domain-check-certificate')).toContainText('Passed')
    await section.getByTestId('domain-save').click()
    await expect(section.getByTestId('domain-saved')).toHaveText('Saved.')
    await expect(section).toContainText('Uploaded by you')
  })

  test('cancel goes back to the status without changing anything', async ({ page }) => {
    const section = await openSection(page, hosted('sn', 'acme'))
    await section.getByTestId('domain-change').click()
    await section.getByTestId('domain-mode-manual').click()
    await section.getByTestId('domain-continue').click()
    await section.getByTestId('domain-back').click()
    await expect(section.getByTestId('domain-mode-manual').getByRole('radio')).toBeChecked()
    await section.getByRole('button', { name: 'Cancel' }).click()
    await expect(section).toContainText('Hosted by the SN')
    await expect(section.getByTestId('domain-change')).toBeVisible()
  })
})

test('phone: the domain section fits the width at every step', async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })
  const page = await context.newPage()
  await page.addInitScript(([key, value]) => localStorage.setItem(key as string, JSON.stringify(value)), [STATE_KEY, hosted('sn', 'acme', { taken: [DOMAIN] })] as const)
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').tap()
  await page.getByRole('button', { name: 'Zone Config', exact: true }).tap()
  const section = page.getByTestId('settings-zone-domain')
  const fits = async () => {
    const overflow = await section.evaluate((element) => element.scrollWidth - element.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390)
  }
  await expect(section.getByTestId('domain-change')).toBeVisible()
  await section.getByTestId('domain-check').tap()
  await expect(section.getByTestId('domain-check-entry')).toContainText('198.51.100.4')
  await fits()
  await section.getByTestId('domain-change').tap()
  await section.getByTestId('domain-mode-manual').tap()
  await fits()
  await section.getByTestId('domain-continue').tap()
  await expect(section.getByTestId('domain-record-pkx')).toBeVisible()
  await section.getByTestId('domain-certificate-paste').tap()
  await section.getByTestId('domain-certificate-text').fill('SELF')
  await section.getByTestId('domain-certificate-check').tap()
  await expect(section.getByTestId('domain-certificate-accept')).toBeVisible()
  await fits()
  await context.close()
})
