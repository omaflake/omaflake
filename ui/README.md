# omaflake UI

The Linux desktop UI is a Tauri 2 application with a TypeScript, React, and Vite frontend. It is intentionally
separate from the existing macOS SwiftUI app. The current screen uses clearly labelled preview data; VM actions are
not connected to `omaf` yet.

## Development on x86_64 NixOS

From the repository root:

```sh
nix develop path:./ui
cd ui
pnpm install
pnpm tauri dev
```

For frontend-only work in a browser:

```sh
pnpm dev
```

The Tauri frontend/backend protocol and live `omaf` integration are tracked separately in
[omaflake/omaflake#4](https://github.com/omaflake/omaflake/issues/4).
