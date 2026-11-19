//! sincy-config: layered configuration.
//!
//! Precedence: built-in defaults < `config/<env>.toml` (optional) < `SINCY_*` env vars.
//! Secrets are NEVER stored in files committed to git; they are injected via env
//! or a local untracked file (see .gitignore).

use sincy_common::AppError;
use std::path::Path;

pub mod model;

pub use model::{AppConfig, DatabaseConfig, HttpConfig, LogConfig, WorkerConfig};

/// Load configuration for the given environment name (local|dev|staging|production).
pub fn load(env: &str) -> Result<AppConfig, AppError> {
    let mut cfg = AppConfig::defaults_for(env);
    let path = format!("config/{env}.toml");
    if Path::new(&path).exists() {
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            AppError::non_retryable("config_read", format!("cannot read {path}: {e}"))
        })?;
        let file_cfg: AppConfig = toml::from_str(&raw).map_err(|e| {
            AppError::non_retryable("config_parse", format!("cannot parse {path}: {e}"))
        })?;
        cfg.merge(file_cfg);
    }
    cfg.apply_env();
    cfg.validate()?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_for_local() {
        let cfg = AppConfig::defaults_for("local");
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.env, "local");
    }

    #[test]
    fn env_overrides_port() {
        unsafe { std::env::set_var("SINCY_HTTP_PORT", "19090") };
        let mut cfg = AppConfig::defaults_for("local");
        cfg.apply_env();
        assert_eq!(cfg.http.port, 19090);
        unsafe { std::env::remove_var("SINCY_HTTP_PORT") };
    }

    #[test]
    fn production_requires_database_url() {
        let cfg = AppConfig {
            database: DatabaseConfig {
                url: String::new(),
                ..DatabaseConfig::default()
            },
            ..AppConfig::defaults_for("production")
        };
        assert!(cfg.validate().is_err());
    }
}
