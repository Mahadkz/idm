# XDM engineering rules

Binding conventions for every contributor and coding agent. When a review finds the same defect twice, add or sharpen a rule here instead of patching files one by one.

## Correctness and safety

1. **Never overwrite user files.** Write to a temporary sibling, then publish with hard link + remove (`xdm_http::publish` pattern); fall back to a checked rename only where hard links are unsupported. An existing destination is an error that keeps the completed data.
2. **Never run a shell.** Spawn programs with explicit argument vectors. No string interpolation into commands, including sound players, completion programs, clipboard tools and ffmpeg.
3. **Destructive OS actions** (sleep, hibernate, shutdown, restart) require explicit per-queue opt-in and a cancelable countdown. Tests and development use `XDM_FAKE_OS_ACTIONS=1`; never execute a real power action.
4. **Secrets never leave their store.** Passwords, cookies, Authorization and request bodies must not appear in task lists, exports, logs, CLI arguments or error messages. Passwords are read from files or stdin, not argv.
5. **Never silently change request semantics.** No POST→GET conversion, no splicing bytes from a changed remote object, no resuming without validators.
6. **Do not block the daemon lock** on network, disk-heavy or child-process work. Spawn and reap in a thread; the lock protects state, not I/O.
7. **Exports must round-trip.** An exported entry must re-import as the same kind of job; omit what cannot be represented rather than degrading it to a different job.

## Portability

8. **Every change must build for `x86_64-pc-windows-gnu`.** Unix-only APIs (`localtime_r`, `tm_gmtoff`, permissions bits, sockets) need `#[cfg(unix)]` with a Windows implementation or a documented, compiled fallback.
9. Paths are `Path`/`PathBuf`, never string concatenation with `/`.

## Compatibility

10. **IPC is append-only.** Add new `Command` variants and new types; never add fields to existing variants (GUI code builds them with struct literals). New reply/summary fields carry `#[serde(default)]`.
11. **SQLite schema changes are additive migrations** that are idempotent on reopen and preserve existing rows.
12. Existing checkpoint formats stay readable; new runtime-only fields are `#[serde(skip)]`.

## Tests and evidence

13. Every behavior change ships with a test that **fails without the change**. End-to-end tests use disposable local servers on 127.0.0.1 and temporary data directories.
14. GUI tests run only under their own Xvfb display (`SLINT_BACKEND=winit-software`); nothing is launched on the user's desktop.
15. Reports state exactly what was run and what was not. "Untested on Windows/real browser" is stated, not omitted. Docs rows describe tested behavior only; a status cell may not claim more than its cited test exercises.
16. Expected-failure (XFAIL) runners count a gap only when the test fails on an assertion; any other exception is a harness error that fails the run. Crashing tests must never be reported as known gaps, and a gap test must exercise the real behavior: asserting on invented fields or outputs is forbidden. Behavior that cannot be exercised headlessly is reported as UNTESTED, never as a pass or a known gap.
17. Builds and tests must be warning-free: `cargo build --workspace --offline` and `cargo test --workspace --offline`.

## Style

18. Match the surrounding code: `Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>` in library crates, sparse comments that explain why, bounded buffers and limits on every untrusted input.
19. Do not copy proprietary IDM code, text or assets; implement independently from the behavior contracts.
