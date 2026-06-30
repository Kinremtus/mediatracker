mod common;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;
use mediatracker::routes::auth;

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
