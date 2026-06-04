pub mod auth;
pub mod static_version;

pub use auth::{auth_middleware, CurrentUser};
pub use static_version::static_version_middleware;
