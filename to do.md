# WhimprFlow Sidecar Architecture Handoff

## Purpose

Move the global keyboard hook and native text-insertion responsibilities out of the Tauri application and into a small, supervised Rust sidecar process.

This is primarily a reliability and isolation change. WhimprFlow currently transcribes correctly, but its keyboard hook, ASR inference, cleanup model, webview, and text insertion all share one process. Heavy ONNX or llama.cpp work can starve the hook thread, and a crash in any major subsystem takes the hook down with the entire application.

This work is recommended before Windows support, beta distribution, or adding more inference-heavy functionality. It is not required to continue testing the current macOS prototype.

## Why it matters

### macOS

The in-process `CGEventTap` is recoverable when macOS disables it, but the triggering Fn press can still be lost during an inference stall. An inference crash also terminates the hook.

### Windows

`WH_KEYBOARD_LL` callbacks must return within the Windows timeout, capped at approximately 1000 ms on modern Windows. If the callback times out, Windows can silently remove the hook. There is no reliable API for detecting that removal from the process that owned the hook.

A supervised sidecar provides:

- Independent scheduling from ASR and LLM inference.
- Crash isolation from Tauri, WebKit/WebView2, ONNX, and llama.cpp.
- Heartbeat-based failure detection.
- The ability to terminate and respawn a damaged hook process.
- A smaller native surface for Accessibility, secure-input detection, and injection.

## Current repository state

### Existing protocol

`crates/whimpr-ipc` already defines a versioned, length-prefixed JSON protocol.

Important protocol types include:

- `ShellToSidecar::Hello`
- `ShellToSidecar::UpdateShortcuts`
- `ShellToSidecar::SetSuppression`
- `ShellToSidecar::PasteText`
- `ShellToSidecar::CheckStaleKeys`
- `ShellToSidecar::QuerySecureInput`
- `ShellToSidecar::ReadContext`
- `ShellToSidecar::Ping`
- `ShellToSidecar::Shutdown`
- `SidecarToShell::Ready`
- `SidecarToShell::Trigger`
- `SidecarToShell::TapEvent`
- `SidecarToShell::PasteResult`
- `SidecarToShell::ContextResult`
- `SidecarToShell::Pong`
- `SidecarToShell::Heartbeat`

The protocol version is currently `1`.

### Existing sidecar

`crates/whimpr-sidecar/src/main.rs` is only a macOS Fn-key demonstration:

- It uses a listen-only `CGEventTap`.
- It prints Fn press/release messages to stdout.
- It exits after three presses or a 60-second timeout.
- It does not use `whimpr-ipc`.
- It does not suppress Fn.
- It does not perform injection, secure-field detection, or Accessibility context reads.
- It has no persistent daemon lifecycle, heartbeat, or shell supervision.
- The non-macOS implementation is a stub.

### Existing production behavior

The real implementation is currently in-process:

- `src-tauri/src/hotkey.rs` owns the macOS event tap, state-machine input, microphone capture, and transcription pipeline coordination.
- `src-tauri/src/paste.rs` owns the macOS insertion ladder and secure-field handling.
- `src-tauri/src/win.rs` contains the current Windows platform implementation.
- `whimpr-core` owns the platform-independent dictation state machine.
- ASR, VAD, cleanup, statistics, and Flow Bar state should remain in the shell/core process.

### Architecture references

Read these before implementation:

- `docs/ARCHITECTURE-DUAL-PLATFORM.md`
- `docs/research/gap-sidecar-vs-inprocess.md`
- `crates/whimpr-ipc/src/lib.rs`
- `crates/whimpr-ipc/src/codec.rs`
- `crates/whimpr-sidecar/src/main.rs`
- `src-tauri/src/hotkey.rs`
- `src-tauri/src/paste.rs`
- `src-tauri/src/win.rs`

`docs/SPEC.md` contains an older one-process/native-Swift architecture section. The current Tauri dual-platform direction is documented in `docs/ARCHITECTURE-DUAL-PLATFORM.md`.

## Target ownership split

### Tauri shell/core remains responsible for

