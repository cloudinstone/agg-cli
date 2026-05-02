# aag

`aag` is a macOS CLI for managing local Antigravity IDE accounts.

It handles Google OAuth login, stores multiple local accounts, shows quota and plan status, and switches the active IDE account by rewriting the local session state and restarting Antigravity.

## Scope

- Supported platform: macOS
- Target app: Antigravity IDE
- Local storage: `~/.aag-cli/accounts.json`
- Current switch behavior: rewrite local state, close Antigravity, restart Antigravity

This project does not currently support Windows or Linux. It also does not do true in-process hot account switching; in practice Antigravity keeps enough auth state in memory that restart-based switching is the reliable path.

## Install

### Homebrew tap

After a release is published:

```bash
brew tap cloudinstone/tap
brew install aag
```

Or in one command:

```bash
brew install cloudinstone/tap/aag
```

Note: this is a custom tap, not `homebrew/core`. That means `brew install aag` only works after tapping `cloudinstone/tap` first, or if the formula is ever accepted into the official Homebrew core repository.

### From source

Requirements:

- Rust toolchain
- macOS
- Antigravity installed locally

Build:

```bash
cargo build --release
```

Binary path:

```bash
./target/release/aag
```

Optional one-time symlink:

```bash
mkdir -p ~/.local/bin
ln -sf "$PWD/target/release/aag" ~/.local/bin/aag
```

If `~/.local/bin` is already in your `PATH`, that is enough. No shell profile changes are required for a one-off local symlink.

## Usage

Common commands:

```bash
aag login <alias>
aag add <alias>
aag switch
aag list
aag status
aag status --internal
aag logout
aag remove
aag clean
```

### `login`

Starts browser-based Google OAuth login and stores the returned account locally.

```bash
aag login work
```

### `add`

Adds an existing account entry by alias flow if you want to keep multiple named local accounts.

```bash
aag add backup
```

### `list`

Shows all saved accounts, sorted by:

1. Available accounts first
2. Then alphabetically by email

The output includes plan badge (`FREE` or `PRO`), account email, quota-related status, and availability.

### `switch`

Interactive account picker. `Esc` exits without switching.

Switch order matches `list`: available first, then alphabetical.

### `status`

Shows the currently active IDE account in the same richer format used by `list`.

```bash
aag status
```

With `--internal`, it also prints raw API responses for:

- `v1internal:loadCodeAssist`
- `v1internal:fetchAvailableModels`

```bash
aag status --internal
```

This is mainly for debugging and only works if the current stored account still has a usable `refresh_token`.

### `remove`

Interactive account removal. `Esc` exits without deleting anything.

### `clean`

Removes invalid or duplicate saved entries.

## Data and Auth

Account data is stored locally in:

```text
~/.aag-cli/accounts.json
```

OAuth client details are not hardcoded in the source. At runtime, `aag` extracts the required client metadata from the locally installed Antigravity app bundle, then uses that to complete the OAuth flow.

## Development

Run tests:

```bash
cargo test
```

Run the CLI in development:

```bash
cargo run -- list
```

## Release and Homebrew

This repository is maintained as:

- Private source repo: full working tree, including `internal/`
- Public mirror repo: export used for open source release
- Homebrew tap repo: stores `Formula/aag.rb`

Release flow:

1. Push to the private source repo.
2. The public sync workflow exports the repo while excluding `internal/`.
3. Tag a version like `v0.1.0` in the private repo, or run the release workflow manually with a version input.
4. The release workflow builds macOS archives, uploads them to the public GitHub release, renders the Homebrew formula, and publishes `Formula/aag.rb` to `cloudinstone/homebrew-tap` if the required secrets are configured.

Relevant files:

- `scripts/sync-public.sh`
- `.github/workflows/sync-public.yml`
- `.github/workflows/release.yml`
- `scripts/package-release.sh`
- `scripts/render-homebrew-formula.sh`
- `scripts/publish-homebrew-tap.sh`

## Repository Notes

`internal/` stays only in the private source repository. The public repository is generated from the private one and is intended to be safe for open source publication.
