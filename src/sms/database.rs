use std::time::Duration;
use anyhow::{anyhow, Result};
use log::debug;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use crate::config::SMSConfig;
use crate::sms::encryption::SMSEncryption;
use crate::sms::types::{SMSDeliveryReport, SMSMessage, SMSStatus};

const SCHEMA_SQL: &str = include_str!("../schema.sql");

pub struct SMSDatabase {
    pool: SqlitePool,
    encryption: SMSEncryption
}
impl SMSDatabase {
    pub async fn connect(config: &SMSConfig) -> Result<Self> {
        let connection_options = SqliteConnectOptions::new()
            .filename(config.database_url.clone())
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(30));

        let pool = SqlitePoolOptions::new()
            .max_connections(20)
            .min_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(None)
            .max_lifetime(None)
            .test_before_acquire(true)
            .after_connect(|conn, _meta| {
                Box::pin(async move {

                    // Optimise connection.
                    sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
                    sqlx::query("PRAGMA cache_size = -64000").execute(&mut *conn).await?; // 64MB Cache
                    sqlx::query("PRAGMA temp_store = memory").execute(&mut *conn).await?;
                    Ok(())
                })
            })
            .connect_with(connection_options)
            .await
            .map_err(|e| anyhow!(e))?;

        let db = Self {
            pool,
            encryption: SMSEncryption::new(config.encryption_key)
        };
        db.init_tables().await?;
        Ok(db)
    }

    async fn init_tables(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;
        
        debug!("SMSDatabase tables initialized successfully!");
        Ok(())
    }
    
    pub async fn insert_message(&self, message: &SMSMessage, is_final: bool) -> Result<i64> {
        let encrypted_content = self.encryption.encrypt(&*message.message_content)?;
        let result = if is_final {
            sqlx::query(
                "INSERT INTO messages (phone_number, message_content, message_reference, is_outgoing, status, completed_at) VALUES (?, ?, ?, ?, ?, unixepoch())"
            )
        } else {
            sqlx::query(
                "INSERT INTO messages (phone_number, message_content, message_reference, is_outgoing, status) VALUES (?, ?, ?, ?, ?)"
            )
        }
            .bind(&message.phone_number)
            .bind(encrypted_content)
            .bind(message.message_reference)
            .bind(message.is_outgoing)
            .bind(u8::from(&message.status))
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;
        
        Ok(result.last_insert_rowid())
    }
    
    pub async fn insert_send_failure(&self, message_id: i64, error_message: String) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO send_failures (message_id, error_message) VALUES (?, ?)"
        )
            .bind(message_id)
            .bind(error_message)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn insert_delivery_report(&self, message_id: i64, status: u8, is_final: bool) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO delivery_reports (message_id, status, is_final) VALUES (?, ?, ?)"
        )
            .bind(message_id)
            .bind(status)
            .bind(is_final)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_delivery_report_target_message(&self, phone_number: String, reference_id: u8) -> Result<Option<i64>> {
        let result = sqlx::query_scalar(
            "SELECT message_id FROM messages WHERE completed_at IS NULL AND is_outgoing = 1 AND phone_number = ? AND message_reference = ? ORDER BY message_id DESC LIMIT 1"
        )
            .bind(phone_number)
            .bind(reference_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(result)
    }

    pub async fn update_message_status(&self, message_id: i64, status: &SMSStatus, completed: bool) -> Result<()> {
        let query = if completed {
            sqlx::query(
                "UPDATE messages SET status = ?, completed_at = unixepoch() WHERE message_id = ?"
            )
        } else {
            sqlx::query(
                "UPDATE messages SET status = ? WHERE message_id = ?"
            )
        };

        query
            .bind(u8::from(status))
            .bind(message_id)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    pub async fn get_latest_numbers(&self, limit: u64, offset: u64) -> Result<Vec<String>> {
        let result: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT phone_number FROM messages GROUP BY phone_number ORDER BY MAX(created_at) DESC LIMIT ? OFFSET ?"
        )
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(result.into_iter().flatten().collect())
    }

    pub async fn get_messages(
        &self,
        phone_number: &str,
        limit: u64,
        offset: u64
    ) -> Result<Vec<SMSMessage>> {
        let result = sqlx::query(
            "SELECT message_id, phone_number, message_content, message_reference, is_outgoing, status, created_at, completed_at FROM messages WHERE phone_number = ? ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )
            .bind(phone_number)
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        result.into_iter()
            .map(|row| -> Result<SMSMessage> {
                Ok(SMSMessage {
                    message_id: row.get("message_id"),
                    phone_number: row.get("phone_number"),
                    message_content: self.encryption.decrypt(&row.get::<String, _>("message_content"))?,
                    message_reference: row.get("message_reference"),
                    is_outgoing: row.get("is_outgoing"),
                    status: SMSStatus::try_from(row.get::<u8, _>("status"))?,
                    created_at: row.get("created_at"),
                    completed_at: row.get("completed_at")
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn get_delivery_reports(
        &self,
        message_id: i64,
        limit: u64,
        offset: u64
    ) -> Result<Vec<SMSDeliveryReport>> {
        sqlx::query_as(
            "SELECT report_id, message_id, status, is_final, created_at FROM delivery_reports WHERE message_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )
            .bind(message_id)
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow!(e))
    }
}