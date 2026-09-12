# App connections and desktop recovery

Selecting an installed app calls `/api/apps/select`. The host validates its Start-menu ID, researches the public MCP registry, and checks a curated adapter when one exists. Registry failure does not prevent desktop fallback. Registry descriptions are discovery data, never executable installation instructions.

The initial executable catalog supports Google Chrome and Microsoft Edge through Microsoft's `@playwright/mcp` **0.0.80**. Setup installs that exact package under the runtime's `conversations/mcp-packages`, with npm lifecycle scripts disabled and a 90-second deadline. Node/npm must already be installed. The npm lockfile records dependency integrity. Existing packages with a different version are rejected rather than silently updated.

Verification performs MCP initialization, protocol negotiation, paginated tool discovery, and a real browser tab-list operation. Each later call must name a discovered tool. The browser uses a separate persistent Klyne profile, in headless mode; this does not attach to existing signed-in tabs or grant full control of browser settings. Authentication may still need user involvement. Reviewers can inspect tool definitions but cannot invoke arbitrary MCP operations.

For other apps, or failed adapter checks, the picker opens the selected app and enables desktop controls. A discovered third-party server is not automatically trusted or installed. Additional app adapters need source review, pinned configuration, and a meaningful probe before joining the executable catalog. General remote HTTP MCP, OAuth provisioning, and arbitrary package-to-app matching are not implemented.

MCP calls preserve their session. The host rejects unsupported client requests, caps message size and pagination, imposes a 45-second request deadline, and uses a Windows job to own the server's descendant processes. An interrupted or disconnected tool call is recorded as uncertain and is not replayed through desktop fallback. Setup failures before dispatch can fall back safely.

Desktop recovery now:

- Rejects clicks and scrolling when the target window moved or resized since observation.
- Refreshes stale targets and focus observations before the model chooses a new action.
- Includes window ownership, enabled state, and supported accessibility patterns in observations, helping the model identify blocking dialogs.
- Supports SelectionItem and ExpandCollapse controls as well as Invoke.
- Records whether any input began. A failure before input returns a fresh observation and lets the model adapt; failures after input remain uncertain.
- Stops after three unsuccessful recovery attempts. Observation-only calls do not reset that count.

Popup decisions remain contextual model decisions, not blind Escape/Enter loops. Security, authentication, unsaved changes, and destructive confirmations are not automatically dismissed. These mechanisms reduce interruptions; they do not make inaccessible apps or protected windows universally controllable.

## Checks

`cargo test --offline -p klyne-studio --bin klyne-studio` covers grants, stale targets, and recovery bounds. The ignored `mcp::tests::live_browser_roundtrip` test uses `KLYNE_MCP_TEST_ROOT` pointing to the runtime conversations folder and verifies a disposable HTTP page through a real browser MCP session. `scripts/test_desktop_recovery.ps1` uses an owned temporary test window to check stale geometry and accessibility selection; it does not operate user documents.

## Research

- [Official MCP Registry API](https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/api/official-registry-api.md): discovery endpoint and filters.
- [Registry trust model](https://modelcontextprotocol.io/registry/about): namespaces authenticate publisher identity; registry listing is not code security verification.
- [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle) and [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports): initialization and JSON-RPC framing.
- [Microsoft Playwright MCP](https://github.com/microsoft/playwright-mcp): browser profiles, installation, and tool interfaces.
- [Windows UI Automation patterns](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controlpatternsoverview): Invoke, SelectionItem, and ExpandCollapse semantics.
