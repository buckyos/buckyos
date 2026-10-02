import { expect, test } from '@playwright/test'

const taskId = 't-0123456789abcdef0123456789abcdef'

for (const force of [false, true]) {
  test(`real installation task cancellation forwards force=${force}`, async ({ page }) => {
    let canceled = false
    const cancellations: Record<string, unknown>[] = []
    await page.route('**/src/main.tsx*', async (route) => {
      const response = await route.fetch()
      const body = (await response.text()).replace(
        /if\s*\(!isMockRuntime\(\)\)/,
        'if (false)',
      )
      await route.fulfill({ response, body })
    })
    await page.route('**/src/runtime.ts*', async (route) => {
      await route.fulfill({
        contentType: 'text/javascript',
        body: `
        export const DESKTOP_USE_MOCK = true
        export const isMockRuntime = () => false
        export const waitForMockLatency = async () => {}
      `,
      })
    })
    await page.route('**/src/api/rpc.ts*', async (route) => {
      await route.fulfill({
        contentType: 'text/javascript',
        body: `
        export async function callRpc(method, params) {
          const response = await fetch('/__installer_rpc', {method: 'POST', body: JSON.stringify({method, params})})
          return {data: await response.json(), error: null}
        }
      `,
      })
    })
    await page.route('**/__installer_rpc', async (route) => {
      const { method, params } = route.request().postDataJSON()
      expect(params.task_id).toBe(taskId)
      if (method === 'apps.install.cancel') {
        cancellations.push(params)
        canceled = true
        await route.fulfill({
          json: {
            task_id: taskId,
            task_phase: 'Terminal',
            task_outcome: 'Canceled',
            mutation_released: true,
            cleanup_pending: force,
          },
        })
      } else {
        expect(method).toBe('apps.install.status')
        await route.fulfill({
          json: {
            schema_version: 4,
            task_id: taskId,
            task_phase: canceled ? 'Terminal' : 'Waiting',
            task_outcome: canceled ? 'Canceled' : undefined,
            app_name: 'Test App',
            stage: 'inspect',
            available_actions: canceled ? undefined : ['cancel'],
          },
        })
      }
    })
    await page.addInitScript(() =>
      localStorage.setItem('buckyos.prototype.locale.v1', 'en')
    )
    await page.goto(`/sysdlg/app_installer?task_id=${taskId}`)
    await expect(page.getByTestId('app-installer-live-task')).toBeVisible()
    await expect(page.getByText('Test App', { exact: false })).toBeVisible()
    await page.getByRole('button', {
      name: force ? 'Force cancel installation' : 'Cancel installation',
      exact: true,
    }).click()
    await expect(page.getByRole('heading', { name: 'Installation canceled' }))
      .toBeVisible()
    expect(cancellations).toEqual([{ task_id: taskId, force }])
    await expect(
      page.getByRole('button', { name: 'Cancel installation', exact: true }),
    ).toHaveCount(0)
    if (force) {
      await expect(
        page.getByText(
          'Installation canceled. Temporary files will be cleaned up automatically.',
        ),
      ).toBeVisible()
    }
  })
}
