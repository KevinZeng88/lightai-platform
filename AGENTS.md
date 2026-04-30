# LightAI Platform - Agent Instructions

## Product

LightAI Platform is a lightweight private GPU model serving management platform.

Initial target users have a small number of GPU servers, typically 1-5 nodes. The product should be simple to deploy, easy to operate, and suitable for private environments.

The platform should provide:

- GPU server management through a Server + Agent architecture.
- Model lifecycle management: register, delete, start, stop, restart, view status, view logs.
- OpenAI-compatible API gateway.
- API Key management.
- Usage metering by API Key, department, project, and model.
- Basic GPU, node, and model monitoring.
- Simple model priority and API Key priority.
- NVIDIA and domestic GPU support through extensible collectors and runtime templates.

## Development Direction

Start lightweight, but avoid designs that block future expansion.

Future versions may add GPU virtualization, IAM/SSO, richer scheduling, billing, high availability, Kubernetes integration, and enterprise integration. These are not MVP tasks unless explicitly requested.

For the MVP, do not implement:

- Kubernetes integration.
- Complex distributed scheduling.
- GPU virtualization.
- Strong multi-tenant isolation.
- Distributed training scheduling.
- Automatic model sharding.
- High availability clustering.
- Complex financial billing.
- Complex A/B testing or canary release.
- Full enterprise IAM / SSO integration.

## Architecture

Use a lightweight Server + Agent architecture.

- Server runs the web console, management API, API gateway, database access, and usage metering.
- Agent runs on every GPU server.
- Agent registers to Server and reports metrics.
- Server sends model start/stop/restart tasks to Agent.
- Model runtimes are external components such as vLLM, Ollama, LMDeploy, MindIE, llama.cpp, or existing OpenAI-compatible services.
- The platform should not directly depend on specific GPU SDK internals.
- Domestic GPU support should be implemented through GPU collector adapters and runtime templates.
- Keep Server and Agent independently runnable.
- Prefer clear module boundaries over premature microservices.

## Technology Guidance

Do not hardcode long-term technology decisions unless confirmed.

For the MVP, prefer lightweight and easy-to-deploy technologies.

Initial recommended direction:

- Backend and Agent: Rust.
- Database: SQLite for MVP, with a repository layer that can later support PostgreSQL.
- Frontend: Vue or React.
- Config: YAML or TOML.
- Deployment: single binary and/or Docker Compose for Server; systemd or containerized deployment for Agent.

Before changing or expanding the stack, explain why.

## Engineering Rules

For every coding task:

1. Inspect the repository before editing files.
2. Provide a short plan before non-trivial changes.
3. Keep changes small and focused.
4. Do not modify unrelated files.
5. Do not introduce unnecessary abstractions.
6. Prefer simple, explicit code over clever patterns.
7. Keep Server and Agent independently runnable.
8. Add tests for critical auth, gateway, database, and agent logic where practical.
9. Run available build/test/lint commands after implementation.
10. Summarize changed files, verification commands, and known limitations.

## Security Rules

- Never store API keys in plaintext.
- Store only API key hash and key prefix.
- Do not log full API keys.
- Validate and sanitize all runtime template parameters.
- Avoid shell command injection.
- Agent must only accept authorized Server requests.
- Do not expose arbitrary command execution APIs.
- Keep authentication and authorization code isolated so IAM/SSO can be added later.

## Extensibility Rules

Keep these extension points in mind, but do not overbuild them in the MVP:

- GPU model should not assume only NVIDIA.
- GPU collector should support vendor-specific adapters.
- Runtime template system should support different inference engines.
- API Gateway should support multiple models, aliases, and routing policies.
- Auth module should allow future IAM/SSO integration.
- Usage metering should allow future billing rules.
- Node and GPU model should allow future resource partitioning or virtualization.
- Deployment model should not require Kubernetes, but should not prevent future Kubernetes integration.

## Repository Layout

Use this monorepo structure unless explicitly changed:

```text
lightai-platform/
  server/
  agent/
  web/
  migrations/
  deploy/
  docs/
  scripts/
  AGENTS.md
  README.md
