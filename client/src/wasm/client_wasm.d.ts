/* tslint:disable */
/* eslint-disable */

export class CrdtClient {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Apply a batch of remote ops (as a JSON array), e.g. everything received
     * since the last call - never invoked per-op from JS.
     */
    applyRemoteOps(ops_json: string): void;
    localDelete(pos: number): string | undefined;
    /**
     * Insert `value` (a single character) at visible-text position `pos`.
     * Returns the produced op, JSON-serialized, for the caller to broadcast.
     */
    localInsert(pos: number, value: string): string;
    constructor(site_id: bigint);
    redo(): string | undefined;
    render_text(): string;
    stateVector(): string;
    undo(): string | undefined;
}

/**
 * Route Rust panics to the browser console with real stack info instead of an
 * opaque "unreachable executed" trap - call this once from the client on load.
 */
export function initPanicHook(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_crdtclient_free: (a: number, b: number) => void;
    readonly crdtclient_applyRemoteOps: (a: number, b: number, c: number) => [number, number];
    readonly crdtclient_localDelete: (a: number, b: number) => [number, number, number, number];
    readonly crdtclient_localInsert: (a: number, b: number, c: number) => [number, number, number, number];
    readonly crdtclient_new: (a: bigint) => number;
    readonly crdtclient_redo: (a: number) => [number, number, number, number];
    readonly crdtclient_render_text: (a: number) => [number, number];
    readonly crdtclient_stateVector: (a: number) => [number, number, number, number];
    readonly crdtclient_undo: (a: number) => [number, number, number, number];
    readonly initPanicHook: () => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
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
