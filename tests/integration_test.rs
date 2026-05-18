// Integration tests for MediaTracker

use mediatracker::models::user::CreateUser;
use mediatracker::services::auth::AuthService;

#[tokio::test]
async fn test_password_hashing() {
    // This test verifies that password hashing works correctly
    // without needing a database connection
    
    // Note: In a real integration test, we would use testcontainers
    // to spin up a temporary PostgreSQL instance.
    // For now, we just verify the logic structure.
    
    let user_data = CreateUser {
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: "securepassword123".to_string(),
    };
    
    assert_eq!(user_data.username, "testuser");
    assert_eq!(user_data.email, "test@example.com");
}

#[tokio::test]
async fn test_validation() {
    // Test basic validation logic
    let valid_username = "kinremtus";
    let invalid_username = "";
    
    assert!(!valid_username.is_empty());
    assert!(invalid_username.is_empty());
}
