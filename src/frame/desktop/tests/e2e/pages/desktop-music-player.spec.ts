import { expect, test, type Page } from '@playwright/test'
import { MOCK_DESKTOP_FOLDER_KEY, seedDesktopFolder } from '../fixtures/desktop'
import { pickZoneFolder, selectDialog, selectEmbedFrame } from '../fixtures/selectDialog'

/**
 * The music player widget over `/home/Music` in the mock Zone: two albums, four tracks and a cover image. The mock
 * reads every track as a 4 s WAV, so a track ends within a test.
 */
const MUSIC = ['Morning Walk/1 Sunrise.mp3', 'Morning Walk/2 Coffee.mp3', 'Morning Walk/10 Long Road.mp3', 'Morning Walk/cover.jpg', 'Night Drive/1 City Lights.wav']
const IN_ORDER = ['1 Sunrise', '2 Coffee', '10 Long Road', '1 City Lights']
const PLAYER = 'Music Player.widget'

/**
 * Puts the music into the mock Zone of each document of the page (the system picker's frame has its own) as soon as
 * the document offers its hook, before its app lists anything.
 */
async function seedMusic(page: Page, music: readonly string[] = MUSIC) {
  await page.addInitScript((paths) => {
    let hook: { put(path: string, base64: string): void } | undefined
    Object.defineProperty(window, '__mockZoneFiles', {
      configurable: true,
      get: () => hook,
      set(value: { put(path: string, base64: string): void }) {
        hook = value
        for (const path of paths) value.put(`/home/Music/${path}`, btoa('mock'))
      },
    })
  }, music)
}

async function seedPlayer(page: Page, config: Record<string, unknown> = {}, music: readonly string[] = MUSIC) {
  await seedMusic(page, music)
  const text = JSON.stringify({ widget: 'music-player', ...config })
  await seedDesktopFolder(page, { Files: 'files' }, { [PLAYER]: { kind: 'file', size: text.length, mtime: 1, text } })
}

async function widgetFile(page: Page): Promise<Record<string, unknown>> {
  return page.evaluate(([key, name]) => JSON.parse(JSON.parse(window.localStorage.getItem(key)!).entries[name].text), [MOCK_DESKTOP_FOLDER_KEY, PLAYER] as const)
}

const audio = (page: Page) =>
  page.getByTestId('music-audio').evaluate((element: HTMLAudioElement) => ({ paused: element.paused, time: element.currentTime }))

