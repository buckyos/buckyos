const resetSearchParam = 'resetSiteData'
const indexedDbFallbackNames = ['buckyos-mock-message-history']

export function beginSiteDataReset() {
  const url = new URL(window.location.href)
  url.searchParams.set(resetSearchParam, '1')
  window.location.assign(url.toString())
}

export async function consumePendingSiteDataReset() {
  const url = new URL(window.location.href)
  if (url.searchParams.get(resetSearchParam) !== '1') {
    return false
  }

  await clearSiteData()
  url.searchParams.delete(resetSearchParam)
  window.location.replace(url.toString())
  return true
}

async function clearSiteData() {
  try {
    window.localStorage.clear()
  } catch {
    // Ignore storage cleanup failures and continue with the rest.
  }

  try {
    window.sessionStorage.clear()
  } catch {
    // Ignore storage cleanup failures and continue with the rest.
  }

  await Promise.allSettled([
    clearIndexedDbDatabases(),
    clearCacheStorage(),
    clearCookies(),
  ])
}

async function clearIndexedDbDatabases() {
  const databaseNames = await listIndexedDbDatabaseNames()
  await Promise.allSettled(databaseNames.map((name) => deleteIndexedDbDatabase(name)))
}

async function listIndexedDbDatabaseNames() {
  const indexedDbWithEnumeration = window.indexedDB as IDBFactory & {
    databases?: () => Promise<Array<{ name?: string | null }>>
  }

  if (typeof indexedDbWithEnumeration.databases !== 'function') {
    return indexedDbFallbackNames
  }

  try {
    const databases = await indexedDbWithEnumeration.databases()
    const names = databases
      .map((item) => item.name)
      .filter((name): name is string => typeof name === 'string' && name.length > 0)

    return names.length > 0 ? names : indexedDbFallbackNames
  } catch {
    return indexedDbFallbackNames
  }
}

function deleteIndexedDbDatabase(name: string) {
  return new Promise<void>((resolve, reject) => {
    const request = window.indexedDB.deleteDatabase(name)

    request.onerror = () => {
      reject(request.error ?? new Error(`Failed to delete IndexedDB database: ${name}`))
    }
    request.onblocked = () => {
      reject(new Error(`IndexedDB delete blocked: ${name}`))
    }
    request.onsuccess = () => {
      resolve()
    }
  })
}

async function clearCacheStorage() {
  if (!('caches' in window)) {
    return
  }

  const cacheKeys = await window.caches.keys()
  await Promise.allSettled(cacheKeys.map((key) => window.caches.delete(key)))
}

async function clearCookies() {
  if (!document.cookie) {
    return
  }

  const cookieNames = document.cookie
    .split(';')
    .map((entry) => entry.split('=')[0]?.trim())
    .filter((name): name is string => Boolean(name))

  const domains = buildCookieDomains(window.location.hostname)
  const paths = ['/', window.location.pathname]

  cookieNames.forEach((cookieName) => {
    domains.forEach((domain) => {
      paths.forEach((path) => {
        document.cookie = `${cookieName}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=${path}; domain=${domain}`
      })
    })

    paths.forEach((path) => {
      document.cookie = `${cookieName}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=${path}`
    })
  })
}

function buildCookieDomains(hostname: string) {
  const segments = hostname.split('.').filter(Boolean)
  const domains = new Set<string>()

  if (segments.length <= 1) {
    domains.add(hostname)
    return [...domains]
  }

  for (let index = 0; index < segments.length - 1; index += 1) {
    domains.add(segments.slice(index).join('.'))
    domains.add(`.${segments.slice(index).join('.')}`)
  }

  return [...domains]
}
