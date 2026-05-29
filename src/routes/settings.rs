use askama::Template;
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Redirect, Response},
    http::{header::SET_COOKIE, HeaderValue},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    username: String,
    email: String,
    stats: SidebarStats,
    active_page: String,
    message: Option<String>,
    error: Option<String>,
    email_notifications: bool,
    weekly_digest: bool,
    current_status: String,
    telegram_chat_id: String,
    telegram_notifications_enabled: bool,
}

#[derive(Deserialize)]
pub struct ProfileForm {
    username: String,
    email: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

pub async fn get_settings(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Html<String> {
    let stats = get_sidebar_stats(&state, user.id).await;

    let user_data = sqlx::query_as::<_, (String, String, Option<String>, bool)>(
        "SELECT username, email, telegram_chat_id, telegram_notifications_enabled FROM users WHERE id = $1"
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await
    .unwrap_or((user.username.clone(), String::new(), None, false));

    SettingsTemplate {
        username: user_data.0,
        email: user_data.1,
        stats,
        active_page: "settings".to_string(),
        message: None,
        error: None,
        email_notifications: false,
        weekly_digest: false,
        current_status: String::new(),
        telegram_chat_id: user_data.2.unwrap_or_default(),
        telegram_notifications_enabled: user_data.3,
    }
    .render()
    .unwrap()
    .into()
}

pub async fn post_profile(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let result = sqlx::query(
        "UPDATE users SET username = $1, email = $2, updated_at = NOW() WHERE id = $3"
    )
    .bind(&form.username)
    .bind(&form.email)
    .bind(user.id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            let user_data = sqlx::query_as::<_, (String, String)>(
                "SELECT username, email FROM users WHERE id = $1"
            )
            .bind(user.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or((form.username, form.email));

            Html(SettingsTemplate {
                username: user_data.0,
                email: user_data.1,
                stats,
                active_page: "settings".to_string(),
                message: Some("Профиль обновлён".to_string()),
                error: None,
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response()
        }
        Err(e) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            Html(SettingsTemplate {
                username: user.username,
                email: String::new(),
                stats,
                active_page: "settings".to_string(),
                message: None,
                error: Some(format!("Ошибка: {}", e)),
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response()
        }
    }
}

pub async fn post_password(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if form.new_password != form.confirm_password {
        let stats = get_sidebar_stats(&state, user.id).await;
        return Html(SettingsTemplate {
            username: user.username,
            email: String::new(),
            stats,
            active_page: "settings".to_string(),
            message: None,
            error: Some("Новые пароли не совпадают".to_string()),
            email_notifications: false,
            weekly_digest: false,
            current_status: String::new(),
            telegram_chat_id: String::new(),
            telegram_notifications_enabled: false,
        }.render().unwrap()).into_response();
    }

    if form.new_password.len() < 6 {
        let stats = get_sidebar_stats(&state, user.id).await;
        return Html(SettingsTemplate {
            username: user.username,
            email: String::new(),
            stats,
            active_page: "settings".to_string(),
            message: None,
            error: Some("Пароль должен быть не менее 6 символов".to_string()),
            email_notifications: false,
            weekly_digest: false,
            current_status: String::new(),
            telegram_chat_id: String::new(),
            telegram_notifications_enabled: false,
        }.render().unwrap()).into_response();
    }

    let user_data = sqlx::query_as::<_, (String, String)>(
        "SELECT username, password_hash FROM users WHERE id = $1"
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let user_data = match user_data {
        Ok(d) => d,
        Err(_) => {
            return Redirect::to("/login").into_response();
        }
    };

    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;

    let parsed_hash = match PasswordHash::new(&user_data.1) {
        Ok(h) => h,
        Err(_) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            return Html(SettingsTemplate {
                username: user.username,
                email: String::new(),
                stats,
                active_page: "settings".to_string(),
                message: None,
                error: Some("Ошибка проверки пароля".to_string()),
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response();
        }
    };

    if Argon2::default().verify_password(form.current_password.as_bytes(), &parsed_hash).is_err() {
        let stats = get_sidebar_stats(&state, user.id).await;
        return Html(SettingsTemplate {
            username: user.username,
            email: String::new(),
            stats,
            active_page: "settings".to_string(),
            message: None,
            error: Some("Текущий пароль неверен".to_string()),
            email_notifications: false,
            weekly_digest: false,
            current_status: String::new(),
            telegram_chat_id: String::new(),
            telegram_notifications_enabled: false,
        }.render().unwrap()).into_response();
    }

    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(_) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            return Html(SettingsTemplate {
                username: user.username,
                email: String::new(),
                stats,
                active_page: "settings".to_string(),
                message: None,
                error: Some("Ошибка хеширования пароля".to_string()),
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response();
        }
    };

    let result = sqlx::query(
        "UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(new_hash)
    .bind(user.id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            Html(SettingsTemplate {
                username: user.username,
                email: String::new(),
                stats,
                active_page: "settings".to_string(),
                message: Some("Пароль успешно изменён".to_string()),
                error: None,
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response()
        }
        Err(e) => {
            let stats = get_sidebar_stats(&state, user.id).await;
            Html(SettingsTemplate {
                username: user.username,
                email: String::new(),
                stats,
                active_page: "settings".to_string(),
                message: None,
                error: Some(format!("Ошибка: {}", e)),
                email_notifications: false,
                weekly_digest: false,
                current_status: String::new(),
                telegram_chat_id: String::new(),
                telegram_notifications_enabled: false,
            }.render().unwrap()).into_response()
        }
    }
}

pub async fn post_delete_account(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Response {
    let uid = user.id;

    let _ = sqlx::query("DELETE FROM activity_log WHERE user_id = $1").bind(uid).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM external_mappings WHERE user_id = $1").bind(uid).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM tracking_entries WHERE user_id = $1").bind(uid).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(uid).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1").bind(uid).execute(&state.db).await;

    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str("session_id=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0").unwrap(),
    );
    response
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user_id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp }
}

// ========== HTMX Endpoints ==========

#[derive(Template)]
#[template(path = "partials/message.html")]
struct MessagePartial {
    message: Option<String>,
    error: Option<String>,
}

pub async fn htmx_update_profile(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let result = sqlx::query(
        "UPDATE users SET username = $1, email = $2, updated_at = NOW() WHERE id = $3"
    )
    .bind(&form.username)
    .bind(&form.email)
    .bind(user.id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            let html = MessagePartial {
                message: Some("Профиль обновлён".to_string()),
                error: None,
            }.render().unwrap();
            (
                [("HX-Trigger", "profileUpdated")],
                Html(html),
            ).into_response()
        }
        Err(e) => {
            let html = MessagePartial {
                message: None,
                error: Some(format!("Ошибка: {}", e)),
            }.render().unwrap();
            Html(html).into_response()
        }
    }
}

pub async fn htmx_update_password(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if form.new_password != form.confirm_password {
        let html = MessagePartial {
            message: None,
            error: Some("Новые пароли не совпадают".to_string()),
        }.render().unwrap();
        return Html(html).into_response();
    }

    if form.new_password.len() < 6 {
        let html = MessagePartial {
            message: None,
            error: Some("Пароль должен быть не менее 6 символов".to_string()),
        }.render().unwrap();
        return Html(html).into_response();
    }

    let user_data = sqlx::query_as::<_, (String, String)>(
        "SELECT username, password_hash FROM users WHERE id = $1"
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let user_data = match user_data {
        Ok(d) => d,
        Err(_) => {
            return Redirect::to("/login").into_response();
        }
    };

    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;

    let parsed_hash = match PasswordHash::new(&user_data.1) {
        Ok(h) => h,
        Err(_) => {
            let html = MessagePartial {
                message: None,
                error: Some("Ошибка проверки пароля".to_string()),
            }.render().unwrap();
            return Html(html).into_response();
        }
    };

    if Argon2::default().verify_password(form.current_password.as_bytes(), &parsed_hash).is_err() {
        let html = MessagePartial {
            message: None,
            error: Some("Текущий пароль неверен".to_string()),
        }.render().unwrap();
        return Html(html).into_response();
    }

    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(_) => {
            let html = MessagePartial {
                message: None,
                error: Some("Ошибка хеширования пароля".to_string()),
            }.render().unwrap();
            return Html(html).into_response();
        }
    };

    let result = sqlx::query(
        "UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(new_hash)
    .bind(user.id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            let html = MessagePartial {
                message: Some("Пароль успешно изменён".to_string()),
                error: None,
            }.render().unwrap();
            (
                [("HX-Trigger", "passwordUpdated")],
                Html(html),
            ).into_response()
        }
        Err(e) => {
            let html = MessagePartial {
                message: None,
                error: Some(format!("Ошибка: {}", e)),
            }.render().unwrap();
            Html(html).into_response()
        }
    }
}

// ========== Telegram Endpoints ==========

#[derive(Deserialize)]
pub struct TelegramForm {
    telegram_chat_id: String,
}

pub async fn htmx_save_telegram_chat_id(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<TelegramForm>,
) -> Response {
    let result = sqlx::query(
        "UPDATE users SET telegram_chat_id = $1, telegram_notifications_enabled = ($1 != ''), updated_at = NOW() WHERE id = $2"
    )
    .bind(&form.telegram_chat_id)
    .bind(user.id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            let html = MessagePartial {
                message: Some("Telegram_CHAT_ID сохранён".to_string()),
                error: None,
            }.render().unwrap();
            Html(html).into_response()
        }
        Err(e) => {
            let html = MessagePartial {
                message: None,
                error: Some(format!("Ошибка: {}", e)),
            }.render().unwrap();
            Html(html).into_response()
        }
    }
}

pub async fn htmx_test_telegram(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Response {
    let chat_id: Option<(String,)> = sqlx::query_as(
        "SELECT telegram_chat_id FROM users WHERE id = $1 AND telegram_chat_id IS NOT NULL"
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let chat_id = match chat_id {
        Some((id,)) => id,
        None => {
            let html = MessagePartial {
                message: None,
                error: Some("Сначала укажите Telegram_CHAT_ID".to_string()),
            }.render().unwrap();
            return Html(html).into_response();
        }
    };

    match state.telegram.send_test_message(&chat_id).await {
        Ok(_) => {
            let html = MessagePartial {
                message: Some("Тестовое сообщение отправлено!".to_string()),
                error: None,
            }.render().unwrap();
            Html(html).into_response()
        }
        Err(e) => {
            let html = MessagePartial {
                message: None,
                error: Some(format!("Ошибка отправки: {}", e)),
            }.render().unwrap();
            Html(html).into_response()
        }
    }
}
