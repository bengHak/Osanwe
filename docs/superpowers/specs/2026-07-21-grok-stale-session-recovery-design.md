# Grok stale-session recovery

## Problem

Osanwe treats any non-empty `.osanwe/sessions/<role>.session-id` marker as resumable. If Grok never created that session, every later launch uses `--resume` and exits with `404 Not Found`.

## Design

Before choosing `--resume`, Osanwe will verify that the saved UUID exists under Grok's local session store. A valid local session keeps the current `--resume <uuid>` behavior. A missing session is stale: Osanwe overwrites the marker with a new UUID and launches with `--session-id <uuid>`.

The check stays in `role_session_binding`, where all Grok role launches already choose between new and resumed sessions. No retry wrapper, output parsing, or new dependency is needed.

## Failure handling

If Grok's session store cannot be found or read, Osanwe favors a launchable new session instead of attempting an unverified resume. Existing Grok session data is never deleted.

## Test

One regression test will create a stale marker without a matching Grok session and assert that Osanwe replaces it and emits `--session-id`, not `--resume`. The existing resume test will create a matching fake session directory and continue to assert `--resume`.
