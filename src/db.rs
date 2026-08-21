use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appointment {
    pub id: i64,
    pub user_name: String,
    pub appointment_time: String,
    pub reason: String,
    pub created_at: String,
}

/// Open (or create) the SQLite database and initialize the schema + seed data.
pub fn init_db(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS appointments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_name TEXT NOT NULL,
            appointment_time TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    // Seed an example appointment for the demo user so "current appointment" works immediately.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM appointments WHERE user_name = ?1",
            params!["admin"],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        schedule_appointment(&conn, "admin", "2026-09-01 10:00", "Annual checkup")?;
    }
    Ok(conn)
}

/// Look up the user's most recent scheduled appointment.
pub fn get_current_appointment(conn: &Connection, user_name: &str) -> Option<Appointment> {
    let mut stmt = conn
        .prepare(
            "SELECT id, user_name, appointment_time, reason, created_at
             FROM appointments
             WHERE user_name = ?1
             ORDER BY appointment_time DESC
             LIMIT 1",
        )
        .ok()?;
    stmt.query_row(params![user_name], |r| {
        Ok(Appointment {
            id: r.get(0)?,
            user_name: r.get(1)?,
            appointment_time: r.get(2)?,
            reason: r.get(3)?,
            created_at: r.get(4)?,
        })
    })
    .ok()
}

/// Insert a new appointment and return the stored record.
pub fn schedule_appointment(
    conn: &Connection,
    user_name: &str,
    appointment_time: &str,
    reason: &str,
) -> rusqlite::Result<Appointment> {
    let created_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO appointments (user_name, appointment_time, reason, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![user_name, appointment_time, reason, created_at],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Appointment {
        id,
        user_name: user_name.to_string(),
        appointment_time: appointment_time.to_string(),
        reason: reason.to_string(),
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        init_db(":memory:").unwrap()
    }

    #[test]
    fn seeds_admin_appointment() {
        let conn = mem();
        let apt = get_current_appointment(&conn, "admin");
        assert!(apt.is_some(), "seeded admin appointment should exist");
        assert_eq!(apt.unwrap().reason, "Annual checkup");
    }

    #[test]
    fn schedule_then_retrieve() {
        let conn = mem();
        let a = schedule_appointment(&conn, "alice", "2026-10-02 09:00", "Dental").unwrap();
        assert!(a.id > 0);
        let got = get_current_appointment(&conn, "alice").unwrap();
        assert_eq!(got.reason, "Dental");
        assert_eq!(got.user_name, "alice");
    }

    #[test]
    fn unknown_user_has_no_appointment() {
        let conn = mem();
        assert!(get_current_appointment(&conn, "nobody").is_none());
    }

    #[test]
    fn latest_appointment_returned() {
        let conn = mem();
        schedule_appointment(&conn, "bob", "2026-03-01 08:00", "Old").unwrap();
        schedule_appointment(&conn, "bob", "2026-12-01 14:00", "New").unwrap();
        let got = get_current_appointment(&conn, "bob").unwrap();
        assert_eq!(got.reason, "New", "should return the most recent appointment");
    }
}
