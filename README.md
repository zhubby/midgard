# Midgard

Midgard is an agent-native middleware operations platform for Docker and Kubernetes workspaces. It uses an LLM agent to reason over operational goals, invoke registered tools, and coordinate middleware controllers that manage workloads in the current workspace runtime.

## Architecture

- Rust backend organized as root-level `midgard-*` crates.
- OpenAI-compatible ReAct agent runtime with explicit completion signaling.
- TOML-backed project configuration loaded from `~/.midgard/config.toml`.
- Toasty-backed storage layer targeting PostgreSQL.
- Pluggable middleware controllers that register capabilities as tools.
- Persistent approval audit history for high-risk and critical tool calls.
- Operator abstraction layer for Midgard-native Kubernetes operators.
- Docker runtime tools that are only visible to Docker-configured workspaces.
- Bun + TypeScript + Next.js frontend in `midgard-web`.

## Workspace

```text
midgard-core                Shared domain types and configuration
midgard-config              TOML config loading and default file creation
midgard-agent               Agent loop and OpenAI-compatible provider
midgard-storage             Toasty/PostgreSQL storage for agent sessions
midgard-tools               Tool trait, registry, and execution results
midgard-controller          Middleware controller plugin contracts
midgard-operator            Kubernetes operator contracts and shared helpers
midgard-docker              Docker runtime tools and plugin
midgard-protocol            gRPC contracts for server/operator control
midgard-server              Axum HTTP API library
midgard-cli                 Clap CLI entrypoint and migration wrapper
operators/midgard-valkey-operator
                            Midgard-native Valkey Kubernetes operator
midgard-web/                 Bun + Next.js UI
midgard-storage/Toasty.toml Toasty migration config
midgard-storage/toasty/     Toasty migration history, migrations, and snapshots
```

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo run -p midgard-cli -- server
cargo run -p midgard-cli -- operator valkey --workspace-id <uuid> --registration-token <token>
cargo run -p midgard-cli -- migrate apply
```

Frontend:

```bash
cd midgard-web
bun install
bun run dev
```

## Configuration

Midgard reads configuration from `~/.midgard/config.toml` by default. The CLI creates this file with editable defaults when it does not exist:

```toml
[server]
bind_address = "0.0.0.0:8080"

[database]
url = ""

[secrets]
workspace_credentials_key = "generated-random-key"

[llm]
base_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
api_mode = "chat_completions"
api_key = ""
```

Fill `database.url` before starting the server or running migrations. The CLI generates `secrets.workspace_credentials_key` for encrypting workspace runtime credentials. The LLM provider target is OpenAI-compatible. Set `base_url` to the complete request endpoint; Midgard does not append `/v1`, `/chat/completions`, `/responses`, or any other path. Use `api_mode = "chat_completions"` by default, or `api_mode = "responses"` with a matching complete Responses API endpoint.
