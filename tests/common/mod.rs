use mediatracker::app_state::AppState;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;

#[allow(dead_code)]
pub struct TestContext {
    pub container: ContainerAsync<PostgresImage>,
    pub pool: PgPool,
    pub state: AppState,
    pub database_url: String,
}

impl TestContext {
    pub async fn new() -> Self {
        let container = PostgresImage::default()
            .with_user("test")
            .with_password("test")
            .with_db_name("test")
            .start()
            .await
            .expect("Failed to start postgres container");

        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let database_url = format!("postgres://test:test@{host}:{port}/test");

        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to postgres");

        sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
            .execute(&pool)
            .await
            .expect("Failed to create pgcrypto extension");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        let state = AppState::new(&database_url, "", "", "", "", "", "", "", "")
            .await
            .expect("Failed to create AppState");

        Self {
            container,
            pool,
            state,
            database_url,
        }
    }

    #[allow(dead_code)]
    pub fn app_state(&self) -> AppState {
        self.state.clone()
    }

    #[allow(dead_code)]
    pub async fn app_state_with_email(&self) -> AppState {
        AppState::new(
            &self.database_url,
            "",
            "",
            "",
            "",
            "",
            "re_dummy_test_key",
            "tracker@example.com",
            "https://tracker.example.com",
        )
        .await
        .expect("Failed to create AppState with email")
    }
}
