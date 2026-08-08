mod common;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use mediatracker::routes::auth;

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

async fn insert_reset_token(
    pool: &PgPool,
    user_id: &Uuid,
    token: &str,
    expires_at: chrono::DateTime<Utc>,
) {
    let token_hash = sha256_hex(token);
    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn auth_register_and_login() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route("/login", axum::routing::get(auth::get_login).post(auth::post_login))
        .route("/register", axum::routing::get(auth::get_register).post(auth::post_register))
        .with_state(state);

    // Register a new user
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=testuser&email=test@example.com&password=secret123"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_redirection(),
        "Register should redirect, got {}",
        response.status()
    );

    // Login with the new user
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=testuser&password=secret123"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_redirection(),
        "Login should redirect, got {}",
        response.status()
    );

    let session_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("Login should set session cookie");

    assert!(session_cookie.starts_with("session_id="), "Cookie should be session_id=...");
}

#[tokio::test]
async fn auth_register_duplicate_fails() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route("/register", axum::routing::get(auth::get_register).post(auth::post_register))
        .with_state(state);

    let form_body = "username=dupuser&email=dup@example.com&password=secret123";

    // First registration should work
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(form_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_redirection(), "First register should redirect");

    // Second registration with same username should fail
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(form_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Duplicate registration should return 400"
    );
}

#[tokio::test]
async fn auth_login_bad_password_fails() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route("/register", axum::routing::get(auth::get_register).post(auth::post_register))
        .route("/login", axum::routing::get(auth::get_login).post(auth::post_login))
        .with_state(state);

    // Register user
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=badpwd&email=badpwd@example.com&password=correctpw"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    // Login with wrong password
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=badpwd&password=wrongpw"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Wrong password should return 401"
    );
}

#[tokio::test]
async fn auth_login_nonexistent_user_fails() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route("/login", axum::routing::get(auth::get_login).post(auth::post_login))
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=nobody&password=anything"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Non-existent user should return 401"
    );
}

#[tokio::test]
async fn forgot_password_anti_enumeration() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state_with_email().await;

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route(
            "/forgot-password",
            axum::routing::get(auth::get_forgot_password).post(auth::post_forgot_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=enumuser&email=enum@example.com&password=secret123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let existing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/forgot-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("email=enum@example.com"))
                .unwrap(),
        )
        .await
        .unwrap();

    let nonexistent = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/forgot-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("email=nobody@example.com"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        existing.status(),
        nonexistent.status(),
        "Responses should be identical regardless of account existence"
    );
    assert!(
        existing.status().is_success(),
        "Should return 200, got {}",
        existing.status()
    );
}

#[tokio::test]
async fn forgot_password_creates_token() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state_with_email().await;

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route(
            "/forgot-password",
            axum::routing::get(auth::get_forgot_password).post(auth::post_forgot_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=tokenuser&email=token@example.com&password=secret123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/forgot-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("email=token@example.com"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "Forgot password should return 200, got {}",
        response.status()
    );

    let (token_hash,): (String,) = sqlx::query_as(
        "SELECT t.token_hash FROM password_reset_tokens t \
         JOIN users u ON u.id = t.user_id WHERE u.email = $1",
    )
    .bind("token@example.com")
    .fetch_one(&ctx.pool)
    .await
    .unwrap();

    assert_eq!(
        token_hash.len(),
        64,
        "token_hash should be sha256 hex (64 chars)"
    );
    assert!(
        token_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "token_hash should be hex"
    );
}

#[tokio::test]
async fn reset_password_success() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route("/login", axum::routing::get(auth::get_login).post(auth::post_login))
        .route(
            "/reset-password",
            axum::routing::get(auth::get_reset_password).post(auth::post_reset_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=resetuser&email=reset@example.com&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind("resetuser")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();

    let token = Uuid::new_v4().to_string();
    insert_reset_token(&ctx.pool, &user_id, &token, Utc::now() + Duration::hours(1)).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reset-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "token={}&password=newpass456&confirm_password=newpass456",
                    token
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_redirection(),
        "Reset should redirect to login, got {}",
        response.status()
    );

    let old_pw = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=resetuser&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        old_pw.status(),
        StatusCode::UNAUTHORIZED,
        "Old password should no longer work"
    );

    let new_pw = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=resetuser&password=newpass456"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        new_pw.status().is_redirection(),
        "New password should work"
    );
}

#[tokio::test]
async fn reset_password_invalidates_sessions() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route("/login", axum::routing::get(auth::get_login).post(auth::post_login))
        .route(
            "/reset-password",
            axum::routing::get(auth::get_reset_password).post(auth::post_reset_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=sessuser&email=sess@example.com&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=sessuser&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("Login should set session cookie");
    let session_token = cookie
        .split('=')
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    assert!(
        ctx.state.auth.get_session(&session_token).await.is_ok(),
        "Session should be valid before reset"
    );

    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind("sessuser")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();

    let token = Uuid::new_v4().to_string();
    insert_reset_token(&ctx.pool, &user_id, &token, Utc::now() + Duration::hours(1)).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reset-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "token={}&password=newpass456&confirm_password=newpass456",
                    token
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    assert!(
        ctx.state.auth.get_session(&session_token).await.is_err(),
        "Session should be invalidated after password reset"
    );
}

#[tokio::test]
async fn reset_password_token_one_time() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route(
            "/reset-password",
            axum::routing::get(auth::get_reset_password).post(auth::post_reset_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=oneuser&email=one@example.com&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind("oneuser")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();

    let token = Uuid::new_v4().to_string();
    insert_reset_token(&ctx.pool, &user_id, &token, Utc::now() + Duration::hours(1)).await;

    let body = format!(
        "token={}&password=newpass456&confirm_password=newpass456",
        token
    );

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reset-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(first.status().is_redirection());

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reset-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        second.status(),
        StatusCode::BAD_REQUEST,
        "Token should be single-use"
    );
}

#[tokio::test]
async fn reset_password_expired_token_rejected() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route(
            "/register",
            axum::routing::get(auth::get_register).post(auth::post_register),
        )
        .route(
            "/reset-password",
            axum::routing::get(auth::get_reset_password).post(auth::post_reset_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("username=expuser&email=exp@example.com&password=oldpass123"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());

    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind("expuser")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();

    let token = Uuid::new_v4().to_string();
    insert_reset_token(
        &ctx.pool,
        &user_id,
        &token,
        Utc::now() - Duration::hours(1),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reset-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "token={}&password=newpass456&confirm_password=newpass456",
                    token
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Expired token should be rejected"
    );
}

#[tokio::test]
async fn forgot_password_no_channels_returns_503() {
    let ctx = common::TestContext::new().await;
    let state = ctx.app_state();

    let app = Router::new()
        .route(
            "/forgot-password",
            axum::routing::get(auth::get_forgot_password).post(auth::post_forgot_password),
        )
        .with_state(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/forgot-password")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("email=anyone@example.com"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "Without any channel configured forgot-password should return 503"
    );
}
