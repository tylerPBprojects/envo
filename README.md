# envo

**Environments that start in 50ms and download nothing you don't use.**

envo is a Nix-based developer environment runtime. It resolves packages from nixpkgs, generates lightweight shims that fetch binaries on first use, and activates your environment by sourcing a single file — no subshell, no container, no multi-GB download before your first `make`.

```
$ time source <(envo activate --inline)
real    0m0.003s    # 3ms. Not a typo.
```

## Install

```
curl -sSf https://envo.dev/install.sh | sh
```

Or build from source:

```
git clone https://github.com/tylerPBprojects/envo.git
cd envo
cargo build --release
cp target/release/envo ~/.envo/bin/envo
```

Requires [Nix](https://install.determinate.systems/nix) for package management. envo will prompt you to install it if needed.

## Quickstart

```
# Create an environment
envo init

# Add packages
envo install ripgrep jq python3

# Activate (sets PATH + env vars, no subshell)
source <(envo activate --inline)

# Use your tools — they fetch on first run, instant after that
rg --version     # first run: downloads ripgrep (~2s), then executes
rg --version     # second run: instant (<5ms)
```

## PyTorch + CUDA in 50ms

The hardest environment problem in software — multi-GB GPU dependencies — in five commands:

```
envo init --template cuda-pytorch
envo install
source <(envo activate --inline)     # 50ms. Nothing downloaded yet.
python3 -c "import torch; print(torch.cuda.is_available())"  # fetches on first use
```

Works on both GPU and CPU machines. PyTorch detects CUDA at runtime, not install time.

Run the full demo: `bash templates/cuda-pytorch/demo.sh`

## How It Works

Traditional tools download everything at activation. envo doesn't.

```
Traditional tools:

  activate → download dependencies → wait → ready

envo:

  activate → set PATH (3ms) → ready
  first use of rg → download rg (2s) → exec
  first use of python → download python (5s) → exec
  second use of anything → instant
```

The trick: envo generates bash shims in `.envo/bin/` that look like real binaries. When you call `rg`, the shim checks if the Nix store path exists. If not, it fetches it. Then it `exec`s the real binary with your arguments. After the first fetch, the shim is just a fast-path `exec`.

## Features

**Core runtime:**

- Lazy package realization — nothing downloads until invoked
- Sub-100ms activation — sources a precomputed env snapshot, no subshell
- Simple TOML manifest — feels like `pyproject.toml` or `Cargo.toml`
- CycloneDX SBOM export — `envo export sbom`
- Built-in templates — `envo init --template cuda-pytorch`
- Auto-updater — `envo self-update`

**IDE integration:**

- VS Code extension — auto-activates terminals, status bar, package management commands
- Manifest LSP — inline diagnostics, autocompletion, hover docs for `manifest.toml`

**AI agent integration:**

- MCP server — 6 tools + 3 resources for Claude Code, Cursor, and other MCP clients
- Structured JSON output — `envo version --json`, `envo search --json`

**DevOps:**

- Nix bootstrap — detects and offers to install Nix interactively
- POSIX installer — `curl | sh`, no sudo required
- Telemetry — opt-out, privacy-conscious, PostHog-backed

## Manifest Format

Environments are defined in `.envo/manifest.toml`:

```
[project]
name = "my-app"
description = "My application"

[packages]
ripgrep = "*"
python = { pkg-path = "python312" }
jq = "1.7"

[vars]
EDITOR = "vim"
DATABASE_URL = "postgres://localhost/dev"

[hook]
on-activate = '''
echo "Welcome to my-app!"
'''

[options]
allow-unfree = true
```

## CLI Reference

```
envo init [--template <name>]    Create a new environment
envo install [packages...]       Install packages (or resolve existing manifest)
envo uninstall <package>         Remove a package
envo activate [--inline]         Print activation script or path
envo deactivate [--inline]       Print deactivation script
envo search <query> [--json]     Search nixpkgs
envo run <command> [args...]     Run a command in the environment
envo update                      Update all packages
envo export sbom                 Export CycloneDX SBOM
envo version [--json]            Show version, Nix status, system info
envo self-update [--check]       Update envo itself
```

## VS Code Extension

The extension auto-activates your environment when you open a project:

1. Install the extension from `envo-vscode/`
2. Open a project with `.envo/manifest.toml`
3. New terminals automatically have the environment active
4. Use the command palette for package management

## MCP Server

Connect Claude Code, Cursor, or any MCP client:

```
{
  "mcpServers": {
    "envo": {
      "command": "envo-mcp"
    }
  }
}
```

Available tools: `envo_init`, `envo_install`, `envo_uninstall`, `envo_search`, `envo_env_info`, `envo_activate`

Available resources: `envo://manifest`, `envo://lockfile`, `envo://status`

## How envo Compares

|                    | envo      | other tools    |
| ------------------ | --------- | --------- |
| Activation speed   | <100ms    | 1-5s      |
| Lazy fetch         | Yes       | No        |
| Subshell           | No        | Yes       |
| Manifest LSP       | Yes       | No        |
| MCP server         | Yes       | No        |
| SBOM export        | Yes       | No        |
| Nix syntax exposed | Never     | Sometimes |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    Surfaces                          │
│  ┌─────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │   CLI   │  │ VS Code Ext  │  │  MCP Server    │  │
│  └────┬────┘  └──────┬───────┘  └───────┬────────┘  │
│       │              │                  │            │
│  ┌────┴──────────────┴──────────────────┴────────┐  │
│  │              envo library (Rust)               │  │
│  │  ┌──────────┐ ┌────────┐ ┌─────────┐         │  │
│  │  │ Manifest │ │Lockfile│ │Realizer │         │  │
│  │  └──────────┘ └────────┘ └─────────┘         │  │
│  │  ┌──────────┐ ┌─────────────┐ ┌───────────┐  │  │
│  │  │Activator │ │Nix Bootstrap│ │ Telemetry │  │  │
│  │  └──────────┘ └─────────────┘ └───────────┘  │  │
│  └───────────────────────┬───────────────────────┘  │
│                          │                           │
│                    ┌─────┴─────┐                     │
│                    │    Nix    │                      │
│                    └───────────┘                     │
└─────────────────────────────────────────────────────┘
```

## Project Layout

```
envo/
├── src/
│   ├── manifest/        # TOML schema, parsing, validation
│   ├── lockfile/        # Nix resolution, store paths
│   ├── realize/         # Shim generation, lazy fetch
│   ├── activate/        # Shell-sourceable env snapshots
│   ├── cli/             # Command routing and handlers
│   ├── lsp/             # Language server (diagnostics, completion, hover)
│   ├── mcp/             # MCP server (tools, resources, protocol)
│   ├── telemetry.rs     # PostHog telemetry (opt-out)
│   ├── nix_bootstrap.rs # Nix detection and installation
│   ├── self_update.rs   # Auto-updater via GitHub releases
│   └── templates.rs     # Embedded environment templates
├── tests/
│   ├── integration/     # Bash end-to-end tests
│   └── *.rs             # Rust unit + integration tests
├── templates/           # Reference template files + demo scripts
├── install.sh           # POSIX installer
└── uninstall.sh         # POSIX uninstaller
```

## Development

```
# Build all binaries (envo, envo-lsp, envo-mcp)
cargo build

# Run all 205 tests
cargo test

# Run integration tests (requires Nix)
bash tests/integration/test_full_workflow.sh
bash tests/integration/test_lazy_fetch.sh
bash tests/integration/test_cuda_demo.sh
bash tests/integration/test_mcp.sh
bash tests/integration/test_lsp.sh
```

## Telemetry

envo collects anonymous usage data to understand how the product is used. Telemetry is opt-out — disable it in `~/.envo/config.toml`:

```
[telemetry]
enabled = false
```

We collect: command name, success/failure, duration, OS, and app version. We never collect: source code, file contents, secrets, environment variable values, file paths, or command arguments.

## License

MIT
