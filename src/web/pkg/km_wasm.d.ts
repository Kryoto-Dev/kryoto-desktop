/* tslint:disable */
/* eslint-disable */

/**
 * This browser as a chat device: its keys, sessions and trust pins.
 */
export class Kryo {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    acknowledgeChange(peer: string): void;
    /**
     * Keep the prekeys from a `keysBundle` frame for the next encryption.
     */
    addClaims(bundle_frame: string): void;
    /**
     * A new recovery code and the backup sealed with it: `{"code","blob"}`.
     */
    backupCreate(): string;
    /**
     * Re-seal the backup with a kept code (trust pins changed).
     */
    backupReseal(code: string): string;
    /**
     * Check a user's devices from a `devices` frame (JSON from `decode`).
     * Returns `{"status": "ok"|"identityChanged"|"absent", "count": n}`.
     */
    checkDevices(devices_frame: string, user_id: string): string;
    /**
     * A brand-new web device for an account, with a fresh identity (used
     * unless an existing identity is restored from a backup).
     */
    static create(user_id: string): Kryo;
    /**
     * A server frame as JSON (`type`, `requestId`, and its fields).
     */
    static decode(bytes: Uint8Array): string;
    /**
     * Decrypt a delivered envelope. Returns `{"ok": {sender, content}}` or
     * `{"error": "unknownSender", "userId"}` (look their devices up, try
     * again) / `"duplicate"` / `"undecryptable"`.
     */
    decrypt(envelope: Uint8Array): string;
    deviceId(): string | undefined;
    forgetDevices(user_id: string): void;
    hello(token: string): Uint8Array;
    knownDevices(user_id: string): number;
    markVerified(peer: string, verified: boolean): void;
    masterPublic(): string;
    /**
     * Does any device of this user still need a session (a prekey claim)?
     */
    needsClaim(user_id: string): boolean;
    static newMessageId(): string;
    static openFile(key_hex: string, sha256_hex: string, ciphertext: Uint8Array): Uint8Array;
    proof(nonce: Uint8Array): Uint8Array;
    /**
     * What a receiver shows: `{"shown": "ok"}`, `{"shown":"truncated","text"}`
     * or `{"shown":"gifAsText","text"}`.
     */
    static receiveRules(content: string, sender_supporter: boolean): string;
    register(nonce: Uint8Array, name: string): Uint8Array;
    /**
     * A client request. `kind` names it; `args` (JSON) carries its fields.
     * Requests that need keys (publishing the master key, certifying this
     * device, uploading prekeys) are filled in here.
     */
    request(request_id: number, kind: string, args: string): Uint8Array;
    /**
     * A new identity for the whole account (contacts are warned).
     */
    resetIdentity(): void;
    /**
     * Take over the identity in a backup (base64 blob) with its code.
     */
    restoreBackup(code: string, blob: string): void;
    static restore(state: string): Kryo;
    /**
     * Seal a file: returns key (32) ‖ sha256 (32) ‖ ciphertext.
     */
    static sealFile(bytes: Uint8Array): Uint8Array;
    /**
     * Encrypt `content` (JSON) for every known device of `users` (JSON array
     * of ids) and return the Send frame. Throws "nothing to send" if no
     * device could be reached.
     */
    sendFrame(request_id: number, content: string, users: string, ephemeral: boolean, group_id: string, quiet: boolean): Uint8Array;
    setDeviceId(id: string): void;
    /**
     * Everything needed to restore this device. Secret: encrypt before storing.
     */
    state(): string;
    userId(): string;
    /**
     * Throws when the content may not be sent (length, supporter perks...).
     */
    static validate(content: string, supporter: boolean): void;
    verifyInfo(peer: string): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_kryo_free: (a: number, b: number) => void;
    readonly kryo_acknowledgeChange: (a: number, b: number, c: number) => [number, number];
    readonly kryo_addClaims: (a: number, b: number, c: number) => [number, number];
    readonly kryo_backupCreate: (a: number) => [number, number, number, number];
    readonly kryo_backupReseal: (a: number, b: number, c: number) => [number, number, number, number];
    readonly kryo_checkDevices: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly kryo_create: (a: number, b: number) => [number, number, number];
    readonly kryo_decode: (a: number, b: number) => [number, number, number, number];
    readonly kryo_decrypt: (a: number, b: number, c: number) => [number, number];
    readonly kryo_deviceId: (a: number) => [number, number];
    readonly kryo_forgetDevices: (a: number, b: number, c: number) => void;
    readonly kryo_hello: (a: number, b: number, c: number) => [number, number];
    readonly kryo_knownDevices: (a: number, b: number, c: number) => number;
    readonly kryo_markVerified: (a: number, b: number, c: number, d: number) => [number, number];
    readonly kryo_masterPublic: (a: number) => [number, number];
    readonly kryo_needsClaim: (a: number, b: number, c: number) => number;
    readonly kryo_newMessageId: () => [number, number];
    readonly kryo_openFile: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly kryo_proof: (a: number, b: number, c: number) => [number, number, number, number];
    readonly kryo_receiveRules: (a: number, b: number, c: number) => [number, number, number, number];
    readonly kryo_register: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly kryo_request: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly kryo_resetIdentity: (a: number) => void;
    readonly kryo_restore: (a: number, b: number) => [number, number, number];
    readonly kryo_restoreBackup: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly kryo_sealFile: (a: number, b: number) => [number, number, number, number];
    readonly kryo_sendFrame: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly kryo_setDeviceId: (a: number, b: number, c: number) => [number, number];
    readonly kryo_state: (a: number) => [number, number, number, number];
    readonly kryo_userId: (a: number) => [number, number];
    readonly kryo_validate: (a: number, b: number, c: number) => [number, number];
    readonly kryo_verifyInfo: (a: number, b: number, c: number) => [number, number, number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
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
