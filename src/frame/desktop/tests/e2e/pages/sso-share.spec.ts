import { expect, test } from '@playwright/test'

/** The mock control panel (src/api/control_panel_mock.ts) serves one invitation, directed to did:bns:bob. */
const SHARE = '/sso/share/inv-MockDirectedShareForBob000000000'

test.describe('resource invitation page', () => {
  test('another signed-in account must switch first; DID sign-in is not offered until then', async ({ page }) => {
    // The mock browser is signed in as Alice, and the DID callback already refused once.
    await page.goto(`${SHARE}?did_error=identity_conflict#comments`)
    const shareView = page.getByTestId('sso-share-page')
    await expect(shareView.getByText('Recipient: did:bns:bob')).toBeVisible()
    await expect(page.getByTestId('sso-share-interrupted')).toHaveText(
      'This browser is signed in as another account. Switch accounts, then sign in as the invited identity.',
    )
    await expect(page.getByTestId('sso-share-mismatch')).toBeVisible()
    await expect(page.getByTestId('sso-did-form')).toHaveCount(0)

    await page.getByTestId('sso-share-switch').click()

    await expect(page.getByTestId('sso-did-input')).toHaveValue('did:bns:bob')
    await expect(page.getByTestId('sso-share-switch')).toHaveCount(0)
    await expect(page.getByTestId('sso-share-mismatch')).toHaveCount(0)
    await expect(page.getByTestId('sso-share-interrupted')).toHaveCount(0)
    const url = new URL(page.url())
    expect(url.searchParams.has('did_error')).toBe(false)
    expect(url.hash).toBe('#comments')
  })

  test('other callback failures keep the invitation wording', async ({ page }) => {
    for (const reason of ['access_denied', 'oauth_validation_failed']) {
      await page.goto(`${SHARE}?did_error=${reason}`)
      await expect(page.getByTestId('sso-share-interrupted')).toHaveText(
        'Sign-in did not complete. You can retry this invitation or switch accounts.',
      )
    }
  })
})
