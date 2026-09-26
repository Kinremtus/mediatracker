use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::session::Session;
use crate::models::user::{CreateUser, User};

/// Hash a raw session token for storage and lookup (hex SHA-256).
///
/// Single source of truth: session creation, lookup and logout all call this
/// so a stored `token_hash` can never diverge from the lookup hash. The raw
/// token only ever lives in the client cookie.
pub fn hash_token(raw: &str) -> String {
    crate::utils::sha256_hex(raw)
}

#[derive(Clone)]
pub struct AuthService {
    db: PgPool,
}

impl AuthService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn register(&self, data: &CreateUser) -> Result<User, anyhow::Error> {
        // Check if user exists
        let existing =
            sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1 OR email = $2")
                .bind(&data.username)
                .bind(&data.email)
                .fetch_optional(&self.db)
                .await?;

        if existing.is_some() {
            return Err(anyhow::anyhow!("Username or email already exists"));
        }

        // Hash password
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(data.password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?
            .to_string();

        // Insert user
        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (username, email, password_hash) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(&data.username)
        .bind(&data.email)
        .bind(password_hash)
        .fetch_one(&self.db)
        .await?;

        Ok(user)
    }

    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&self.db)
            .await
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> Result<String, anyhow::Error> {
        // Find user
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(&self.db)
            .await?;

        let user = match user {
            Some(u) => u,
            None => return Err(anyhow::anyhow!("Invalid credentials")),
        };

        // Verify password
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|e| anyhow::anyhow!("Failed to parse password hash: {}", e))?;

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| anyhow::anyhow!("Invalid credentials"))?;

        // Create session
        let token = Uuid::new_v4().to_string();
        let token_hash = hash_token(&token);
        let expires_at = Utc::now() + Duration::days(30);

        sqlx::query(
            "INSERT INTO sessions (user_id, token_hash, user_agent, ip, expires_at) VALUES ($1, $2, $3, $4::inet, $5)",
        )
        .bind(user.id)
        .bind(&token_hash)
        .bind(user_agent)
        .bind(ip)
        .bind(expires_at)
        .execute(&self.db)
        .await?;

        Ok(token)
    }

    pub async fn get_session(&self, token: &str) -> Result<Session, anyhow::Error> {
        let token_hash = hash_token(token);
        // Fetch by hash only; expiry is evaluated in-process so an expired
        // row can be deleted on read instead of lingering until a sweep.
        let session = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, token_hash, device_name, user_agent, ip::text as ip, expires_at, created_at, last_seen_at FROM sessions WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.db)
        .await?;

        match session {
            Some(s) if s.expires_at > Utc::now() => Ok(s),
            Some(_) => {
                // Expired session: remove it so it can never authenticate.
                let _ = sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
                    .bind(&token_hash)
                    .execute(&self.db)
                    .await;
                Err(anyhow::anyhow!("Session not found or expired"))
            }
            None => Err(anyhow::anyhow!("Session not found or expired")),
        }
    }

    pub async fn get_user_by_id(&self, user_id: Uuid) -> Result<User, anyhow::Error> {
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.db)
            .await?;

        user.ok_or_else(|| anyhow::anyhow!("User not found"))
    }

    pub async fn logout(&self, token: &str) -> Result<(), anyhow::Error> {
        let token_hash = hash_token(token);
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Delete all of a user's sessions except the one whose hash is
    /// `keep_token_hash`.
    ///
    /// Called after a password change so every other (possibly stolen) cookie
    /// is invalidated while the current, legitimate session keeps working.
    /// Returns the number of sessions removed.
    pub async fn delete_other_sessions(
        &self,
        user_id: Uuid,
        keep_token_hash: &str,
    ) -> Result<u64, anyhow::Error> {
        let result = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2")
            .bind(user_id)
            .bind(keep_token_hash)
            .execute(&self.db)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::hash_token;

    #[test]
    fn hash_token_is_deterministic() {
        let a = hash_token("raw-session-token");
        let b = hash_token("raw-session-token");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_token_is_sha256_hex_and_differs_per_input() {
        let h = hash_token("raw-session-token");
        // SHA-256 hex is always 64 lowercase hex chars.
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, h.to_lowercase());
        assert_ne!(h, hash_token("another-token"));
        // Raw token must never be recoverable from the stored value.
        assert_ne!(h, "raw-session-token");
    }
}
