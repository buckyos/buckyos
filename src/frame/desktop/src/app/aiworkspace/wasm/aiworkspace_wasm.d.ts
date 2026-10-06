/* tslint:disable */
/* eslint-disable */

/**
 * The offline replica: confirmed layer + pending submissions (design §6.3).
 */
export class Replica {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Apply change-stream events to the confirmed layer and rebase.
     */
    apply_remote(events_json: string): string;
    /**
     * Full-history snapshot of a rich text in the *confirmed* layer (the editor's confirmed document).
     */
    collab_snapshot(entity_id: string): Uint8Array;
    /**
     * Rows of the confirmed layer, to persist after it advanced.
     */
    confirmed_rows(): string;
    discard(key: string): boolean;
    lineage_id(entity_id: string): string | undefined;
    /**
     * `doc.list_annotations` on the working view: `{ target_ids?, parent_id? }`.
     */
    list_annotations(params_json: string): string;
    mark(key: string, state: string, result_json?: string | null): void;
    /**
     * `tables_json`: `{ entities, tree_edges, table_fields, table_records, richtext_states, refs, assets }`
     * as rows of the replica database (BLOBs as base64).
     */
    constructor(principal: string, workspace_id: string, epoch: string, tables_json: string, confirmed_seq: number, peer: number);
    /**
     * The next queued request ready to send, or `null`.
     */
    next_to_send(): string;
    outline(): string;
    pending(): string;
    /**
     * Rich text updates of still-undecided pending submissions for one entity, in order
     * (base64): confirmed snapshot + these = the working document.
     */
    pending_richtext_updates(entity_id: string): string;
    /**
     * `doc.query` on the working view (embedded tables only; URL tables need the network).
     */
    query(params_json: string): string;
    read(entity_id: string, selector_json?: string | null): string;
    /**
     * Restore persisted pending submissions: `[{ idempotency_key, request, state, result? }]`.
     */
    restore_pending(rows_json: string): void;
    /**
     * Plan a local Commit on the working view. Returns `{ status: "saved_locally", ... }`
     * or the same conflict/rejected result the backend would give.
     */
    submit_local(request_json: string): string;
    /**
     * Rows of the confirmed layer changed by `apply_remote` since the last call (upserts by
     * primary key, plus `refs_deleted`): what the replica database writes in the same
     * transaction as the new `confirmed_seq` and the pending rows.
     */
    take_confirmed_delta(): string;
    readonly confirmed_seq: number;
}

/**
 * RFC 8785 text of a JSON value, after the strict pre-checks (V01).
 */
export function canonical_json(json_text: string): string;

export function chunk_id(data: Uint8Array): string;

export function core_version(): string;

/**
 * FileObject id of a single-chunk file with these bytes.
 */
export function file_object_id(data: Uint8Array): string;

/**
 * Normalize a value for a field definition (same rules as a write).
 */
export function normalize_value(field_def_json: string, value_json: string): string;

/**
 * `type:hex` ObjectId of a JSON NamedObject.
 */
export function object_id(obj_type: string, json_text: string): string;

export function order_key_between(a?: string | null, b?: string | null): string;

/**
 * Block index (`{ block_id: { parent, node_type, hash, struct_rev } }`) of a canonical AST.
 */
export function richtext_block_index(ast_json: string): string;

/**
 * Build a new-lineage document from an AST (offline creation); returns the snapshot bytes.
 */
export function richtext_build(ast_json: string, peer: number): Uint8Array;

export function richtext_canonicalize(ast_json: string): string;

/**
 * Decode a Loro snapshot (optionally followed by updates) into the canonical AST.
 */
export function richtext_decode(snapshot: Uint8Array): string;

/**
 * Block-level operations turning `base` into `target` (explicit draft commit).
 * `block_index_json` is the base's `{ block_id: { hash, struct_rev } }` as read from the backend.
 */
export function richtext_diff(entity_id: string, base_ast_json: string, target_ast_json: string, block_index_json: string): string;

/**
 * The rich text schema definition both sides are generated from.
 */
export function richtext_schema(): string;

export function verify_object(id: string, canonical_text: string): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_replica_free: (a: number, b: number) => void;
    readonly canonical_json: (a: number, b: number, c: number) => void;
    readonly chunk_id: (a: number, b: number, c: number) => void;
    readonly core_version: (a: number) => void;
    readonly file_object_id: (a: number, b: number, c: number) => void;
    readonly normalize_value: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly object_id: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly order_key_between: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly replica_apply_remote: (a: number, b: number, c: number, d: number) => void;
    readonly replica_collab_snapshot: (a: number, b: number, c: number, d: number) => void;
    readonly replica_confirmed_rows: (a: number, b: number) => void;
    readonly replica_confirmed_seq: (a: number) => number;
    readonly replica_discard: (a: number, b: number, c: number) => number;
    readonly replica_lineage_id: (a: number, b: number, c: number, d: number) => void;
    readonly replica_list_annotations: (a: number, b: number, c: number, d: number) => void;
    readonly replica_mark: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
    readonly replica_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
    readonly replica_next_to_send: (a: number, b: number) => void;
    readonly replica_outline: (a: number, b: number) => void;
    readonly replica_pending: (a: number, b: number) => void;
    readonly replica_pending_richtext_updates: (a: number, b: number, c: number, d: number) => void;
    readonly replica_query: (a: number, b: number, c: number, d: number) => void;
    readonly replica_read: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly replica_restore_pending: (a: number, b: number, c: number, d: number) => void;
    readonly replica_submit_local: (a: number, b: number, c: number, d: number) => void;
    readonly replica_take_confirmed_delta: (a: number, b: number) => void;
    readonly richtext_block_index: (a: number, b: number, c: number) => void;
    readonly richtext_build: (a: number, b: number, c: number, d: number) => void;
    readonly richtext_canonicalize: (a: number, b: number, c: number) => void;
    readonly richtext_decode: (a: number, b: number, c: number) => void;
    readonly richtext_diff: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly richtext_schema: (a: number) => void;
    readonly verify_object: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly __wbindgen_export: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export2: (a: number, b: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
