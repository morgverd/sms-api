use std::time::Duration;
use anyhow::{anyhow, Result};
use log::{debug, warn};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use crate::sms::encryption::SMSEncryption;
use crate::sms::types::{SMSMessage, SMSStatus};

const SCHEMA_SQL: &str = include_str!("../schema.sql");

pub struct SMSDatabase {
    pool: SqlitePool,
    encryption: SMSEncryption
}
impl SMSDatabase {
    pub async fn connect(database_url: &str, encryption_key: [u8; 32]) -> Result<Self> {
        let connection_options = SqliteConnectOptions::new()
            .filename(database_url)
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
            encryption: SMSEncryption::new(encryption_key)
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
    
    pub async fn insert_message(&self, message: SMSMessage) -> Result<i64> {
        warn!("INSERT SMSMessage: {:?}", message);
        let encrypted_content = self.encryption.encrypt(&*message.message_content)?;
        let result = sqlx::query(
            "INSERT INTO messages (phone_number, message_content, message_reference, is_outgoing, status) VALUES (?, ?, ?, ?, ?)"
        )
            .bind(message.phone_number)
            .bind(encrypted_content)
            .bind(message.message_reference)
            .bind(message.is_outgoing)
            .bind(u8::from(message.status))
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;
        
        Ok(result.last_insert_rowid())
    }
    
    pub async fn insert_send_failure(&self, message_id: i64, error_message: String) -> Result<i64> {
        warn!("INSERT send_failure. Message ID: {:?}, Error: {:?}", message_id, error_message);
        let result = sqlx::query(
            "INSERT INTO send_failures (id, error_message) VALUES (?, ?)"
        )
            .bind(message_id)
            .bind(error_message)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn get_latest_numbers(&self, limit: i64, offset: i64) -> Result<Vec<String>> {
        let result: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT phone_number FROM messages GROUP BY phone_number ORDER BY MAX(created_at) DESC LIMIT ? OFFSET ?"
        )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(result.into_iter().flatten().collect())
    }

    pub async fn get_messages(
        &self,
        phone_number: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SMSMessage>> {
        let result = sqlx::query(
            "SELECT id, phone_number, message_content, message_reference, is_outgoing, status, created_at, updated_at FROM messages WHERE phone_number = ? ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )
            .bind(phone_number)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow!(e))?;

        result.into_iter()
            .map(|row| -> Result<SMSMessage> {
                Ok(SMSMessage {
                    id: row.get("id"),
                    phone_number: row.get("phone_number"),
                    message_content: self.encryption.decrypt(&row.get::<String, _>("message_content"))?,
                    message_reference: row.get("message_reference"),
                    is_outgoing: row.get("is_outgoing"),
                    status: SMSStatus::try_from(row.get::<u8, _>("status"))?,
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }
}