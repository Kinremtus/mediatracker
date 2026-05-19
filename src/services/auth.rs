use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::user::{CreateUser, User};
use crate::models::session::Session;

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
        let existing = sqlx::query_as::<_, User>(
            "SELECT * FROM users WHERE username = $1 OR email = $2",
        )
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
        let token_hash = sha256(&token);
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
        let token_hash = sha256(token);
        let session = sqlx::query_as::<_, Session>(
            "SELECT * FROM sessions WHERE token_hash = $1 AND expires_at > NOW()",
        )
        .bind(token_hash)
        .fetch_optional(&self.db)
        .await?;

        match session {
            Some(s) => Ok(s),
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
        let token_hash = sha256(token);
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

fn sha256(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