test.describe('music player widget', () => {
  test.use({ viewport: { width: 1280, height: 800 } })

  test('a folder chosen from the Zone plays its tracks in folder order, on to the next when one ends', async ({ page }) => {
    await seedPlayer(page)
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-status')).toContainText('Play a folder of music')

    await page.getByTestId('music-choose-folder').click()
    await pickZoneFolder(selectDialog(selectEmbedFrame(page)), '/home/Music')
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[0])
    await expect(page.getByTestId('music-count')).toHaveText('1/4')
    // Choosing the folder starts it; the cover image is not a track.
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
    expect(await widgetFile(page)).toEqual({ widget: 'music-player', folder: '/home/Music' })

    // Numbers in names sort as numbers; past the last track it starts over.
    for (const title of [...IN_ORDER.slice(1), IN_ORDER[0]]) {
      await page.getByTestId('music-next').click()
      await expect(page.getByTestId('music-title')).toHaveText(title)
    }
    await expect(page.getByTestId('music-folder')).toHaveText('Morning Walk')
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')

    // A track that ends goes on to the next one, still playing.
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[1], { timeout: 8000 })
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
  })

  test('closing the page leaves it paused where it was; the next visit picks up there', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Music' })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[0])
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'false')

    await page.getByTestId('music-next').click()
    await page.getByTestId('music-next').click()
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[2])
    await page.getByTestId('music-play').click()
    await expect.poll(async () => (await audio(page)).time, { timeout: 5000 }).toBeGreaterThan(1.5)

    await page.reload()
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[2])
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'false')
    await expect.poll(async () => (await audio(page)).time).toBeGreaterThan(1.2)
    const resumed = await audio(page)
    expect(resumed.paused).toBe(true)

    // Play goes on from there.
    await page.getByTestId('music-play').click()
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
    await expect.poll(async () => (await audio(page)).time).toBeGreaterThan(resumed.time)
  })

  test('shuffle keeps the current track, plays every track once per round, and is remembered', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Music' })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[0])

    const shuffle = page.getByTestId('music-shuffle')
    await expect(shuffle).toHaveAttribute('aria-pressed', 'false')
    await shuffle.click()
    await expect(shuffle).toHaveAttribute('aria-pressed', 'true')
    expect(await widgetFile(page)).toEqual({ widget: 'music-player', folder: '/home/Music', order: 'shuffle' })
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[0])

    // Back to the start of the shuffled round (paused at 0:00, Previous goes to the track before), then through it.
    for (let step = 0; step < 4 && (await page.getByTestId('music-count').textContent()) !== '1/4'; step++) {
      await page.getByTestId('music-previous').click()
    }
    await expect(page.getByTestId('music-count')).toHaveText('1/4')
    const round: string[] = []
    for (let position = 1; position <= 4; position++) {
      await expect(page.getByTestId('music-count')).toHaveText(`${position}/4`)
      round.push((await page.getByTestId('music-title').textContent())!)
      if (position < 4) await page.getByTestId('music-next').click()
    }
    expect([...round].sort()).toEqual([...IN_ORDER].sort())

    // The same round after a reload: on the last track, and the one before it is still the third.
    await page.reload()
    await expect(page.getByTestId('music-shuffle')).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('music-title')).toHaveText(round[3])
    await expect(page.getByTestId('music-count')).toHaveText('4/4')
    await page.getByTestId('music-previous').click()
    await expect(page.getByTestId('music-title')).toHaveText(round[2])

    // In order again from the same track.
    await page.getByTestId('music-shuffle').click()
    await expect(page.getByTestId('music-shuffle')).toHaveAttribute('aria-pressed', 'false')
    await expect(page.getByTestId('music-title')).toHaveText(round[2])
    await expect(page.getByTestId('music-count')).toHaveText(`${IN_ORDER.indexOf(round[2]) + 1}/4`)
  })

  // The mock serves an `.ogg` track as bytes no browser decodes.
  test('a track that cannot be played is skipped; a folder where none can says so', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Music' }, [...MUSIC, 'Morning Walk/3 Broken.ogg'])
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-count')).toHaveText('1/5')
    await page.getByTestId('music-next').click()
    await expect(page.getByTestId('music-title')).toHaveText('2 Coffee')
    await page.getByTestId('music-play').click()
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
    await page.getByTestId('music-next').click()
    await expect(page.getByTestId('music-title')).toHaveText('10 Long Road')
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
  })

  test('a folder where nothing can be played says so', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Music' }, ['Live/1 Opening.ogg', 'Live/2 Encore.ogg'])
    await page.goto('/?scenario=normal')
    // Found out while loading each track's metadata, before Play is pressed.
    await expect(page.getByTestId('music-status')).toContainText('The music in this folder cannot be played')
    await expect(page.getByTestId('music-choose-folder')).toHaveText('Choose another folder')
  })

  test('a folder without music says so and offers another', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Documents' })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-status')).toContainText('There is no music in this folder')
    await expect(page.getByTestId('music-choose-folder')).toHaveText('Choose another folder')
  })

  test('a folder that is gone says so', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Gone' })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-status')).toContainText('This folder cannot be opened')
    await expect(page.getByTestId('music-choose-folder')).toBeVisible()
  })
})

test.describe('music player widget on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

  test('taps play, skip and pause without dragging the widget', async ({ page }) => {
    await seedPlayer(page, { folder: '/home/Music' })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[0])
    const before = await page.getByTestId(`desktop-item-${PLAYER}`).boundingBox()

    await page.getByTestId('music-play').tap()
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'true')
    await page.getByTestId('music-next').tap()
    await expect(page.getByTestId('music-title')).toHaveText(IN_ORDER[1])
    await page.getByTestId('music-play').tap()
    await expect(page.getByTestId('music-play')).toHaveAttribute('data-playing', 'false')
    expect(await page.getByTestId(`desktop-item-${PLAYER}`).boundingBox()).toEqual(before)
  })
})
