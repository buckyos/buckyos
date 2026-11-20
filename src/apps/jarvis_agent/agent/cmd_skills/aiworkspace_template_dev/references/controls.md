#### Build, import and upgrade controls
Start with `component.json`, its entries and `AGENT.md` in a local or Zone folder. Full contracts in `src/frame/aiworkspace/README.md` and `schemas/{component,composition,migration,template}.schema.json`. Check `buckyos aiws <verb> --help` for flags.
Set `ME` to the owner's name, `WS` to the source workspace and `OTHER_WS` to the destination. At each step, substitute the IDs returned by the preceding command. `DIR` may also name a Zone folder. For a composite skeleton, add `--composite` to `pikg init`.

```bash
buckyos pikg init /tmp/kpi --owner "did:bns:$ME" --kind aiws-controls --source ./aiws
DIR=/tmp/kpi/aiws/controls/kpi
buckyos aiws build "$DIR" --workspace "$WS" --dry-run
buckyos aiws build "$DIR" --workspace "$WS"
# DEF is the returned def_entity_id. Pull the workspace version to a local folder.
buckyos aiws pull "$DEF" --workspace "$WS" --out "$DIR" --local
buckyos aiws ext-export "$DEF" --workspace "$WS" --out ./pack --name kpi-pack --version 0.1.0 --install
buckyos aiws library list
# PKG_ID and IDENT (<author_did>#<name>) are the library entry.
buckyos aiws ext-import "$PKG_ID#$IDENT" --workspace "$OTHER_WS"
# DEST_DEF is the imported definition ID. Preview an upgrade after the library changes.
buckyos aiws ext-upgrade "$DEST_DEF" --workspace "$OTHER_WS" --dry-run
```

For a template, set `SURFACE` to the chosen surface ID and `TABLE` to the table whose rows should reset. Use the installed package ID from `template list`.

```bash
buckyos aiws template export --workspace "$WS" --surfaces "$SURFACE" --id team-board --title "Team board" --param team_name:string:Team --reset "$TABLE" --out ./tpl --name team-kit --version 0.1.0 --install
buckyos aiws template list
buckyos aiws template create "$TEMPLATE_PKG_ID#team-board" --title "Red team" --param team_name=Red
```

`build --watch` rebuilds a Zone folder on save. One build can send up to 4 MiB total inline. If its source exceeds that, the client uploads the largest files first and sends references. Zone files already stored as objects go by reference. `pull` retrieves source from a workspace definition, including wish-made HTML. `.aiws-build.json` tracks the base. On `REVISION_CONFLICT`, `BUILD_BASE_STALE` or `BUILD_BASE_REQUIRED` pull, merge and build. `build --force` applies the supplied source over the workspace version. `BUILD_FAILED` lists `data.diagnostics` with file, line, code, instances and dependents; it commits nothing. `DEP_MISSING` requires building/importing the dependency first.

#### Define and place
`component.json` (manifest version 5, accepts 1–4) names `ident:{author_did,name}` (name matches `[a-z0-9][a-z0-9._-]{0,63}`), `kind`, `title`, `entry`. Declarative entry is JSON, HTML uses `{html,css?,js?}`, composite uses `composition.json`. Include `config_schema`, `bindings` (accepts/required/many/schema_id), `default_size`, `catalog`, `agent`, `assets` and `deps` as applicable. Filenames are relative with no dot-prefixed components. `accepts` and `allow_no_source` apply to render controls. Use `doc/workspace/control-examples/` for examples.

- **Render.** An instance is `buckyos.cell` with `view.type`, `def_ref.entity_id`, named references or lists in `bindings`, `config` overrides and `on` handlers. Declarative items bind `sum:amount`, `count`, a field, `props.<key>` or `<binding>.count`. HTML uses `aiws.input(name)` and `aiws.context.config`. Check `aiws.asset` exists before getting an asset blob URL. Writing HTML control code needs manage. A wish-generated HTML result is trusted automatically only when its run identity manages it; otherwise a manager sets `trusted:true`.
- **Composite.** A container lays out real child Blocks in named slots with `layout` (stack/grid/fixed), `slots` (accepts/min/max/placement), and `chrome`. `{{config.<key>}}` interpolates properties. See the content reference for instance and clone operations.
- **Events and actions.** Declare typed `events` in the manifest. With `host_api:3` use `aiws.emit(name,params)`, declarative `button.emit` or composite `events.from:{slot,event}`. A handler is a write on a binding (`recordSet`, `upsert`, `setMarkdown`, `toggle`, `addOptions`), a wish, open or notify. `{{event.<param>}}` and `{{config.<key>}}` interpolate arguments. One click runs one Commit with that person's rights; reloading never replays. Toolbar `actions` give `id,title,modes,needs,run`. `run` also supports `config` with `{{now}}` and `forward` to `aiws.on("action:<id>",fn)`. `host_api:4` adds `onConfig`, `serverNow`, `userState` and `mediaUrl`. Composite projections lay out labels/shapes/assets derived from records, folder members, counts or viewer. Media slots use `{pos_ms,at_ms,playing}`. `visible.from:"first_open"` is per-viewer presentation, not access control.
- **Binding and initialization.** Version 5 composite `on_bind` runs parent to child and can pick one/many/all, inherit `from_parent`, or create a standard/custom table or directory. `on_init_control` runs child to parent and can require children or return a declared action. HTML `host_api:5` can use `on_bind:"script"` / `on_init_control:"script"`; handle `aiws.context.role === "bind"` in `behavior.js` before touching the DOM. Handle `bind`/`init_control`, use `ctx.given`, `parent`, `decls`, `defaults`, `find`, `read`, `canCreate` or `dialogs`. Return bindings/creates, `null` to cancel. `buckyos aiws place <def or ident> --workspace <ws> --surface <canvas> [--bind …]` runs without dialogs or HTML script: supply required bindings (`NEEDS_UI`) and run the returned `then` actions.
- **Concurrent edits.** Change one `config`, `bindings` or `on` property with `entity.set_props {entity_id,key,props:{<name>:value|null},expect:{<name>:rev}}`. `key_revs["p:<key>.<name>"]` gives the revision (0 if absent). Add a definition `rev` precondition against stale builds.

#### Migrations, packages and templates
Changing a required property, binding type, slot, event or parameter can affect existing instances/dependents. Migrate with `--migration m.json` (`{"<ident>":{config|bindings|events:{rename,drop,fill},slots:{rename,move}}}`); the build updates instances atomically. `ext-upgrade` includes dependencies, may report `unchanged`, and refuses local modifications (`EXT_LOCAL_CHANGES`). `ext-upgrade --force` replaces those local edits with the library version. `EXT_IDENT_EXISTS` on import means upgrade the existing identity. Without CLI, `ext.pack {workspace_id,entity_ids,package:{name,version,title?}}` produces an unsigned PIKG in exports; `ext.install {pikg:{bucket_id,key}}` installs it.

`template export` captures selected surfaces, shown data/controls and `--data` entities, excluding presentation paths, sessions and personal content. `{{params.<k>}}` in titles, configs, records and table text is replaced at creation. `--reset` clears selected tables. Installation adds it to the library. Each `template create` (`ext.create_from_template`) creates an independent workspace owned by the caller. Check `template list`, then create and inspect a workspace from the result.
