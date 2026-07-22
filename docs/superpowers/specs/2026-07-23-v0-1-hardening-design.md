# Osanwe v0.1 Hardening Design

## Goal

Make the project-local `.osanwe` file-bus flow the only product architecture, fix the release and runtime reliability issues found in the repository audit, and remove code and configuration that no longer serve that flow.

## Decision

Use a full pre-1.0 cutover. Hidden legacy commands and their daemon, MCP bridge, hook, state machine, provider overlay, home-state store, checks, worktree, pane, and legacy TUI modules are removed without a compatibility shim.

Alternatives rejected:

- Keeping both architectures preserves compatibility but leaves more than half the Rust and test code attached to a hidden product.
- Feature-gating the legacy path reduces default binary surface but retains the maintenance and security burden.

## Runtime architecture

The only supported flow is:

```text
CLI -> onboarding/project config -> role launch specs -> Zellij panes
                                      `-> project-local .osanwe file bus
```

Role bootstrap instructions are passed as each provider's native initial prompt argument. This removes timing sleeps and Zellij paste/send-key injection. The board pane rereads its source files on a short interval so it reflects current todos and status.

`onboard --defaults` follows the same overwrite rule as interactive onboarding: an existing config requires `--force`. `doctor` requires Zellij and at least one provider, while Git and the unused provider are reported as optional. `stop` ignores only a confirmed missing session and returns other Zellij failures.

## Configuration and dependencies

Verifier enablement is represented only by `roles.verifier: Option<RoleChoice>`; existing `enable_verifier` keys remain loadable because Serde ignores unknown fields. The unused session record is removed because `config.toml` already contains the session and active roles.

The non-security project-path suffix uses the standard-library hasher, allowing `sha2` and `hex` to be removed. `thiserror` disappears with the legacy state machine. Clap, Tokio, and UUID use only required features. The single-implementation `PaneHost` trait becomes inherent `ZellijPaneHost` methods.

## Release and documentation

The release workflow builds on version tags and manual dispatch, but publishes only version tags whose value matches `Cargo.toml`. Existing releases are not overwritten. Write permission is scoped to the publish job. The duplicate Clippy diagnostics workflow and stale legacy architecture documents are deleted.

Security documentation describes the actual trust boundary: selected providers run in the original project, Codex role sandboxes are defaults rather than a complete boundary, Grok permissions come from Grok configuration, and project-local prompts are trusted input.

## Error handling

- Provider startup receives its prompt atomically through argv; pane creation failure aborts launch.
- Board rendering surfaces filesystem read errors instead of silently showing stale data.
- Config overwrite attempts fail before any write.
- Non-missing Zellij stop failures preserve the error status and stderr.
- Release publication fails on tag/version mismatch or an already-existing release.

## Testing

- Add a regression test proving `onboard --defaults` cannot overwrite without `--force`.
- Add command-spec tests proving bootstrap text is included as the native initial prompt.
- Add board rendering tests using temporary files.
- Add pure decision tests for doctor requirements and stop-error classification.
- Update configuration tests to derive verifier enablement from `Option`.
- Remove tests that only cover the deleted legacy product.
- Finish with format, Clippy, all Rust tests, installer tests, release build, shell syntax, and workflow/config inspection.

## Non-goals

- No compatibility layer for hidden commands.
- No dynamic provider model discovery in this pass.
- No authenticated live-provider CI test.
- No new dependencies.
