# aag

`aag` is a macOS CLI for managing local Antigravity IDE accounts.

It handles Google OAuth login, stores multiple local accounts, shows quota and plan status, and switches the active IDE account locally.

## Scope

- Supported platform: macOS
- Target app: Antigravity IDE
- Local storage: `~/.aag-cli/accounts.json`

This project does not currently support Windows or Linux.

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

If `~/.local/bin` is already in your `PATH`, that is enough.

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

## Data

Account data is stored locally in:

```text
~/.aag-cli/accounts.json
```

## Contributing

Requirements:

- Rust toolchain
- macOS
- Antigravity installed locally

Build:

```bash
cargo build
```

Run tests:

```bash
cargo test
```

Run the CLI in development:

```bash
cargo run -- list
```
