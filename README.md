# envo

A Nix-based developer environment runtime with lazy package realization and instant activation.

## What is envo?

envo wraps Nix to provide developer environments that:

- **Activate in under 100ms** — no subshell, no per-shell overhead
- **Download nothing until you use it** — packages are fetched on first invocation, not at activation
- **Work across platforms** — Linux (x86_64, aarch64) and macOS (aarch64)
- **Never expose Nix syntax** — the manifest is a simple TOML file

## Quick Start

```bash
# Build from source
cargo build --release

# Initialize a new environment
envo init

# Add packages
envo install ripgrep python3 jq

# Activate (source the env snapshot)
source <(envo activate --inline)

# Use your tools — they fetch on first run
rg --version    # downloads ripgrep on first use, instant thereafter
```

## Manifest Format

envo environments are defined in `.envo/manifest.toml`:

```toml
[project]
name = "my-app"
description = "My application"

[packages]
ripgrep = "*"                                          # latest version
python = { version = "3.12", pkg-path = "python3" }    # specific version + nixpkgs attr
jq = "1.7"                                             # pinned version
nodejs = { priority = 10 }                             # priority for PATH ordering

[vars]
EDITOR = "vim"
DATABASE_URL = "postgres://localhost/dev"

[hook]
on-activate = '''
echo "Welcome to my-app!"
'''

[services.postgres]
command = "postgres -D ./data/pg"
shutdown = "pg_ctl stop -D ./data/pg"

[options]
nixpkgs-channel = "nixpkgs/nixos-24.11"
allow-unfree = true
systems = ["x86_64-linux", "aarch64-linux"]
```

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run all tests (unit + integration)
cargo test

# Run only manifest tests
cargo test manifest

# Run bash integration tests
bash tests/integration/test_init.sh
```

## Architecture

envo is built in five modules:

1. **Manifest** (`src/manifest/`) — TOML schema, parsing, validation
2. **Lockfile** (`src/lockfile/`) — Nix resolution, per-system store paths
3. **Realize** (`src/realize/`) — lazy shim generation, on-demand fetching
4. **Activate** (`src/activate/`) — shell-sourceable env snapshot generation
5. **CLI** (`src/cli/`) — user-facing commands tying it all together

## Project Layout

```
.envo/
├── manifest.toml       # Environment declaration (you edit this)
├── manifest.lock        # Resolved store paths (generated, commit to git)
├── bin/                 # Shim scripts (generated)
│   ├── rg
│   ├── python3
│   └── jq
└── env-snapshot.sh      # Activation script (generated)
```

## Development Status

Session 1 (Manifest) — ✅ Complete
Session 2 (Lockfile) — ✅ Complete
Session 3 (Realize) — ✅ Complete
Session 4 (Activate) — ✅ Complete
Session 5 (CLI) — ✅ Complete

## Testing

```bash
# All unit and integration tests (no Nix required)
cargo test

# Full end-to-end workflow (requires Nix + network)
cargo build
bash tests/integration/test_full_workflow.sh

# Error case handling
bash tests/integration/test_error_cases.sh

# Individual module tests
bash tests/integration/test_activate.sh
bash tests/integration/test_lazy_fetch.sh
bash tests/integration/test_install.sh
```

## License

MIT
