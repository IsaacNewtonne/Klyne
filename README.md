# Klyne

**A local-first AI workspace. A little spark. Something real.**

Give Klyne a task, watch the steps unfold, and inspect the result. Built in Rust with a simple browser interface, local model support.

[![Klyne workspace preview open the interactive demo](site/preview.png)](https://isaacnewtonne.github.io/Klyne/)

### [Open the interactive workspace](https://isaacnewtonne.github.io/Klyne/)

Run a sample task, switch between conversation and workspace, or open Details. The demo uses the real presentation code with simulated task data. It makes no model calls and cannot access your computer. GitHub README files cannot run JavaScript; the live experience opens on GitHub Pages.

## What Klyne does

- **Work you can follow.** Persisted task dependencies, progress, evidence, and a readable final answer.
- **Models you choose.** Ollama, Codex, and OpenCode connections; a separately configured fallback can handle model-request failures.
- **Tools with explicit access.** Files, browser automation, desktop controls, terminal commands, saved APIs, and curated MCP integrations.
- **Recovery grounded in evidence.** Completed steps survive restarts. Known pre-dispatch connection failures can switch to desktop observation. Uncertain actions are preserved instead of blindly repeated.
- **Checks beyond a model's claim.** Caller-owned file contracts, supported desktop focus/field checks, and a first Notepad save-reconciliation adapter.
- **A quieter interface.** Visual progress, compact steps, optional details.

Klyne is under active development. Desktop automation currently targets Windows. App coverage depends on available adapters, accessibility controls, and verifiable results; arbitrary message delivery, purchases, deletes, and every application's save behavior are not universally verified. A fallback model using the same Ollama service still shares that service's failure modes.

## Run locally

Install a current stable Rust toolchain with edition-2024 support and the platform's native build tools. On Windows, use the MSVC toolchain and Visual Studio C++ build tools. Install and start Ollama if you want local inference; choose a model that fits your hardware.

```powershell
git clone https://github.com/IsaacNewtonne/Klyne.git
cd Klyne
cargo build --locked -p klyne-studio --bins
.\target\debug\klyne-supervisor.exe --root workspace/studio --port 4317
```

Open **http://127.0.0.1:4317**, select your provider in Settings, and check the connection. Enable only the tools needed for your task. Terminal and desktop access can act on your real computer; the public demo has neither capability.

The runtime stores local conversations and generated artifacts under `workspace/`, which is excluded from Git. No model weights are bundled. Chrome or Edge is needed for browser workflows; Node/npm is needed for the curated browser MCP adapter. See [provider setup](docs/studio-providers.md) and [app connections](docs/app-connections.md).

## Development

```powershell
cargo test --locked -p harness-core
cargo test --locked -p klyne-studio --bin klyne-studio --test chat --test runtime -- --test-threads=1
```

Studio browser tests require an installed browser. The optional live MCP test is ignored by default. Windows desktop fixture checks can be run with `powershell -MTA -NoProfile -File scripts/test_desktop_recovery.ps1`; they operate a disposable test window.

Build and preview the static demo without starting Klyne:

```powershell
python scripts/build_site.py
python -m http.server 8080 --directory site
```

Open **http://localhost:8080**. The Pages workflow rebuilds presentation assets from Studio when publishing. Configure GitHub Pages to use **GitHub Actions** for deployment.

## Explore the project

| Area | Guide |
| --- | --- |
| Studio and providers | [Workspace](docs/studio.md) ? [Providers](docs/studio-providers.md) |
| App integrations | [MCP and desktop routes](docs/app-connections.md) |
| Recovery | [Recovery service](docs/recovery.md) |
| Architecture and next steps | [Autonomy roadmap](docs/autonomy-roadmap.md) |
| Core runtime | [Harness overview](docs/harness-overview.md) |

The Rust workspace includes the core runtime, providers, browser control, benchmarks, memory, experiments, CLI, and Studio. The model proposes actions; the host owns execution, permissions, persistence, and independent checks.
