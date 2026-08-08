use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::user::User;
use crate::services::auth::AuthService;
use crate::services::email::EmailService;
use crate::services::notifications::TelegramNotifier;
use crate::utils::sha256_hex;

#[derive(Clone)]
pub struct PasswordResetService {
    db: PgPool,
    auth: AuthService,
    email: EmailService,
    telegram: TelegramNotifier,
    base_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResetError {
    #[error("Недействительная или истёкшая ссылка для восстановления пароля")]
    InvalidToken,
    #[error("Пароль должен быть не менее 6 символов")]
    WeakPassword,
    #[error("Новые пароли не совпадают")]
    PasswordMismatch,
    #[error("Ошибка хеширования пароля")]
    HashError,
    #[error("Ошибка базы данных: {0}")]
    Database(#[from] sqlx::Error),
}

impl PasswordResetService {
    pub fn new(
        db: PgPool,
        auth: AuthService,
        email: EmailService,
        telegram: TelegramNotifier,
        base_url: String,
    ) -> Self {
        Self {
            db,
            auth,
            email,
            telegram,
            base_url,
        }
    }

    pub fn channels_available(&self) -> bool {
        (self.email.is_configured() && !self.base_url.is_empty()) || self.telegram.is_configured()
    }

    pub async fn request_reset(&self, email: &str) -> Result<(), ResetError> {
        let Some(user) = self.auth.find_by_email(email).await? else {
            return Ok(());
        };

        sqlx::query("DELETE FROM password_reset_tokens WHERE user_id = $1")
            .bind(user.id)
            .execute(&self.db)
            .await?;

        let token = Uuid::new_v4().to_string();
        let token_hash = sha256_hex(&token);
        let expires_at = Utc::now() + Duration::hours(1);

        sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(user.id)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(&self.db)
        .await?;

        let reset_url = format!("{}/reset-password?token={}", self.base_url, token);

        self.deliver_reset_link(&user.email, &user.id, &reset_url)
            .await;

        Ok(())
    }

    pub async fn validate_token(&self, token: &str) -> Result<User, ResetError> {
        let token_hash = sha256_hex(token);
        let user = sqlx::query_as::<_, User>(
            "SELECT u.id, u.username, u.email, u.password_hash, u.role, u.created_at, u.updated_at \
             FROM password_reset_tokens t \
             JOIN users u ON u.id = t.user_id \
             WHERE t.token_hash = $1 AND t.expires_at > NOW() AND t.used_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.db)
        .await?;

        user.ok_or(ResetError::InvalidToken)
    }

    pub async fn reset_password(
        &self,
        token: &str,
        new_password: &str,
        confirm: &str,
    ) -> Result<(), ResetError> {
        let user = self.validate_token(token).await?;

        if new_password.len() < 6 {
            return Err(ResetError::WeakPassword);
        }
        if new_password != confirm {
            return Err(ResetError::PasswordMismatch);
        }

        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(new_password.as_bytes(), &salt)
            .map_err(|_| ResetError::HashError)?
            .to_string();

        let token_hash = sha256_hex(token);

        let mut tx = self.db.begin().await?;
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2")
            .bind(&password_hash)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE password_reset_tokens SET used_at = NOW() WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        Ok(())
    }

    async fn deliver_reset_link(&self, user_email: &str, user_id: &Uuid, reset_url: &str) {
        if self.email.is_configured() && !self.base_url.is_empty() {
            if let Err(e) = self
                .email
                .send_password_reset(user_email, reset_url)
                .await
            {
                eprintln!("Password reset email failed for user {}: {}", user_id, e);
            }
            return;
        }

        let chat_id = match self.fetch_telegram_chat_id(user_id).await {
            Some(id) => id,
            None => {
                eprintln!(
                    "Password reset requested for user {} but no delivery channel available",
                    user_id
                );
                return;
            }
        };

        let text = format!("Восстановление пароля: {}", reset_url);
        if let Err(e) = self.telegram.send_message(&chat_id, &text).await {
            eprintln!("Password reset telegram failed for user {}: {}", user_id, e);
        }
    }

    async fn fetch_telegram_chat_id(&self, user_id: &Uuid) -> Option<String> {
        sqlx::query_scalar(
            "SELECT telegram_chat_id FROM users WHERE id = $1 AND telegram_chat_id IS NOT NULL",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        .ok()
        .flatten()
    }
}
