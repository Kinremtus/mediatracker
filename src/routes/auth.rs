use askama::Template;
use axum::{
    extract::{Form, State},
    http::{header::SET_COOKIE, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::app_state::AppState;

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
            let cookie = format!("session_id={}; Path=/; HttpOnly; Max-Age=2592000", token);
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
