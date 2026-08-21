# AGENTS.md

Durable instructions for any AI agent (or human) working in this repository.

## Project

`scheduler-agent` is a Rust appointment-scheduler agentic app: an axum chatbot
web interface that routes natural-language requests to subagents/tools via an
LLM, backed by SQLite. See `README.md` for the full overview.

## Build, run, test, lint

Always run these from the repo root.

```bash
cargo build                     # debug build
cargo build --release           # optimized build (target/release/scheduler-agent)

cargo test                       # run the 14 unit tests (db/tools/agents)
cargo clippy --all-targets -- -D warnings   # lint: must be warning-free
cargo fmt --all -- --check       # format check: must be clean (run `cargo fmt` to fix)
```

Run the app locally (Ollama is the default local LLM; needs Ollama running):

```bash
LLM_PROVIDER=ollama cargo run --release      # http://localhost:8080 (set PORT to override)
# Anthropic Sonnet 5 instead:
ANTHROPIC_API_KEY=... cargo run --release
```

Note: port 8080 may be taken by another local service (e.g. a TomEE app) — use `PORT=8081`.

## Configuration (env vars)

| Var | Default | Purpose |
| --- | --- | --- |
| `LLM_PROVIDER` | `anthropic` | `ollama` \| `anthropic` \| `openai` |
| `OLLAMA_BASE_URL` | `http://localhost:11434/v1` | Ollama endpoint |
| `OLLAMA_MODEL` | `qwen3:8b` | Ollama model (must support tool calling) |
| `ANTHROPIC_API_KEY` | unset | Anthropic key (provider=anthropic) |
| `SCHEDULER_MODEL` | `claude-sonnet-5` | Anthropic model id |
| `PORT` | `8080` | HTTP port |
| `SCHEDULER_DB` | `scheduler.db` | SQLite path (`:memory:` for ephemeral) |

Never commit real secrets. `.env` is gitignored; use `.env.example` for the template.

## Architecture

- `src/main.rs` — axum web server, routes `/`, `/api/chat`, `/health`; `AppState` holds `Mutex<Connection>` + `Option<LlmClient>`.
- `src/agents.rs` — request router + the LLM tool-use loop; offline keyword fallback.
- `src/llm.rs` — LLM client: Anthropic Messages API and OpenAI Chat Completions (Ollama); normalized `ToolDef`/`NormMessage`/`ToolCall`.
- `src/tools.rs` — the three subagents/tools: `welcome`, `show_current_appointment`, `schedule`.
- `src/db.rs` — SQLite schema, queries, seed data.
- `static/index.html` — chatbot UI (embedded into the binary).

## Conventions and guardrails

- **Do not hold the `std::sync::Mutex<Connection>` guard across an `.await`.** Acquire it in a scoped block, do synchronous DB work, drop it before any `.await`. See `agents::llm_route` for the pattern.
- All SQL must use parameterized bindings (`params![...]`) — never concatenate user input into queries.
- Propagate errors rather than silently producing wrong-but-happy results (the `.ok()`-swallowing in `db::get_current_appointment` is a known debt).
- New tools must be added in `tools.rs`, declared in `agents::tool_defs`, dispatched in `agents::execute_tool`, and unit-tested.
- Keep `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
- The HTTP `user_name`/`message` fields are untrusted. Known outstanding security debt: no authentication/authorization on `/api/chat` (IDOR), `user_name` is interpolated into the LLM system prompt (prompt injection), and there is no rate limiting. See the baseline review.

## Tests

Tests are inline `#[cfg(test)] mod tests` in each module. Add tests for any new behavior. Run `cargo test` before considering work done.
