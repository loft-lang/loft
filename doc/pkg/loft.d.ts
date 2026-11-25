/* tslint:disable */
/* eslint-disable */

/**
 * `@PLN226` — a page's debugger: the debug session over the program it started.  The page
 * holds the handle and passes it to every call (`@FR-C-Boundary`): two debuggers on one page
 * never meet, and nothing is kept per thread.
 *
 * A doc page (@PLN149 step 8) holds the source the reader is editing and drives it through
 * [`Self::start`] and [`Self::command`] — set a breakpoint, pause, read the frame, evaluate
 * an expression against it, resume; the writer route's relay feeds the same grammar through
 * [`Self::pump`], so the two drive one implementation.
 *
 * ```js
 * const dbg = new Debugger();
 * JSON.parse(dbg.start(source));            // {"ok":true} | {"ok":false,"error":"…"}
 * JSON.parse(dbg.command('bp 4'));          // {"replies":[…],"output":"…"}
 * ```
 */
export class Debugger {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Apply one debug command to the live session.
     *
     * Returns JSON `{"replies":[…],"output":"…"}` — the `D:` replies the command produced
     * and whatever the program printed while it ran.  The two travel together because a
     * `run` or a `resume` produces both, and a page fetching them separately could paint a
     * pause before the output that led to it.
     *
     * The grammar is [`Self::replies`]': `bp <fn>` / `bp <line>`, `run`, `resume`, `step`,
     * `eval <expr>`, `fns`.
     */
    command(cmd: string): string;
    /**
     * A debugger with no program yet.
     */
    constructor();
    /**
     * Start a debug session over `source`, replacing this debugger's previous one.
     *
     * Returns JSON `{"ok":true}`, or `{"ok":false,"error":"…"}` carrying the diagnostics — a
     * page that offers to run the code in front of the reader has to say why it will not.
     *
     * A bare script (top-level statements, no `fn main`) is desugared exactly as
     * `compile_and_run` desugars it, so the two entries accept the same inputs.
     */
    start(source: string): string;
}

/**
 * `@PLN226` — a page's game: the program it started and runs frame by frame, and the world
 * snapshot staged for the next program it starts (@PLN18 08-S6).  The page holds the handle and
 * passes it to every call (`@FR-C-Boundary`): two games on one page never meet, and nothing is
 * kept per thread.
 *
 * ```js
 * const game = new Game();
 * let r = JSON.parse(game.start(JSON.stringify([{ name: 'main.loft', content: src }])));
 * while (r.running) { await nextFrame(); r = JSON.parse(game.resume_frame()); }
 * ```
 */
export class Game {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * A game with nothing started.
     */
    constructor();
    /**
     * Run the next frame.  JSON `{"running":true}` (with `"output"` when the frame printed),
     * `{"running":false,"output":"…"}` when the program finished, or
     * `{"running":false,"error":"…"}` when it crashed.
     */
    resume_frame(): string;
    /**
     * Stage `snapshot` (`swap_export` of another game) for the next program this game starts:
     * its `swap_world` restores it — the browser's form of the native `LOFT_RESUME`.
     */
    stage_swap(snapshot: string): void;
    /**
     * Start the program in `files_json` (`[{name, content}]`), ending this game's previous
     * one: parse, compile, execute until the first frame yield.  JSON
     * `{"ok":true,"running":true}`, `{"ok":true,"running":false,"output":"…"}` when it ran to
     * its end, or `{"ok":false,"error":"…"}`.
     */
    start(files_json: string): string;
    /**
     * End the running program and close its window.
     */
    stop(): void;
    /**
     * May the program in `files_json` take over this game's world?
     */
    swap_check(files_json: string): string;
    /**
     * The running program's world, for another game to take over (`stage_swap`).
     */
    swap_export(): string;
}

/**
 * Run a loft program supplied as a JSON array of `{name, content}` file objects.
 *
 * Returns a JSON string: `{"output": "...", "diagnostics": [...], "success": true|false}`.
 *
 * The default standard library files are embedded in the binary; user files
 * are taken from `files_json`.  Any `use <id>;` statement is resolved against
 * files whose name matches `<id>.loft` in the supplied file list.
 *
 * # Errors
 * Returns a JSON error object if `files_json` cannot be parsed.
 *
 * When compiled with `--features wasm` and exported via `wasm-bindgen`, this
 * function is callable from JavaScript as:
 * ```js
 * const result = JSON.parse(loft.compile_and_run(JSON.stringify([
 *   {name: 'main.loft', content: 'fn main() { println("hi") }'}
 * ])));
 * ```
 */
export function compile_and_run(files_json: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_debugger_free: (a: number, b: number) => void;
    readonly __wbg_game_free: (a: number, b: number) => void;
    readonly compile_and_run: (a: number, b: number) => [number, number];
    readonly debugger_command: (a: number, b: number, c: number) => [number, number];
    readonly debugger_new: () => number;
    readonly debugger_start: (a: number, b: number, c: number) => [number, number];
    readonly game_new: () => number;
    readonly game_resume_frame: (a: number) => [number, number];
    readonly game_stage_swap: (a: number, b: number, c: number) => void;
    readonly game_start: (a: number, b: number, c: number) => [number, number];
    readonly game_stop: (a: number) => void;
    readonly game_swap_check: (a: number, b: number, c: number) => [number, number];
    readonly game_swap_export: (a: number) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
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
