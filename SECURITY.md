# Security

## Supported version

Security fixes are applied to the latest commit on `main` while the project is pre-1.0.

## Reporting

Please report vulnerabilities privately through GitHub's private vulnerability reporting feature when available. Do not include credentials, provider session files, MCP tokens, or repository secrets in public issues.

## Trust boundaries

Osanwe launches user-installed Codex CLI, Grok Build, Git, and Zellij executables. Their binaries, authentication, configuration, plugins, skills, hooks, MCP servers, and other extensions remain in the user's trust domain.

Selected providers run in the original project directory. Codex roles default to `workspace-write` for orchestrator/worker and `read-only` for planner/verifier; these defaults are not a complete security boundary. Grok permissions follow the user's Grok configuration and command-line defaults.

The project-local `.osanwe/` directory contains configuration, role prompts, coordination files, and provider session identifiers. Treat repository-controlled prompt files as trusted input before launching Osanwe, and keep `.osanwe/sessions/` out of version control.

Osanwe does not add a daemon, local RPC socket, MCP bridge, hook overlay, or isolated Git worktree to the primary file-bus flow.
