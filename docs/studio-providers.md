# Local providers in Klyne Studio

Choose **Run with** in the creation form. Open **Connection**, check the
connection, and choose a model. The choice is saved in this browser; each run
also stores its own provider settings so resuming does not switch providers.
The built-in file agent remains available without a model server.

| Provider | Connection | Setup |
| --- | --- | --- |
| Ollama | Local HTTP, default `http://127.0.0.1:11434` | Start Ollama; Check connection lists installed models from `/api/tags`. Select one, then launch. |
| OpenCode | Local HTTP, default `http://127.0.0.1:4096` | Run `opencode serve --hostname 127.0.0.1 --port 4096`; configure its provider login in OpenCode. Select a `provider/model` from discovery. |
| Codex | Installed CLI | Run `codex login` once. Studio reuses CLI authentication; no API key is entered into the page. Model is optional. |

Ollama inference runs through its local OpenAI-compatible chat endpoint.
OpenCode is a local agent server; its selected model can still be hosted by a
cloud provider. Codex uses the installed CLI and its signed-in account; it does
not imply offline inference. Connection checks verify service reachability or
CLI login, not entitlement to every advertised model.

Studio currently creates exact-content file objectives. These integrations
supply decisions to that existing runtime; they do not turn the form into a
general coding assistant. Every proposed file action is validated and executed
by Klyne, and completion requires independent read-back verification.

OpenCode sessions receive a deny-all tool permission ruleset. Codex runs with
an explicit read-only sandbox, ephemeral sessions, and without user config or
exec rules, in the isolated run directory. Its prompt requests JSON decisions
only. The integration does not provide an OS sandbox beyond the provider's
own protections. Codex authentication remains in the existing CLI credential
store. OpenCode basic auth is supported through server-side
`OPENCODE_SERVER_PASSWORD` and optional `OPENCODE_SERVER_USERNAME`; these are
never sent to the browser or saved in run settings.

`KLYNE_CODEX_BIN` can point to a native Codex executable. Otherwise Studio
finds a native executable on PATH or invokes the npm JavaScript entrypoint
through Node, without shell interpolation. Restart Studio after changing its
environment. HTTP endpoints must be loopback addresses without URL credentials,
paths, queries, or fragments. HTTP connection checks time out; catalog responses
are capped at 8 MiB and decision responses at 512 KiB. Codex subprocess output
and execution time are bounded; Windows timeout cleanup kills the process tree.

A model request can take up to 120 seconds. Stop is cooperative between runtime
steps, so a pending provider request may finish before the stop takes effect.
No silent provider fallback occurs. An unavailable provider or invalid decision
is reported as an error, and a model-proposed unsafe path is refused.

## Verification

`cargo test -p klyne-studio --locked` covers connection validation, persisted
settings, an Ollama-compatible HTTP fixture through independent file verification
and token accounting, browser provider selection/discovery/persistence, and the
existing create/export/stop/resume and mobile flows. Fixture tests do not contact
paid services. Real local smoke checks and evidence are in the ignored
`workspace/provider-qa/` directory.

Protocols: [Ollama compatibility](https://docs.ollama.com/api/openai-compatibility),
[Ollama model discovery](https://docs.ollama.com/api/tags),
[OpenCode server](https://opencode.ai/docs/server/), and
[Codex non-interactive mode](https://developers.openai.com/codex/noninteractive).

Live verification on 2026-09-09: the installed Ollama model, OpenCode
`opencode/big-pickle`, and the signed-in Codex CLI each completed an actual
file objective and independent read-back. Evidence JSON and a disk-verified
report are in `workspace/provider-qa/`. The initial Ollama attempt prefixed
an absolute workspace path and was correctly refused; clearer relative-path
instructions produced a verified completion on the subsequent run.
Eight Studio tests and eleven provider tests passed; Clippy, formatting, and
JavaScript syntax checks passed. The temporary verified Studio instance uses
port 4318; the default launch command continues to use port 4317.
