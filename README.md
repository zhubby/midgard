# Midgard

Midgard is an agent-native middleware operations platform for Kubernetes. It uses an LLM agent to reason over operational goals, invoke registered tools, and coordinate middleware controllers that manage workloads deployed on Kubernetes.

## Architecture

- Rust backend organized as root-level `midgard-*` crates.
- OpenAI-compatible ReAct agent runtime with explicit completion signaling.
- Pluggable middleware controllers that register capabilities as tools.
- Kubernetes abstraction layer for cluster, namespace, workload, pod, and event operations.
- Bun + TypeScript + Next.js frontend in `midgard-ui`.

## Workspace

```text
midgard-core                Shared domain types and configuration
midgard-agent               Agent loop and OpenAI-compatible provider
midgard-tools               Tool trait, registry, and execution results
midgard-controller          Middleware controller plugin contracts
midgard-k8s                 Kubernetes operations abstraction
midgard-plugin-example      Example middleware plugin
midgard-server              Axum HTTP API
midgard-ui/                 Bun + Next.js UI
spec.md                     Project design specification
```

## Development

```bash
cargo check --workspace
cargo test --workspace
```

Frontend:

```bash
cd midgard-ui
bun install
bun run dev
```

## Configuration

The first LLM provider target is OpenAI-compatible:

```bash
MIDGARD_LLM_BASE_URL=https://api.openai.com/v1
MIDGARD_LLM_API_KEY=...
MIDGARD_LLM_MODEL=gpt-4o-mini
```

Use the same shape for compatible gateways such as DeepSeek, Qwen, or an internal model proxy.