- Dictation state machine.
- Flow Bar and Hub UI.
- Microphone capture.
- Audio buffering and resampling.
- Silero VAD.
- Parakeet/Whisper inference.
- Cleanup model execution.
- Dictionary, snippets, transforms, history, and statistics.
- Sidecar startup, health supervision, and restart policy.

### Sidecar becomes responsible for

- Global keyboard hook installation.
- Fn and configured shortcut detection.
- Shortcut suppression.
- Esc cancellation detection while recording.
- Hook/tap health monitoring.
- Stale-held-key detection and clearing.
- Secure-input or elevated-target detection.
- Accessibility/UI Automation context reads.
- Text insertion and its fallback ladder.
- Clipboard save, concealed write, paste, and restoration.
- Structured native logs and recoverable error reports.

The sidecar should not run ASR or cleanup inference.

## Proposed implementation phases

### Phase 0 — Baseline and migration safety

Before moving behavior:

1. Confirm the current macOS dictation flow still works.
2. Add or preserve tests for `whimpr-ipc` frame encoding and decoding.
3. Record current shortcut behavior:
   - Hold-to-talk.
   - Release-to-finalize.
   - Double-tap hands-free mode.
   - Esc cancellation.
   - Bare-Fn suppression.
4. Keep the in-process hook and insertion implementation available behind a temporary fallback flag.
5. Do not delete proven behavior until its sidecar replacement passes end-to-end tests.

Exit criteria:

- Existing dictation remains functional.
- IPC codec tests cover malformed frames, oversized frames, and EOF.
- There is a documented way to force the in-process fallback.

### Phase 1 — Persistent IPC sidecar

Convert `whimpr-sidecar` from a demo into a persistent daemon:

1. Read framed `ShellToSidecar` messages from stdin.
2. Write framed `SidecarToShell` messages to stdout.
3. Reserve stderr for human-readable diagnostics so it cannot corrupt framed stdout.
4. Require `Hello` before accepting operational commands.
5. Validate `PROTOCOL_VERSION`.
6. Return `Ready` with the current OS and capability flags.
7. Implement `Ping`/`Pong`.
8. Implement clean `Shutdown`.
9. Emit periodic `Heartbeat` messages.
10. Treat malformed or oversized frames as protocol errors and exit safely.

Exit criteria:

- A standalone integration test can spawn the sidecar, complete the handshake, ping it, and shut it down.
- Protocol mismatches fail clearly.
- No non-framed data is written to stdout.

### Phase 2 — Shell-side supervisor

Add a Tauri-side sidecar manager:

1. Spawn the bundled `whimpr-sidecar` executable.
2. Connect its stdin, stdout, and stderr.
3. Perform the version handshake.
4. Continuously decode sidecar events on a dedicated reader thread/task.
5. Route `Trigger` messages into the existing `whimpr-core` state machine.
6. Send shortcut configuration after every successful handshake.
7. Track heartbeat and ping deadlines.
8. Restart the sidecar after crashes, broken pipes, failed heartbeats, or an unhealthy hook.
9. Use bounded restart backoff to prevent a crash loop.
10. Surface a Flow Bar or Hub error if recovery repeatedly fails.
11. Shut the child down cleanly when WhimprFlow exits.

Suggested location:

- New module such as `src-tauri/src/sidecar.rs`.

Exit criteria:

- Killing the sidecar manually causes an automatic restart.
- A restarted sidecar receives the latest shortcuts and suppression settings.
- Tauri shutdown does not leave an orphan helper process.

### Phase 3 — Move the macOS keyboard hook

Replace the sidecar demo with the real consuming macOS tap:

1. Use a session-level, head-insert, default/consuming `CGEventTap`.
2. Detect Fn using keycode `63` and the secondary-Fn flag.
3. Handle key-down, key-up, and flags-changed events required by configured shortcuts.
4. Suppress only registered shortcuts and required bare-Fn system behavior.
5. Keep the callback minimal: update atomic state or send to an internal channel and return.
6. Run the tap on its own CFRunLoop thread.
7. Re-enable on `TapDisabledByTimeout` and `TapDisabledByUserInput`.
8. Add a periodic health check.
9. Fully recreate the tap if re-enabling fails.
10. Implement stale-held-key reconciliation.
11. Emit `TapEvent` and `Heartbeat { hook_alive }` updates.

