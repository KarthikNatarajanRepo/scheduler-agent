# Appointment Scheduler Agent

An **agentic appointment-scheduling application** with a chatbot web interface, built in **Rust**.

The chatbot understands natural-language requests and routes them to subagents/tools:

- **Welcome** — greets the user by name and reminds them to call **911** in an emergency.
- **Current appointment** — looks up the user's most recent appointment.
- **Schedule appointment** — books a new appointment and returns a confirmation.

Data is stored in **SQLite**. The LLM router is pluggable: **Claude Sonnet 5** (Anthropic API), a **local Ollama** model (e.g. `qwen3:8b`), or any **OpenAI-compatible** endpoint. Each agent/tool ships with unit tests.

---

## Architecture

```
 Browser (chat UI)  ──HTTP──►  axum web server (main.rs)
                                  │
                                  ▼
                         agents.rs  (router / orchestrator)
                          ├─ Sonnet 5 LLM router (llm.rs)   ◄── tool-use loop
                          └─ offline keyword router (fallback, no API key)
                                  │
                                  ▼
                         tools.rs  (welcome / current / schedule)
                                  │
                                  ▼
                         db.rs     (SQLite via rusqlite)
```

- `src/main.rs` — axum web server, routes (`/`, `/api/chat`, `/health`).
- `src/agents.rs` — request routing + the Sonnet 5 tool-use loop, with an offline fallback.
- `src/llm.rs` — LLM client with a normalized tool-use interface, supporting the Anthropic Messages API (Sonnet 5) and any OpenAI-compatible endpoint (Ollama / OpenAI).
- `src/tools.rs` — the three subagents/tools (welcome, current appointment, schedule).
- `src/db.rs` — SQLite schema, queries, and seed data.
- `static/index.html` — the chatbot web interface.

## Prerequisites

- **Rust** toolchain (`cargo`, `rustc`) — built with 1.95.
- A C compiler (for the bundled SQLite; Xcode Command Line Tools on macOS).
- **Optional:** an Anthropic API key for live Sonnet 5 routing. Without it the app runs in offline mode using a keyword router (still fully functional for manual testing).

## Configuration

All configuration is via environment variables (see `.env.example`). The LLM provider is selected with `LLM_PROVIDER`:

| `LLM_PROVIDER` | What it uses                          | Key required? |
| -------------- | ------------------------------------- | ------------- |
| `ollama`       | A local Ollama model (OpenAI-compatible) | No          |
| `anthropic`    | Claude Sonnet 5 via the Anthropic API  | Yes           |
| `openai`       | Any OpenAI-compatible endpoint         | Yes           |

If `LLM_PROVIDER` is unset or the selected provider's key is missing, the app falls back to an **offline keyword router** so it always runs.

| Variable             | Default                       | Description                                                          |
| -------------------- | ----------------------------- | ------------------------------------------------------------------- |
| `LLM_PROVIDER`       | `anthropic`                   | `ollama` \| `anthropic` \| `openai`                                  |
| `OLLAMA_BASE_URL`    | `http://localhost:11434/v1`   | Ollama OpenAI-compatible endpoint (provider = ollama)              |
| `OLLAMA_MODEL`       | `qwen3:8b`                    | Ollama model id (must support tool/function calling)                |
| `ANTHROPIC_API_KEY`  | _(unset)_                     | Anthropic API key (provider = anthropic)                            |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com`   | Override the Anthropic API base URL (e.g. a proxy)                  |
| `SCHEDULER_MODEL`    | `claude-sonnet-5`             | Anthropic model id (provider = anthropic)                           |
| `OPENAI_BASE_URL`    | `https://api.openai.com/v1`   | OpenAI-compatible endpoint (provider = openai)                     |
| `OPENAI_API_KEY`     | _(unset)_                     | API key (provider = openai)                                         |
| `OPENAI_MODEL`       | `gpt-4o-mini`                 | Model id (provider = openai)                                        |
| `PORT`              | `8080`                        | HTTP port to listen on                                              |
| `SCHEDULER_DB`       | `scheduler.db`                | SQLite database file path. Use `:memory:` for an ephemeral in-memory DB. |

## Build

```bash
cd scheduler-agent
cargo build --release
```

The release binary is placed at `target/release/scheduler-agent`.

## Run

```bash
# Local Ollama (no API key needed) — recommended for testing
LLM_PROVIDER=ollama cargo run --release

# Live Sonnet 5 routing
export ANTHROPIC_API_KEY="sk-ant-..."
cargo run --release

# Offline mode (no LLM) — keyword router
cargo run --release
```

Then open <http://localhost:8080> in your browser. Use `PORT=8081` if 8080 is taken.

> Ollama's first call to a model can take 10–60s while the model loads; subsequent calls are faster. The app sets a 120s per-request timeout.

## Test

Unit tests cover the database layer, each tool, and the router:

```bash
cargo test
```

## API reference

### `GET /`
Returns the chatbot web interface (`index.html`).

### `GET /health`
Returns `ok` — used for liveness checks.

### `POST /api/chat`
Sends a message to the scheduler agent.

**Request body**
```json
{ "user_name": "admin", "message": "what is my current appointment?" }
```

**Response body**
```json
{ "reply": "admin, your current appointment is on 2026-09-01 10:00 for: Annual checkup ..." }
```

**Examples**
```bash
# Health check
curl http://localhost:8080/health

# Ask for current appointment
curl -s localhost:8080/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"user_name":"admin","message":"what is my current appointment?"}' | jq

# Schedule an appointment
curl -s localhost:8080/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"user_name":"admin","message":"please schedule a dental appointment on 2026-09-15 14:00"}' | jq
```

## Packaging

To build a self-contained release binary:

```bash
cargo build --release
# Distribute: target/release/scheduler-agent  (plus static/index.html is embedded in the binary)
```

The web UI is embedded into the binary via `include_str!`, so the single `scheduler-agent`
executable plus a writable `scheduler.db` location is all you need to run it. To ship without a
DB file, set `SCHEDULER_DB=:memory:` or let it create one on first run.

## Manual testing checklist

1. `cargo run --release` and open <http://localhost:8080>.
2. Enter your name and send **"hi"** → expect a greeting that includes your name and a 911 notice.
3. Send **"what is my current appointment?"** → the seeded `admin` appointment is returned.
4. Send **"schedule a dental appointment on 2026-10-10 09:00"** → expect a confirmation with an id.
5. Send the current-appointment query again → expect the newly booked appointment.
6. Send **"this is an emergency"** → expect a 911 instruction.

## Troubleshooting

- **"offline keyword router"** at startup → no `ANTHROPIC_API_KEY` is set; set one for live Sonnet 5 routing.
- **Anthropic API errors in replies** → check the key, the model id (`SCHEDULER_MODEL`), and `ANTHROPIC_BASE_URL`. On API errors the router falls back to the keyword router so the app stays usable.
- **Database locked** → only one process should write to a file-based `scheduler.db`. Use `:memory:` for ephemeral runs.

## License

MIT
