use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{header::SET_COOKIE, HeaderValue, StatusCode},
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

pub async fn get_login() -> Html<String> {
    LoginTemplate { error: None }.render().unwrap().into()
}

pub async fn post_login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let user_agent = "Unknown"; // TODO: Get from headers
    let ip = "127.0.0.1"; // TODO: Get from headers

    match state.auth.login(&form.username, &form.password, Some(user_agent), Some(ip)).await {
        Ok(token) => {
            let cookie = format!("session_id={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000", token);
            let mut response = Redirect::to("/").into_response();
            response.headers_mut().insert(
                SET_COOKIE,
                HeaderValue::from_str(&cookie).unwrap(),
            );
            response
        }
        Err(e) => {
            let html = LoginTemplate {
                error: Some(e.to_string()),
            }
            .render()
            .unwrap();
            (StatusCode::UNAUTHORIZED, Html(html)).into_response()
        }
    }
}

pub async fn get_register() -> Html<String> {
    RegisterTemplate { error: None }.render().unwrap().into()
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
            .unwrap();
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
    .unwrap()
    .into()
}

pub async fn post_forgot_password(
    State(state): State<AppState>,
    Form(form): Form<ForgotPasswordForm>,
) -> Response {
    if !state.password_reset.channels_available() {
        let html = ForgotPasswordTemplate {
            message: None,
            error: Some(
                "Восстановление пароля временно недоступно. Попробуйте позже.".to_string(),
            ),
        }
        .render()
        .unwrap();
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
    .unwrap();
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
            .unwrap();
            Html(html).into_response()
        }
        Err(_) => {
            let html = ResetPasswordTemplate {
                token: String::new(),
                error: Some(ResetError::InvalidToken.to_string()),
            }
            .render()
            .unwrap();
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
            .unwrap();
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
        Err(e) => {
            let html = ResetPasswordTemplate {
                token: form.token,
                error: Some(e.to_string()),
            }
            .render()
            .unwrap();
            (StatusCode::BAD_REQUEST, Html(html)).into_response()
        }
    }
}
