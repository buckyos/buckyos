#### Asked from the AI Workspace panel
The `[aiworkspace context]` block supplies workspace, page, canvas, guide step, view, open entity and selected Block when applicable. Subsequent messages may inherit it. On a selected wish use the entity ID after `shows … (buckyos.wish)`, which differs from the Block ID. Read with `buckyos obj read <url>` or `buckyos aiws read <id or path> --workspace <ws>`. When asked for analysis, work out the task, start and submit the analysis, and tell the user it is in the wish. A request for a data source from a collaborator goes to the workspace owner; build it when the owner hands it to you.

#### Actions on workspace objects
Describe `obj://aiworkspace/<ws>/entities/<id>` with `buckyos obj read <url>` (or `agent_tool read <url>`). Inspect `declared.actions`, risk, input schema, and `meta.capabilities`. Read properties or fixed references by selector/version with the caller's access. Read `obj://aiworkspace` property `controls` for the catalog, or `buckyos aiws controls describe <kind>`.
Wish actions (`analyze`, `code`, `run`) write a contract, a macro or results. A sourced entity (`meta.data.write: refresh`) declares an `idempotent_write` refresh. Explain what it will write, including overwritten edits, and ask the user before any action whose risk differs from `read_only`. Other entities declare no actions.

```sh
agent_tool xcall "obj://aiworkspace/$WS/entities/$WISH" analyze --params '{"execution":{"mode":"local"},"idempotency_key":"<key>"}'
agent_tool xcall "obj://aiworkspace/$WS/entities/$SOURCE" refresh --params '{"idempotency_key":"<key>"}'
```

Each invokes `proc.start` with the caller's rights. The stage may also take `request` (`today`, `timezone`) and `location`. Put the key inside `--params` and reuse the same key/input for retry. `local` returns `run_id` and `claim_token` for you to execute. `hosted` is for an account dispatching to its own Agent (`EXECUTION_CONTEXT_MISSING` otherwise). Poll `buckyos aiws run-get --workspace "$WS" --run "$RUN"` for either action.

#### Taking a stage back
Only on request, use `buckyos aiws run-rollback --workspace "$WS" --run "$RUN"` with update on the wish. Only the current record for that stage can be undone (`NOT_CURRENT_RECORD`). This restores overwritten edits and the previous macro or analysis, removes results and Blocks created by the run, and makes the previous record current. On `needs_confirm` tell the user what `lost` and `hidden_lost` report. Repeat with `--confirm` only after agreement. Verify the accepted commit and rollback record; the former record is retained and marked.
