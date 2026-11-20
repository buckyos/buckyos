# SDK identity

- Build against npm `buckyos`; installed types: `/opt/buckyos/libexec/buckyos-tool/dist/{sdk_core,runtime,node}.d.ts`. Never package keys, tokens or host tools.
- Script/Docker: environment supplies BUCKYOS_APP_ID, BUCKYOS_APP_DID, BUCKYOS_APP_INSTANCE_ID (`<app_id>@<owner>`), BUCKYOS_OWNER_USER_ID, BUCKYOS_DATA_DIR, BUCKYOS_APP_TOKEN (device-signed assertion exchanged for session), BUCKYOS_HOST_GATEWAY and BUCKYOS_ZONE_CONFIG. TypeScript service pattern from websdk `examples/app_service_local_debug.ts`:
```ts
await buckyos.initBuckyOS('', { appId: '', runtimeType: RuntimeType.AppService, zoneHost, defaultProtocol: 'https://' })
await buckyos.login()
```
  Then use `getAppDataDir()`, `getAppSetting()/setAppSetting()`, `getZoneServiceURL('<svc>')`. The original check did not exercise the pattern in an installed App.
- Browser: `await buckyos.initBuckyOS('<app_id>')` then `await buckyos.getAccountInfo()`; use `buckyos.login()` when user_id is absent. `getCurrentIdentity()` distinguishes account (user_type/restricted), External DID and anonymous visitor. Limited sessions can reach their App yet get kRPC `access_denied` on restricted methods. Inspect `buckyos.accessDenialOf(err)?.reason`, show account limitation. Keep the user's identity for all user actions. See `doc/sdk/SSO.md`.

## Desktop integration

Use `buckyos/app-frame`. Derive shell origin via `embedderOrigin(o => originInZone(o, buckyos.getZoneHostName()))`; system origin is `${location.protocol}//${buckyos.getZoneHostName()}`. Hostname splitting fails on custom domains.

- Content handlers (`content_handlers.open`): `frame.openContent({source}, 'chooser', {hints:{name,mime,size}})` opens system “Open with” (embedded chooser excludes current App; standalone opens system in new tab). For inline preview of unsupported content, `PreviewEmbed` uses `/embed/preview` and its own session.
- Every open/folder/save entry uses the system picker, `await frame.selectContent({mode, multiple, accept, start, suggestedName, allowLocal})`, with `mode` = open/folder/save. Include device files by setting `allowLocal:true` in the same picker. Embedded shell draws the dialog; standalone frames `/embed/sysdlg/select`. Handle `{action:'selected',items}` (Zone `source` cyfs:/// reference for caller session, device item File), `{action:'save',parent,name,overwrite}`, `{action:'cancelled'}` and SystemDialogError (unsupported/invalid_params/busy/closed/load_failed/protocol). `ensureObjId(item)` gets immutable ObjId; `ndm.pickupAndImport` is for explicit device upload. Reference `doc/desktop/BuckyOS 系统对话框实现方案.md` §4, `src/apps/text_editor/readme.md` §7, `test/app_installer_test/pikg_samples/content-open-handler/web/main.js`, websdk `src/app-frame.ts`.
- Chat with user's Agent via `new ConversationEmbed({systemOrigin, frame: () => iframe, init: {target:{kind:'agent'}, theme,locale,placeholder}})`. Set iframe to embed's url (`/embed/conversation` uses its Zone-root session), `embed.send({text,attachments:[contentRef]})` sends as user. onState/onTurn report ready/sessionId/turn state, not message bodies. Host-derived `ui.source` identifies App; `{kind:'session',sessionId}` opens only its own conversations. See `doc/sdk/conversation-embed.md` and Text Editor's `AgentPanel` in `src/apps/text_editor/src/ui/App.tsx`.
