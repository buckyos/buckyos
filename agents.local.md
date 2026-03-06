# Local Agent Notes

Machine-local constraints for this workspace. Keep this file short and current.

## Environment

- Python: `source /home/aa/app/venv/bin/activate`
- `buckyos-devkit` is already installed in that venv.
- Node path: `/home/aa/.nvm/versions/node/v24.11.0/bin/node`
- Before web build, prefer:
  - `export PATH="/home/aa/.nvm/versions/node/v24.11.0/bin:$PATH"`

## Build/Deploy (local -> mylinux)

- Runtime host: `ssh mylinux`
- In this workspace, do build only.
- Frontend only:
  - `cd src/frame/control_panel/web && pnpm build`
- Frontend + backend:
  - `cd src/frame/control_panel/web && pnpm build`
  - `cd src && cargo build -p control_panel --release`
- Do not run local install/deploy here:
  - `buckyos-install`
- Avoid `buckyos-build` for current control_panel deployment (musl artifact risk on target).
- After build, copy artifacts/binaries to `mylinux`.

## Restart Strategy

- Default: fine-grained restart.
  - Frontend-only (`control_panel_web`): copy web output to `/opt/buckyos/bin/control-panel/web/`, no restart.
  - Backend (`control_panel`): copy binary to `/opt/buckyos/bin/control-panel/control_panel`, then kill only that process and let node-daemon restart it.
  - Health checks:
    - `ssh mylinux "pkill -f /opt/buckyos/bin/control-panel/control_panel; sleep 2; pgrep -af /opt/buckyos/bin/control-panel/control_panel"`
    - `ssh mylinux "pgrep -af /opt/buckyos/bin/node-daemon/node_daemon"`
- Full restart only when necessary:
  - `ssh mylinux "systemctl restart buckyos && systemctl is-active buckyos"`

## Files Module Status (control panel -> desktop -> files)

- Already available:
  - recycle bin + restore/permanent delete
  - folder zip download
  - list scopes: `Recent`, `Starred`, `Trash`
  - list sort/filter controls
- Current P0 focus:
  - replace browser `prompt/confirm` with unified in-app dialogs
  - sharing ACL model (`viewer`/`editor`/`commenter`) + `Shared with me`
- P1:
  - version history/rollback
  - conflict UX
  - per-file/folder activity timeline
  - keyboard + shift-range batch polish
- P2:
  - richer grid view + details panel
  - pin/shortcut capabilities
  - storage quota + sync/offline status
- Priority note:
  - editor-related improvements are currently low priority.

## Thumbnail Note

- Thumbnail generation scope: `jpg/jpeg/png/webp`.
- GIF thumbnails are intentionally not generated yet (fallback to file icon).
