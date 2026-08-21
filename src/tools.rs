use rusqlite::Connection;

use crate::db;

/// Welcome subagent: greet the user by name and give the emergency (911) notice.
pub fn welcome(name: &str) -> String {
    format!(
        "Hello, {name}! I'm your appointment scheduler assistant. \
         If you are experiencing a medical emergency, please call 911 immediately. \
         How can I help you today?"
    )
}

/// Show the user's current (most recent) appointment.
pub fn show_current_appointment(conn: &Connection, name: &str) -> String {
    match db::get_current_appointment(conn, name) {
        Some(a) => format!(
            "{name}, your current appointment is on {} for: {} (booked on {}).",
            a.appointment_time, a.reason, a.created_at
        ),
        None => format!(
            "{name}, you don't have any appointments scheduled yet. Would you like to schedule one?"
        ),
    }
}

/// Schedule a new appointment and return a confirmation message.
pub fn schedule(conn: &Connection, name: &str, time: &str, reason: &str) -> String {
    let reason = if reason.is_empty() {
        "General appointment"
    } else {
        reason
    };
    match db::schedule_appointment(conn, name, time, reason) {
        Ok(a) => format!(
            "Confirmation: {name}, your appointment has been scheduled for {} (reason: {}). Reference id: {}.",
            a.appointment_time, a.reason, a.id
        ),
        Err(e) => format!("Sorry {name}, I couldn't schedule that appointment: {e}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        db::init_db(":memory:").unwrap()
    }

    #[test]
    fn welcome_includes_name_and_911() {
        let s = welcome("Alice");
        assert!(s.contains("Alice"), "greets the user by name");
        assert!(s.contains("911"), "mentions 911 for emergencies");
    }

    #[test]
    fn show_current_when_present() {
        let conn = mem();
        db::schedule_appointment(&conn, "Alice", "2026-05-01 10:00", "Eye exam").unwrap();
        let s = show_current_appointment(&conn, "Alice");
        assert!(s.contains("Eye exam"), "{}", s);
    }

    #[test]
    fn show_current_when_absent() {
        let conn = mem();
        let s = show_current_appointment(&conn, "Ghost");
        assert!(s.contains("don't have any appointments"), "{}", s);
    }

    #[test]
    fn schedule_returns_confirmation() {
        let conn = mem();
        let s = schedule(&conn, "Bob", "2026-06-02 11:00", "Follow-up");
        assert!(s.contains("Confirmation"), "{}", s);
        assert!(s.contains("Bob"), "{}", s);
        assert!(s.contains("Follow-up"), "{}", s);
    }

    #[test]
    fn schedule_empty_reason_defaults() {
        let conn = mem();
        let s = schedule(&conn, "Bob", "2026-06-02 11:00", "");
        assert!(s.contains("General appointment"), "{}", s);
    }
}
