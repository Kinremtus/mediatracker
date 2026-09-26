use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode, header::USER_AGENT},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::services::password_reset::ResetError;

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "auth/register.html")]
struct RegisterTemplate {
    error: Option<String>,
}

/// Renders an auth page, degrading to a 500 instead of panicking the request
/// task if a template ever fails to render.
fn render_auth_page<T: Template>(template: &T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct RegisterForm {
    username: String,
    email: String,
    password: String,
}

#[derive(Template)]
#[template(path = "auth/forgot_password.html")]
struct ForgotPasswordTemplate {
    message: Option<String>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "auth/reset_password.html")]
struct ResetPasswordTemplate {
    token: String,
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct ForgotPasswordForm {
    email: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordForm {
    token: String,
    password: String,
    confirm_password: String,
}

#[derive(Deserialize)]
pub struct ResetTokenQuery {
    token: String,
}

pub async fn get_login() -> Response {
    render_auth_page(&LoginTemplate { error: None })
}

pub async fn post_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    // Real client IP behind Cloudflare/Traefik: cf-connecting-ip, then the
    // leftmost x-forwarded-for entry. `client_ip` validates the value as an IP
    // address (headers are attacker-controlled and `sessions.ip` is INET), so a
    // bogus/absent source yields NULL.
    let ip = crate::utils::client_ip(&headers, None);

    match state
        .auth
        .login(&form.username, &form.password, user_agent, ip.as_deref())
        .await
    {
        Ok(token) => {
            let cookie = format!(
                "session_id={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000",
                token
            );
            let mut response = Redirect::to("/").into_response();
            crate::utils::set_set_cookie(&mut response, &cookie);
            response
        }
        Err(e) => {
            let html = LoginTemplate {
                error: Some(e.to_string()),
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            (StatusCode::UNAUTHORIZED, Html(html)).into_response()
        }
    }
}

pub async fn get_register() -> Response {
    render_auth_page(&RegisterTemplate { error: None })
}

pub async fn post_register(
    State(state): State<AppState>,
    Form(form): Form<RegisterForm>,
) -> Response {
    let create_user = crate::models::user::CreateUser {
        username: form.username,
        email: form.email,
        password: form.password,
    };

    match state.auth.register(&create_user).await {
        Ok(_) => Redirect::to("/login").into_response(),
        Err(e) => {
            let html = RegisterTemplate {
                error: Some(e.to_string()),
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
    }
}

pub async fn get_forgot_password() -> Html<String> {
    ForgotPasswordTemplate {
        message: None,
        error: None,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        String::from("Internal Server Error")
    })
    .into()
}

pub async fn post_forgot_password(
    State(state): State<AppState>,
    Form(form): Form<ForgotPasswordForm>,
) -> Response {
    if !state.password_reset.channels_available() {
        let html = ForgotPasswordTemplate {
            message: None,
            error: Some("Восстановление пароля временно недоступно. Попробуйте позже.".to_string()),
        }
        .render()
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "template render failed");
            String::from("Internal Server Error")
        });
        return (StatusCode::SERVICE_UNAVAILABLE, Html(html)).into_response();
    }

    let _ = state.password_reset.request_reset(&form.email).await;

    let html = ForgotPasswordTemplate {
        message: Some(
            "Если аккаунт с таким email существует, ссылка для восстановления отправлена."
                .to_string(),
        ),
        error: None,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        String::from("Internal Server Error")
    });
    Html(html).into_response()
}

pub async fn get_reset_password(
    State(state): State<AppState>,
    Query(query): Query<ResetTokenQuery>,
) -> Response {
    match state.password_reset.validate_token(&query.token).await {
        Ok(_) => {
            let html = ResetPasswordTemplate {
                token: query.token,
                error: None,
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            Html(html).into_response()
        }
        Err(_) => {
            let html = ResetPasswordTemplate {
                token: String::new(),
                error: Some(ResetError::InvalidToken.to_string()),
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
    }
}

pub async fn post_reset_password(
    State(state): State<AppState>,
    Form(form): Form<ResetPasswordForm>,
) -> Response {
    match state
        .password_reset
        .reset_password(&form.token, &form.password, &form.confirm_password)
        .await
    {
        Ok(_) => Redirect::to("/login").into_response(),
        Err(ResetError::InvalidToken) => {
            let html = ResetPasswordTemplate {
                token: String::new(),
                error: Some(ResetError::InvalidToken.to_string()),
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
        Err(e) => {
            let html = ResetPasswordTemplate {
                token: form.token,
                error: Some(e.to_string()),
            }
            .render()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "template render failed");
                String::from("Internal Server Error")
            });
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
    }
}