Migration sequence:

1. Start the sidecar hook in observation mode while the in-process hook remains authoritative.
2. Compare trigger sequences in logs.
3. Make the sidecar authoritative behind a feature flag.
4. Disable the in-process tap when the sidecar is healthy.
5. Fall back only when explicitly enabled during development.

Exit criteria:

- Hold, release, double-tap lock, locked stop, and Esc cancellation match existing behavior.
- Fn does not trigger the macOS Globe action.
- Artificial tap disablement recovers.
- Sidecar death and restart do not leave Fn stuck or permanently suppressed.

### Phase 4 — Move text insertion and context APIs

Move the responsibilities currently in `paste.rs` behind IPC:

1. Implement `PasteText`.
2. Preserve the current insertion ladder:
   - Accessibility selected-text insertion.
   - Clipboard paste.
   - Chunked paste.
   - Unicode typing fallback.
   - Per-application overrides.
   - Failed-paste detection.
3. Preserve password/secure-field refusal.
4. Preserve clipboard snapshot and restoration behavior.
5. Preserve concealed clipboard metadata.
6. Implement `QuerySecureInput`.
7. Implement `ReadContext`.
8. Return accurate `PasteResult` and `ContextResult` messages.
9. Add request IDs to the IPC protocol if more than one request can be outstanding.
10. Add explicit timeouts in the shell so an Accessibility call cannot stall dictation forever.

Important:

- Do not delete the current insertion behavior before parity tests pass.
- UI Automation/Accessibility calls should run away from the hook callback.
- The hook must remain responsive while paste or context operations are blocked.

Exit criteria:

- Dictation still inserts correctly into Cursor, browsers, native text fields, and terminals.
- Secure fields are declined.
- Clipboard contents are restored.
- Injection failure returns an actionable Flow Bar error.

### Phase 5 — Windows hook and injection

Implement the Windows side of the same Rust sidecar:

1. Install `WH_KEYBOARD_LL` using `SetWindowsHookExW`.
2. Run the installing thread with a proper Windows message loop.
3. Keep the hook callback to minimal state work/channel send.
4. Implement configured modifier-only shortcuts.
5. Implement correct Win-key suppression and release behavior.
6. Implement `SendInput`, clipboard, Unicode, chunked, and Shift+Insert paths.
7. Implement UI Automation context and secure/elevated-target checks.
8. Raise hook-thread priority where appropriate.
9. Use the shell heartbeat and sidecar restart path as the recovery mechanism.
10. Test on real Windows hardware or a representative VM.

Exit criteria:

- A saturating inference workload does not break the shortcut.
- Sidecar termination is detected and recovered.
- No Start menu or stuck-modifier regressions occur.
- Injection works in standard, Chromium, terminal, and elevated-target scenarios where Windows permits it.

### Phase 6 — Packaging, permissions, and release hardening

1. Bundle the sidecar with Tauri for macOS and Windows.
2. Ensure the executable path works in development and packaged builds.
3. Sign the helper binary.
4. Include it in notarization and release verification.
5. Verify macOS Accessibility/Input Monitoring behavior for the helper identity.
6. Verify upgrades replace shell and sidecar atomically.
7. Add protocol-version diagnostics for mixed-version installations.
8. Ensure uninstall removes all bundled components.
9. Ensure logs never contain transcript text unless explicitly running a development diagnostic mode.
10. Add crash-loop limits and a user-visible recovery path.

Exit criteria:

- A clean packaged install completes permissions and dictation successfully.
- Signing/notarization passes.
- Upgrading cannot leave an incompatible shell/sidecar pair running.
- No orphan sidecar remains after quit, crash recovery, or uninstall.

### Phase 7 — Remove temporary in-process implementations

Only after sidecar parity is proven:

1. Remove the production in-process keyboard hook.
2. Remove duplicated native insertion paths or retain them only under a development feature.
3. Simplify `hotkey.rs` so it consumes sidecar triggers and coordinates the core pipeline.
4. Keep the platform-independent state machine unchanged.
5. Update architecture documentation to match the shipped ownership split.

