# Security

## Supported version

Security fixes are applied to the latest commit on `main` while the project is pre-1.0.

## Reporting

Please report vulnerabilities privately through GitHub's private vulnerability reporting feature when available. Do not include credentials, provider session files, MCP tokens, or repository secrets in public issues.

## Trust boundaries

Osanwe launches user-installed Codex CLI, Grok Build, Git, and Zellij executables. Their binaries, existing extensions, and authenticated MCP servers remain in the user's trust domain. Osanwe adds a role-scoped local MCP bridge and hooks but does not treat hook delivery as a complete sandbox.

Run state uses user-only filesystem permissions and agent-scoped random tokens. The original repository working tree is not used as a writable provider workspace.
