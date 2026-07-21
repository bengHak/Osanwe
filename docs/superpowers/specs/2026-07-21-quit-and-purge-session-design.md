# Quit and Purge Session Design

## Goal

Pressing `Ctrl+Q` inside an Osanwe Zellij session must stop every pane process, close the session, and delete its resurrectable Zellij metadata.

Detaching with Zellij's normal detach command must keep the live session available for reattachment.

## Design

Keep Zellij's native global `Ctrl+Q` binding instead of adding an Osanwe key-event layer. After `zellij attach` returns successfully, Osanwe checks whether the named session is still active:

- Active means the user detached, so Osanwe leaves the session untouched.
- Inactive means the user quit, so Osanwe runs `zellij delete-session <session>`.
- An already-absent session is treated as clean; other deletion failures are reported.

The launch output and README will identify `Ctrl+Q` as the quit-all-and-delete shortcut.

## Testing

Use the existing command-runner seam to verify two cases without launching an interactive terminal:

1. An active session is preserved after attach returns.
2. An inactive session triggers `delete-session` for the exact session name.

Run the complete existing Rust and installer CI commands after implementation.

## Scope

No custom Zellij config, background watcher, new dependency, confirmation dialog, or configurable shortcut is added.
