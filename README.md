# Robit Link Mover

Windows-only GUI utility for moving a file or folder to another drive/location while keeping the original path alive through a link.

## Prerequisites

- Windows 10/11
- Node.js 20+
- Rust stable toolchain with Cargo
- Microsoft WebView2 Runtime

## Development

```powershell
npm install
npm run typecheck
npx vite build
npm run dev
```

The native Tauri build requires Cargo to be available in `PATH`.
The helper is built before `tauri dev`/`tauri build`, so both executables are available under `src-tauri\target\debug` or `src-tauri\target\release`.

## Behavior

- Select a source file or folder.
- Select a destination parent folder.
- The app creates the final destination by appending the source item name.
- Safe mode copies first, verifies, deletes the source, then creates the link.
- `Robocopy /MOVE` is available as an advanced mode for large folder moves.
- Completed operations are stored in `%LOCALAPPDATA%\RobitLinkMover\operations.sqlite`.
- Logs are stored in `%LOCALAPPDATA%\RobitLinkMover\logs`.
- The elevated helper is launched through UAC only for move/rollback operations.

## Current Build Note

This workspace currently does not expose `cargo`/`rustc` in `PATH`, so only the TypeScript and Vite frontend checks were run here.
