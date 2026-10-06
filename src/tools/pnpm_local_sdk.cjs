const { existsSync } = require('node:fs')
const { resolve } = require('node:path')
const source = process.env.BUCKYOS_SDK_TOOL_SOURCE || resolve(__dirname, '../../../buckyos-websdk')
module.exports = { hooks: { readPackage(pkg) {
  if (pkg.dependencies?.buckyos && existsSync(resolve(source, 'package.json'))) pkg.dependencies.buckyos = `link:${source}`
  return pkg
} } }
