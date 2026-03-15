import { buckyos, RuntimeType, parseSessionTokenClaims } from 'buckyos/node'

function getEnv(name) {
  const value = process.env[name]
  if (typeof value !== 'string') {
    return null
  }
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

async function main() {
  const appId = getEnv('BUCKYOS_TEST_APP_ID') ?? 'buckycli'
  const systemConfigServiceUrl = getEnv('BUCKYOS_SYSTEM_CONFIG_URL') ?? 'http://127.0.0.1:3200/kapi/system_config'

  const probeResponse = await fetch(systemConfigServiceUrl, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      method: 'sys_config_get',
      params: { key: 'boot/config' },
      sys: [1],
    }),
  })

  if (!probeResponse.ok) {
    throw new Error(`system_config probe failed: ${probeResponse.status} ${probeResponse.statusText}`)
  }

  await buckyos.initBuckyOS(appId, {
    appId,
    runtimeType: RuntimeType.AppClient,
    zoneHost: '',
    defaultProtocol: 'https://',
    systemConfigServiceUrl,
    privateKeySearchPaths: [
      '/opt/buckyos/etc',
      '/opt/buckyos',
      `${process.env.HOME ?? ''}/.buckycli`,
      `${process.env.HOME ?? ''}/.buckyos`,
    ],
  })

  const accountInfo = await buckyos.login()
  const bootConfig = await buckyos.getSystemConfigClient().get('boot/config')
  const tokenClaims = parseSessionTokenClaims(accountInfo?.session_token ?? null)
  const parsedBootConfig = JSON.parse(bootConfig.value)

  if (!accountInfo?.session_token) {
    throw new Error('app client login did not return a session_token')
  }

  console.log('AppClient login succeeded')
  console.log(JSON.stringify({
    appId,
    userId: accountInfo.user_id,
    tokenClaims,
    bootConfigKeys: Object.keys(parsedBootConfig),
  }, null, 2))

  buckyos.logout(false)
}

main().catch((error) => {
  console.error('AppClient demo failed')
  console.error(error)
  process.exitCode = 1
})
