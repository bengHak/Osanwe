# Curl Installation and Release Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let developers install Osanwe with a checksum-verified `curl | sh` command backed by native Linux and macOS GitHub release binaries.

**Architecture:** A POSIX installer detects the host target, downloads the matching release archive and `SHA256SUMS`, verifies the archive, and atomically installs `osanwe` into a configurable directory. GitHub Actions builds four native platform archives, validates them on pull requests, and creates or updates the Cargo-version release from `main`.

**Tech Stack:** POSIX shell, GitHub Actions, Rust 1.82, GitHub Releases

## Global Constraints

- Keep Codex and Grok execution fully interactive.
- Do not introduce `codex exec`, Grok `-p`, app-server, or ACP.
- Support Linux, macOS, and WSL on x86_64 and aarch64 where release runners exist.
- Verify every downloaded binary archive with SHA-256 before installation.
- Preserve authenticated installation for the private GitHub repository.

---

### Task 1: Installer behavior tests

**Files:**
- Create: `tests/install.sh`

**Interfaces:**
- Consumes: release archive name `osanwe-<rust-target>.tar.gz`
- Produces: executable regression test invoked as `sh tests/install.sh`

- [x] Write a fake release fixture and fake `curl` implementation.
- [x] Verify Linux and macOS target detection.
- [x] Verify authenticated headers, installation, execution, unsupported hosts, and checksum rejection.
- [x] Run `sh tests/install.sh` and observe failure before `install.sh` exists.

### Task 2: POSIX curl installer

**Files:**
- Create: `install.sh`

**Interfaces:**
- Consumes: `OSANWE_VERSION`, `OSANWE_INSTALL_DIR`, `OSANWE_GITHUB_TOKEN`, `GH_TOKEN`, `OSANWE_REPOSITORY`, `OSANWE_OS`, and `OSANWE_ARCH`
- Produces: installed executable at `<install-dir>/osanwe`

- [x] Detect supported Rust release targets.
- [x] Download release assets with public or authenticated GitHub requests.
- [x] Fall back to the GitHub Releases API for private assets.
- [x] Verify `SHA256SUMS` before extraction.
- [x] Atomically install an executable binary and print PATH guidance.
- [x] Run the installer regression test to verify green behavior.

### Task 3: Release assets

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: Cargo package version and source at the workflow commit
- Produces: four `.tar.gz` assets and `SHA256SUMS` on release `v<package-version>`

- [x] Build native x86_64 and aarch64 binaries for Linux and macOS.
- [x] Package the binary with README and license files.
- [x] Validate the matrix on pull requests.
- [x] Publish or replace assets after a successful `main` build.

### Task 4: Continuous verification

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `install.sh` and `tests/install.sh`
- Produces: shell syntax and installer behavior gates

- [x] Add POSIX shell syntax validation.
- [x] Run the installer regression suite in CI.
- [ ] Run Rust formatting, Clippy, tests, release build, and installer tests on the final branch.

### Task 5: Installation documentation

**Files:**
- Modify: `README.md`

**Interfaces:**
- Produces: authenticated private-repository and public-repository curl commands

- [x] Document token-authenticated `curl | sh` installation.
- [x] Document public raw URL installation for a future public repository.
- [x] Document version, destination, target, PATH, and source-build options.
- [ ] Confirm the documented command against a published release asset.
