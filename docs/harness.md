# Harness development and acceptance

Feature requirements, acceptance criteria and their verification commands are
managed in [`requirements/harness.sdoc`](requirements/harness.sdoc); each
requirement has a `HARNESS-*` UID and links the files that verify it. Design
rules live in [`requirements/architecture.sdoc`](requirements/architecture.sdoc).
`mise run docs` fails when a linked file is missing and exports the HTML to
`target/strictdoc`. Write new or changed requirements there first.

## Commands

Use the Node.js pinned in `mise.toml` and the Rust toolchain pinned in
`rust-toolchain.toml`. The developer harness and its tests are Rust workspace targets.
mise tasks (`mise run check`, `mise run test -- --package <name>`, …) are the
common entry point and wrap the commands below; see `docs/build-system.md`.

The harness package lives in `crates/harness`. The `cargo harness` alias in
`.cargo/config.toml` selects the `gpui-agent-harness` package and binary by name.

Run from this checkout (or use `cargo run --manifest-path <checkout>/crates/harness/Cargo.toml --locked --bin gpui-agent-harness -- <command>` from another directory):

```text
cargo harness doctor
cargo harness setup
cargo harness dev
cargo harness dev --surface desktop
cargo harness client-fixture
cargo harness test --package gpui-agent-application --test architecture
cargo harness test --package gpui-agent-api --test jobs
cargo harness test --package gpui-agent-api --test project_startup
cargo harness test --package gpui-agent-api --test devices
cargo harness test --package gpui-agent-api --test pull_requests
cargo harness test --package gpui-agent-infrastructure --test pull_requests
cargo harness test --package gpui-agent-api --test project_tools --filter environment
cargo harness check
cargo harness check --include-desktop
cargo harness audit
```

On Windows, desktop `check` also runs
`cargo test --locked --package accesskit_windows --lib` before workspace tests.
This exercises actual hidden HWND/COM providers across window destruction,
including element bounds, hit testing, and text geometry. It needs no account,
screen reader, or pointer input. The pinned adapter patch and its removal
condition are documented in `vendor/accesskit_windows/GPUI-AGENT-PATCH.md`.

`setup` fetches locked dependencies. `dev` retains state in `.dev/home` under the
checkout; use a separate checkout for another environment. It does not copy real
profiles, consume pairing links, or launch a login browser. The server binds to
loopback and prints its chosen address. Authenticate explicitly when needed.
`check` never launches the desktop or formats files in place. The older
`scripts/dev.*` and `scripts/run.*` use checkout-local `.gpui-agent-dev` by
default, keeping development data separate from the installed application's
`~/.gpui-agent`. The run scripts still honor an explicit `--home`. Release
scripts keep their existing behavior.

The desktop opens the saved project catalog without registering its startup
directory. An empty home therefore shows the project opener until the user
chooses a folder; provider and account settings remain available. Restarting
restores registered projects. The headless server retains its working-directory
workspace for command-line and automation use.

`crates/harness/src/plan.rs` owns deterministic argument planning and runner
validation; `src/main.rs` owns manifest reads, tool discovery and process I/O.
The Rust integration targets `cli`, `scripts`, `mise`, `layers` and
`client_fixture` run as part of workspace tests. The script and mise tests replace
Cargo with a Rust executable in temporary checkouts; they do not install tools
or build release packages. `layers` checks workspace dependencies against
`tools/layers.toml`. The client fixture test starts the real HTTP host,
cancels a native fixture job at its approval boundary, then checks shutdown and
temporary-home removal.

`--filter` is a libtest name filter. Custom process runners (`protocols`,
`recordings`, `http`, `jobs`, `devices`, `pull_requests`, and `workspaces`) reject it instead of silently
running all scenarios. This also applies to package-wide filters when a custom
runner would be selected. Choose a libtest target explicitly, or use a documented
custom scenario, for example:

```text
cargo harness test --package gpui-agent-application --test architecture --filter production_dependencies
cargo test --locked -p gpui-agent-api --test jobs -- --mcp-only
cargo test --release --locked -p gpui-agent-api --test jobs -- --long-stream-only
cargo test --locked -p gpui-agent --test protocols -- --loop-only
cargo test --locked -p gpui-agent-api --test jobs -- --conversations-only
cargo test --locked -p gpui-agent-api --test jobs -- --context-transfers-only
cargo test --locked -p gpui-agent-api --test jobs -- --questions-only
cargo test --locked -p gpui-agent-api --test jobs -- --native-tools-only
cargo test --locked -p gpui-agent-api --test http -- --codex-running-only
cargo harness test --package gpui-agent-presentation --test tool_activity
cargo test --locked -p gpui-agent-desktop given_recorded_tool_activity
cargo test --locked -p gpui-agent-api --test jobs -- --codex-policy-only
cargo test --locked -p gpui-agent-domain --test codex_policy
cargo test --locked -p gpui-agent-desktop given_codex_
cargo test --locked -p gpui-agent-cli policy_tests
cargo test --locked -p gpui-agent-api --test jobs -- --remote-mcp-only
cargo harness test --package gpui-agent-infrastructure --test mcp
cargo test --locked -p gpui-agent --test oauth
cargo test --locked -p gpui-agent --test protocols -- --oauth-mcp
cargo test --locked -p gpui-agent-api --test jobs -- --oauth-mcp-only
cargo test --locked -p gpui-agent-desktop given_saved_oauth_servers
cargo harness test --package gpui-agent-infrastructure --test queue
cargo harness test --package gpui-agent-api --test control
cargo test --locked -p gpui-agent-control --test browser_automation -- --ignored
cargo test --locked -p gpui-agent --test recordings
```

`client-fixture` requires an interactive terminal. It serves the production web
assets, authenticated HTTP API, application services and SQLite repositories
with synthetic native provider processes. Its home is a temporary directory,
all three fixture profiles are signed in, and real browser/desktop controls are
disabled. Read its URL and fixture token path, exercise the client, then press
Enter to stop the owned host and remove its temporary state. EOF also stops it.

## Review and evidence

Native progress fixtures also cover MCP progress messages, full patch replacements
(including empty snapshots), and terminal input with process IDs, empty text and
newlines. They reject foreign-thread/turn and malformed notifications before and
after the turn/start response. The jobs `--native-tools-only` scenario compares
recorded conversation activity with journal replay. Presentation and web tests
preserve activity through final output; GPUI tests open the persisted tool details.
The http `--codex-running-only` scenario cancels after observing tool progress while the
turn/start reply is still pending. A turn/started notification enables scoped
early progress; notifications received before it wait for the returned turn ID.

Review changed behavior against its scenario, package dependency direction,
input validation, authorization, cancellation, stored history compatibility, and
applicable desktop/web/CLI/provider callers. A diagnostic suppression needs a
nearby reason and human review. A successful fixture test does not demonstrate
live account authentication or graphical rendering.

Retain an exact command and result for each gate. For user-visible changes retain
screenshots or a short recording outside the source tree. Reproduce a bug at a
public boundary, fix the shared cause, and turn repeatable mistakes into a test
or existing static rule before adding more instructions.
