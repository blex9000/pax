use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::Database;

impl Database {
    pub fn set_app_preference(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_preferences (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn get_app_preference(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM app_preferences WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;

    #[test]
    fn app_preference_roundtrip_works() {
        let db = Database::open_memory().unwrap();

        assert_eq!(db.get_app_preference("theme").unwrap(), None);

        db.set_app_preference("theme", "dracula").unwrap();
        assert_eq!(
            db.get_app_preference("theme").unwrap().as_deref(),
            Some("dracula")
        );
    }

    #[test]
    fn app_preference_updates_existing_value() {
        let db = Database::open_memory().unwrap();

        db.set_app_preference("theme", "nord").unwrap();
        db.set_app_preference("theme", "catppuccin-mocha").unwrap();

        assert_eq!(
            db.get_app_preference("theme").unwrap().as_deref(),
            Some("catppuccin-mocha")
        );
    }
}