Exit criteria:

- All supported platforms use the supervised sidecar.
- No two hooks can be active simultaneously.
- No duplicate paste can occur.
- Documentation and packaging match the implementation.

## Suggested first implementation slice

The safest first pull request should contain only:

1. Persistent framed sidecar process.
2. `Hello`/`Ready`.
3. `Ping`/`Pong`.
4. Periodic heartbeat.
5. Clean shutdown.
6. Shell supervisor with spawn, read loop, timeout, restart, and bounded backoff.
7. Integration tests that kill and restart the helper.

Do not move the live keyboard hook in the first slice. Establish lifecycle reliability before putting user input through it.

## Testing plan

### Protocol tests

- Round-trip every IPC message.
- Reject unknown protocol versions.
- Reject frames larger than `MAX_FRAME_LEN`.
- Handle partial reads and writes.
- Handle EOF and broken pipes.
- Confirm stderr cannot corrupt stdout framing.

### Supervisor tests

- Spawn and handshake.
- Delayed handshake timeout.
- Child exits before `Ready`.
- Missing heartbeat.
- Broken stdin.
- Manual child kill.
- Repeated crash loop and backoff.
- Clean parent shutdown.
- No orphan process.

### Hook tests

- Fn hold/release.
- Fast single tap produces no dictation.
- Double tap enters locked mode.
- Re-press exits locked mode.
- Esc cancels.
- Other keys are not swallowed.
- Modifier state is reconciled after focus changes.
- Tap disable/re-enable.
- Full tap recreation.
- Sidecar restart while no key is held.
- Sidecar restart while a key appears held.

### Stress tests

- Run Parakeet transcription while repeatedly triggering the hotkey.
- Run local llama.cpp cleanup while repeatedly triggering the hotkey.
- Saturate CPU cores and verify no trigger loss.
- Crash or terminate the inference worker and verify the sidecar remains responsive.
- Kill the sidecar and verify supervised recovery.

### Injection tests

- Cursor and VS Code.
- Chromium browser fields.
- Native macOS text fields.
- Terminal and terminal-based AI agents.
- Long text requiring chunking.
- Unicode-heavy text.
- Password fields.
- Clipboard restoration.
- Targets that reject ordinary paste.

## Important engineering constraints

- Never perform inference, Accessibility calls, clipboard waits, logging I/O, or JSON serialization in the low-level hook callback.
- The hook callback should do the minimum necessary and return immediately.
- Never allow both the sidecar hook and in-process hook to suppress the same key.
- Preserve ordering between trigger events.
- Treat stdout as IPC-only.
- Use request IDs before allowing concurrent request/response operations.
- Use timeouts for every shell-to-sidecar operation that expects a response.
- Avoid logging dictated or selected text.
- Keep protocol changes backward-incompatible only when necessary, and increment `PROTOCOL_VERSION` when they are.
- Do not silently fall back to an unhealthy helper in production; surface a clear error.

## Key risks

- macOS permissions may attach to the helper executable rather than the Tauri shell, requiring onboarding changes.
- Two simultaneous hooks can double-trigger or suppress unrelated keys during migration.
- Restarting while a modifier is held can create stale-key state.
- Clipboard restoration and paste completion are timing-sensitive across IPC.
- Shell and sidecar updates must remain version-compatible.
- Windows security software may classify a global hook plus injection as suspicious.
- Automatic restart can become an uncontrolled crash loop without backoff.

## Definition of done

The sidecar milestone is complete when:

- The bundled Rust helper owns hotkeys and text insertion on macOS and Windows.
- The Tauri shell supervises it and automatically recovers from helper failure.
- Heavy ASR/LLM work cannot starve or remove the keyboard hook.
- Hook health and stale-key recovery are implemented.
- Existing dictation behavior and insertion reliability are preserved.
- Secure fields remain protected.
- Packaged builds are signed, notarized where required, and leave no orphan processes.
- The old production in-process hook cannot run concurrently with the sidecar hook.

