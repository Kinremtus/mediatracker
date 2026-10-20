//! Shared test harness.
//!
//! One Postgres container per test binary (process) plus one migrated template
//! database; every `TestContext` clones the template into its own database
//! (`CREATE DATABASE ... TEMPLATE`). Tests stay fully isolated, but the suite
//! no longer pays for a container and a full migration run per test — the old
//! per-test containers (~94 per suite) dominated the test wall time.
//!
//! The container lives in a process-wide `OnceCell` and is removed by
//! testcontainers' reaper when the test process exits.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use mediatracker::app_state::AppState;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use tokio::sync::OnceCell;

struct SharedPg {
    /// Kept alive for the whole process; removed by `remove_shared_container_at_exit`.
    _container: ContainerAsync<PostgresImage>,
    host: String,
    port: u16,
    admin_url: String,
    template_db: String,
}

static SHARED: OnceCell<SharedPg> = OnceCell::const_new();
static NEXT_DB: AtomicU32 = AtomicU32::new(0);
static CONTAINER_ID: OnceLock<String> = OnceLock::new();

/// Remove the shared container at process exit.
///
/// testcontainers 0.27 has no reaper and no `Drop` cleanup, and a container
/// kept in a `static` is never dropped at all — without this hook every test
/// binary would leak one running Postgres container per suite run.
extern "C" fn remove_shared_container_at_exit() {
    let Some(id) = CONTAINER_ID.get() else {
        return;
    };
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", id.as_str()])
        .output();
}

/// Safety net for crashed runs (SIGKILL skips `atexit`): remove managed
/// containers left by earlier runs. The 30-minute age filter protects
/// containers of test binaries running concurrently — a suite run is shorter
/// than that.
fn prune_stale_test_containers() {
    let Ok(output) = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "label=org.testcontainers.managed-by=testcontainers",
            "--format",
            "{{.ID}} {{.CreatedAt}}",
        ])
        .output()
    else {
        return;
    };
    let now = chrono::Utc::now().naive_utc();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((id, created)) = line.split_once(' ') else {
            continue;
        };
        let created = created.trim();
        let stamp = &created[..19.min(created.len())];
        let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S") else {
            continue;
        };
        if now.signed_duration_since(parsed).num_minutes() > 30 {
            let _ = std::process::Command::new("docker")
                .args(["rm", "-f", id])
                .output();
        }
    }
}

async fn shared() -> &'static SharedPg {
    SHARED
        .get_or_init(|| async {
            prune_stale_test_containers();
            // Postgres 17, matching production: the testcontainers default is
            // still postgres:11, so the migrations were never exercised on the
            // server major the app actually runs.
            let container = PostgresImage::default()
                .with_user("test")
                .with_password("test")
                .with_db_name("test")
                .with_tag("17-alpine")
                .start()
                .await
                .expect("Failed to start postgres container");

            // Register the exit cleanup only once the container exists.
            let _ = CONTAINER_ID.set(container.id().to_string());
            // SAFETY: the handler reads a static and shells out once at exit.
            unsafe {
                libc::atexit(remove_shared_container_at_exit);
            }

            let host = container.get_host().await.unwrap().to_string();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let admin_url = format!("postgres://test:test@{host}:{port}/test");

            // Build the template once: extension + every migration. Clones
            // inherit both, so `AppState::new`'s migrate run is a fast no-op.
            let template_db = "tracker_template".to_string();
            let admin_pool = PgPool::connect(&admin_url)
                .await
                .expect("admin connection to the test container");
            sqlx::query("CREATE DATABASE tracker_template")
                .execute(&admin_pool)
                .await
                .expect("create the template database");

            let template_url = format!("postgres://test:test@{host}:{port}/{template_db}");
            let template_pool = PgPool::connect(&template_url)
                .await
                .expect("connection to the template database");
            sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
                .execute(&template_pool)
                .await
                .expect("create pgcrypto extension");
            sqlx::migrate!("./migrations")
                .run(&template_pool)
                .await
                .expect("run migrations in the template database");
            // `CREATE DATABASE ... TEMPLATE` refuses while the source is in
            // use, so both setup pools must be closed first.
            template_pool.close().await;
            admin_pool.close().await;

            SharedPg {
                _container: container,
                host,
                port,
                admin_url,
                template_db,
            }
        })
        .await
}

#[allow(dead_code)]
pub struct TestContext {
    pub pool: PgPool,
    pub state: AppState,
    pub database_url: String,
}

impl TestContext {
    pub async fn new() -> Self {
        let shared = shared().await;
        let seq = NEXT_DB.fetch_add(1, Ordering::Relaxed);
        let db_name = format!("test_{seq}");

        let admin_pool = PgPool::connect(&shared.admin_url)
            .await
            .expect("admin connection to the test container");
        sqlx::query(&format!(
            "CREATE DATABASE {db_name} TEMPLATE {}",
            shared.template_db
        ))
        .execute(&admin_pool)
        .await
        .expect("clone the template database for one test");
        admin_pool.close().await;

        let database_url = format!(
            "postgres://test:test@{}:{}/{}",
            shared.host, shared.port, db_name
        );
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to the per-test database");

        let state = AppState::new(&database_url, "", "", "", "", "", "", "", "", "", "")
            .await
            .expect("Failed to create AppState");

        Self {
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
            "",
            "",
        )
        .await
        .expect("Failed to create AppState with email")
    }
}
