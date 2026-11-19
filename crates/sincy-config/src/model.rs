//! Configuration model shared by all apps (cloud-api, workers, agents).

use serde::{Deserialize, Serialize};
use sincy_common::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub env: String,
    pub http: HttpConfig,
    pub database: DatabaseConfig,
    pub worker: WorkerConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// postgres://... — required outside `local` when running migrations.
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_pool_size")]
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// NATS JetStream URL (ADR-007).
    #[serde(default = "default_nats_url")]
    pub nats_url: String,
    /// Redis URL for fast rate limiting only; never authoritative (TDD 3).
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// e.g. "info,sincy=debug"
    #[serde(default = "default_log_filter")]
    pub filter: String,
    #[serde(default)]
    pub json: bool,
}

fn default_pool_size() -> u32 {
    10
}
fn default_nats_url() -> String {
    "nats://127.0.0.1:4222".to_string()
}
fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}
fn default_log_filter() -> String {
    "info".to_string()
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: default_pool_size(),
        }
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            nats_url: default_nats_url(),
            redis_url: default_redis_url(),
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: default_log_filter(),
            json: false,
        }
    }
}

impl AppConfig {
    pub fn defaults_for(env: &str) -> Self {
        Self {
            env: env.to_string(),
            http: HttpConfig::default(),
            database: DatabaseConfig::default(),
            worker: WorkerConfig::default(),
            log: LogConfig::default(),
        }
    }

    /// Overlay fields from a parsed file config. The environment name always
    /// comes from the caller argument, never from the file.
    pub fn merge(&mut self, other: AppConfig) {
        self.http = other.http;
        self.database = other.database;
        self.worker = other.worker;
        self.log = other.log;
    }

    /// Apply SINCY_* environment overrides.
    pub fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("SINCY_HTTP_HOST") {
            self.http.host = v;
        }
        if let Ok(v) = std::env::var("SINCY_HTTP_PORT")
            && let Ok(p) = v.parse()
        {
            self.http.port = p;
        }
        if let Ok(v) = std::env::var("SINCY_DATABASE_URL") {
            self.database.url = v;
        }
        if let Ok(v) = std::env::var("SINCY_NATS_URL") {
            self.worker.nats_url = v;
        }
        if let Ok(v) = std::env::var("SINCY_REDIS_URL") {
            self.worker.redis_url = v;
        }
        if let Ok(v) = std::env::var("SINCY_LOG_FILTER") {
            self.log.filter = v;
        }
        if let Ok(v) = std::env::var("SINCY_LOG_JSON") {
            self.log.json = v == "1" || v.eq_ignore_ascii_case("true");
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.env != "local" && self.database.url.is_empty() {
            return Err(AppError::non_retryable(
                "config_invalid",
                format!("database url is required in {} environment", self.env),
            ));
        }
        if self.http.port == 0 {
            return Err(AppError::non_retryable(
                "config_invalid",
                "http port must be > 0",
            ));
        }
        Ok(())
    }
}
