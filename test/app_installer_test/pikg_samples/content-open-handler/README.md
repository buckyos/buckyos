# Content Open Handler fixture

This independent static Web App declares a Markdown `open` handler at priority 40. It displays the URL request and App Frame request, sets the window title, and reads the first 1 KiB through `buckyos/nfsp`.

Build the sibling `buckyos-websdk` first, then run here:

```bash
pnpm install
pnpm build
node ../../../../src/rootfs/libexec/buckyos-tool/cli/launcher.mjs --allow-read dist pikg build dapp_meta
node ../../../../src/rootfs/libexec/buckyos-tool/cli/launcher.mjs pikg pack dapp_dist
```

The output is `dapp_dist/content-open-handler.buckyos.bns.did-0.1.0.pikg`. Install it through the existing local PIKG installer in an activated Zone. Files → Open with must list the fixture beside other Markdown handlers and Preview. Choose the fixture as the default, reopen the Markdown file, then restore the previous default. Uninstalling or disabling the fixture removes it from candidates after the registry refresh. Real Zone installation is a separate DV step; the Desktop mock tests exercise the same generic routing with two fixture app identities.
