use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Mutex;

use crate::llm::{LlmClient, NormMessage, ToolDef};
use crate::tools;

/// System prompt for the orchestrating LLM.
fn system_prompt(name: &str) -> String {
    format!(
        "You are the Appointment Scheduler assistant. The user's name is {name}. \
         Always greet the user by their name. \
         If the user mentions a medical emergency, immediately tell them to call 911. \
         Use the provided tools to look up or create appointments. \
         Keep replies concise and friendly. \
         When you have the information the user asked for, answer in plain text without calling more tools."
    )
}

/// Tool definitions exposed to the LLM (provider-agnostic; translated per provider).
fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "welcome".into(),
            description: "Greet the user by name and remind them to call 911 in emergencies."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
        },
        ToolDef {
            name: "get_current_appointment".into(),
            description: "Look up the user's most recent scheduled appointment.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
        },
        ToolDef {
            name: "schedule_appointment".into(),
            description: "Schedule a new appointment for the user and confirm it.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "appointment_time": { "type": "string", "description": "Datetime like 2026-09-03 10:00" },
                    "reason": { "type": "string" },
                },
                "required": ["name", "appointment_time"],
            }),
        },
    ]
}

/// Execute a single tool call synchronously against the database and return its text result.
fn execute_tool(conn: &Connection, name: &str, input: &Value, user_name: &str) -> String {
    let get_str = |k: &str| {
        input
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or(user_name)
            .to_string()
    };
    match name {
        "welcome" => tools::welcome(&get_str("name")),
        "get_current_appointment" => tools::show_current_appointment(conn, &get_str("name")),
        "schedule_appointment" => {
            let time = input
                .get("appointment_time")
                .and_then(|v| v.as_str())
                .unwrap_or("2026-09-03 10:00");
            let reason = input.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            tools::schedule(conn, &get_str("name"), time, reason)
        }
        _ => format!("Unknown tool: {name}"),
    }
}

/// Main entry: route a user message to the subagents/tools and return the reply text.
///
/// Uses the configured LLM (Anthropic Sonnet 5 or a local Ollama model) as the router when
/// available, otherwise falls back to an offline keyword router so the app still works.
pub async fn route_and_handle(
    db: &Mutex<Connection>,
    llm: Option<&LlmClient>,
    user_name: &str,
    message: &str,
) -> String {
    if let Some(llm) = llm {
        if let Ok(reply) = llm_route(db, llm, user_name, message).await {
            if !reply.is_empty() {
                return reply;
            }
        }
    }
    let conn = match db.lock() {
        Ok(g) => g,
        Err(e) => return format!("DB lock error: {e}"),
    };
    keyword_route(&conn, user_name, message)
}

async fn llm_route(
    db: &Mutex<Connection>,
    llm: &LlmClient,
    user_name: &str,
    message: &str,
) -> Result<String, String> {
    let system = system_prompt(user_name);
    let tools = tool_defs();
    let mut convo: Vec<NormMessage> = vec![NormMessage::User(message.to_string())];

    for _ in 0..5 {
        let turn = llm.chat(&system, &convo, &tools).await?;

        if turn.tool_calls.is_empty() {
            return Ok(turn.text);
        }

        // Record the assistant turn (with its tool calls) for the next round.
        convo.push(NormMessage::Assistant {
            text: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
        });

        // Execute each tool call, holding the DB lock only for this block.
        let mut results: Vec<NormMessage> = Vec::with_capacity(turn.tool_calls.len());
        {
            let conn = db.lock().map_err(|e| format!("db lock: {e}"))?;
            for tc in turn.tool_calls {
                let out = execute_tool(&conn, &tc.name, &tc.arguments, user_name);
                results.push(NormMessage::ToolResult {
                    tool_call_id: tc.id,
                    content: out,
                });
            }
        } // guard dropped here, before the next `.await`

        convo.extend(results);
    }
    Ok("I wasn't able to complete that request.".to_string())
}

/// Offline keyword-based router used when no LLM is configured.
fn keyword_route(conn: &Connection, user_name: &str, message: &str) -> String {
    let m = message.to_lowercase();
    let greeting = tools::welcome(user_name);

    if m.contains("emergency") || m.contains("911") {
        return format!("{greeting}\nIf this is a medical emergency, please call 911 immediately.");
    }
    if m.contains("current") && m.contains("appointment") {
        return tools::show_current_appointment(conn, user_name);
    }
    if m.contains("schedule")
        || m.contains("book")
        || (m.contains("make") && m.contains("appointment"))
    {
        let time = pick_time(&m).unwrap_or_else(|| "2026-09-03 10:00".to_string());
        let reason = pick_reason(&m);
        return tools::schedule(conn, user_name, &time, &reason);
    }
    greeting
}

fn pick_time(m: &str) -> Option<String> {
    let c: Vec<char> = m.chars().collect();
    let n = c.len();
    let is_digits = |range: &[char]| range.iter().all(|x| x.is_ascii_digit());
    let mut i = 0;
    while i + 10 <= n {
        if c[i + 4] == '-'
            && c[i + 7] == '-'
            && is_digits(&c[i..i + 4])
            && is_digits(&c[i + 5..i + 7])
            && is_digits(&c[i + 8..i + 10])
        {
            let date: String = c[i..i + 10].iter().collect();
            if i + 15 < n
                && c[i + 10] == ' '
                && c[i + 13] == ':'
                && is_digits(&c[i + 11..i + 13])
                && is_digits(&c[i + 14..i + 16])
            {
                let t: String = c[i + 11..i + 16].iter().collect();
                return Some(format!("{date} {t}"));
            }
            return Some(date);
        }
        i += 1;
    }
    None
}

fn pick_reason(m: &str) -> String {
    if m.contains("dental") {
        "Dental appointment".into()
    } else if m.contains("checkup") || m.contains("check up") {
        "Checkup".into()
    } else {
        "General appointment".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn keyword_emergency_mentions_911() {
        let conn = db::init_db(":memory:").unwrap();
        let r = keyword_route(&conn, "Alice", "I have an emergency!");
        assert!(r.contains("911"), "{}", r);
    }

    #[test]
    fn keyword_current_returns_appointment() {
        let conn = db::init_db(":memory:").unwrap();
        db::schedule_appointment(&conn, "Alice", "2026-05-01 10:00", "Eye exam").unwrap();
        let r = keyword_route(&conn, "Alice", "what is my current appointment");
        assert!(r.contains("Eye exam"), "{}", r);
    }

    #[test]
    fn keyword_schedule_creates_appointment() {
        let conn = db::init_db(":memory:").unwrap();
        let r = keyword_route(
            &conn,
            "Alice",
            "please schedule a dental appointment on 2026-07-08 09:00",
        );
        assert!(r.contains("Confirmation"), "{}", r);
        let cur = keyword_route(&conn, "Alice", "my current appointment");
        assert!(cur.contains("Dental"), "{}", cur);
    }

    #[test]
    fn keyword_greeting_default() {
        let conn = db::init_db(":memory:").unwrap();
        let r = keyword_route(&conn, "Alice", "hi there");
        assert!(r.contains("Alice"), "{}", r);
    }

    #[test]
    fn pick_time_parses_datetime() {
        assert_eq!(
            pick_time("book on 2026-07-08 09:00 please").as_deref(),
            Some("2026-07-08 09:00")
        );
        assert_eq!(
            pick_time("see you 2026-07-08").as_deref(),
            Some("2026-07-08")
        );
        assert!(pick_time("no date here").is_none());
    }
}
