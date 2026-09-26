// Integration tests for MediaTracker: real DB round-trip via testcontainers.

mod common;

use mediatracker::models::user::CreateUser;
use uuid::Uuid;

#[tokio::test]
async fn register_login_session_logout_round_trip() {
    let ctx = common::TestContext::new().await;
    let suffix = Uuid::new_v4().simple().to_string();
    let username = format!("integration-{suffix}");
    let email = format!("integration-{suffix}@example.com");
    let password = "correct-horse-battery-staple";

    let data = CreateUser {
        username: username.clone(),
        email,
        password: password.to_string(),
    };

    let user = ctx.state.auth.register(&data).await.expect("register");
    assert_eq!(user.username, username);

    let token = ctx
        .state
        .auth
        .login(&username, password, Some("integration-test"), None)
        .await
        .expect("login");
    let session = ctx.state.auth.get_session(&token).await.expect("session");
    assert_eq!(session.user_id, user.id);

    assert!(
        ctx.state
            .auth
            .login(&username, "wrong-password", None, None)
            .await
            .is_err()
    );

    ctx.state.auth.logout(&token).await.expect("logout");
    assert!(ctx.state.auth.get_session(&token).await.is_err());
}
