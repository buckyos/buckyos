# ext-fonts

The font package: fonts for printing (export to PDF) beyond the default fonts of
ext-core. It has no command. What it gives is fonts, each font family one online
item (`ext-manifest.json` `online`, `auto: false`): a node downloads one when an
administrator asks for it, in Desktop Settings > Extension tools or from the
print dialog, or with `buckyos ext-tools download ext-fonts <item>`.

## Not distributed

No font file is in this package, its payloads, its PIKG or a seed. Each online
item names the upstream's own release on GitHub, with the archive's SHA256 and
size, and a node downloads it from there and checks it. Every platform names the
same archive. We do not distribute the fonts and keep no copy or mirror.

Only fonts whose license allows embedding them in a PDF are listed: all are under
the SIL Open Font License 1.1 and every font file has `fsType` 0 (installable
embedding). Each item's `license` says whose release it is and where its license
text is. Source Sans 3's release archive has no license file; its text is
`LICENSE.md` of github.com/adobe-fonts/source-sans at the released tag, and the
fonts' own name table names the license.

The files of the package (`fonts.conf`, the generated `NOTICE.md`) are BuckyOS's,
AGPL-3.0-only.

## How fontconfig finds them

`fonts.conf` goes to `<tools>/store/ext-fonts/fonts.conf` and lists, for every
item, the directory of its static TrueType fonts inside `<tools>/online/<item>/`.
fontconfig resolves those relative directories from where the file really is,
`<tools>/versions/ext-fonts/<dir>/store/ext-fonts/`. ext-core (1.3.3 and later)
takes the file in from its own fontconfig setup (`conf.d/64-ext-fonts.conf`), so
`soffice` and the browser of `page-render` see what has been downloaded; an item
that is not downloaded is a directory that is not there. Only on Linux: Chromium
on macOS and Windows does not use fontconfig.

Workflow's `media.page_to_pdf`, `media.page_snapshot` and `media.to_pdf` put the
items that are ready (`<tools>/online/<item>.json`, `package: ext-fonts`,
`state: ready`) into their cache fingerprint when they run, so a download makes
the next print render again without a restart.

## Adding or updating a font

1. Find the family's newest release on its upstream's GitHub (for Noto, the
   `notofonts/<script>` repository's `Noto<Family>-v<version>` release), download
   the zip, take its SHA256 and size, check the license and that the fonts have
   `fsType` 0.
2. Add the item to `ext-manifest.json` (the same entry on every platform; `desc`
   starts with the family name as fontconfig reports it), its static TrueType
   directory to `fonts.conf`, its family to `PACK_FONTS` in
   `src/frame/aiworkspace/print/fonts.ts`, and its name to
   `settings.extTools.online.name.<item>` in
   `src/frame/desktop/src/i18n/settings.ts`, in English and Chinese.
3. Raise `version` in `ext-manifest.json` and `dapp_meta/app.json`, and the file
   name in `dapp_meta/pikg.json`.
4. Check:

   ```bash
   uv run python -m unittest discover -s publish/exttool/ext-fonts -p 'test_*.py'
   cd src && cargo test -p buckyos-api --lib -- --ignored --nocapture the_fonts_come_from_their_upstreams
   ```

   The second downloads every item through the node's own engine.

## Building and publishing

```bash
python3 publish/exttool/ext-fonts/build.py --archive    # payload/<platform>/ and dist/
```

Every platform's payload is the same but for the line of `NOTICE.md` that names
it, and a build gives the same bytes every time. A developer's machine seeds the
package with ext-core: `src/tools/prepare_ext_tools.py` builds it into
`src/rootfs/bin/ext-tools/` every time. `.github/workflows/ext-fonts.yml`
publishes a version on the tag `ext-fonts-v<version>`: the PIKG, the payloads and
`manifest.json` on GitHub and on R2 under `ext-tools/ext-fonts/<version>/`.
