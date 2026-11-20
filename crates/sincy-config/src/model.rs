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
    pub security: SecurityConfig,
    pub ai: AiConfig,
}

/// LLM provider settings (WP-14). provider: "mock" (dev only) or
/// "openai-compatible" (any /chat/completions endpoint: OpenAI, Azure, vLLM...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_llm_provider")]
    pub llm_provider: String,
    #[serde(default)]
    pub llm_base_url: String,
    #[serde(default)]
    pub llm_api_key: String,
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
}

fn default_llm_provider() -> String {
    "mock".to_string()
}
fn default_llm_model() -> String {
    "gpt-4o-mini".to_string()
}

/// Security/product flags. Mock adapters are FORBIDDEN unless explicitly
/// enabled (dev only) — DoD red line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// HMAC secret for console JWTs. Required outside local.
    #[serde(default)]
    pub jwt_secret: String,
    #[serde(default = "default_jwt_ttl")]
    pub jwt_ttl_minutes: i64,
    /// When false (default, required in production) mock channel adapters
    /// refuse to execute and the API route uses the real platform adapter.
    #[serde(default)]
    pub allow_mock_adapters: bool,
    /// Monthly model-cost budget in USD (FR-026); empty = unlimited (dev only).
    #[serde(default)]
    pub monthly_budget_usd: Option<f64>,
}

fn default_jwt_ttl() -> i64 {
    60
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

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            jwt_secret: String::new(),
            jwt_ttl_minutes: default_jwt_ttl(),
            allow_mock_adapters: false,
            monthly_budget_usd: None,
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            llm_provider: default_llm_provider(),
            llm_base_url: "https://api.openai.com/v1".to_string(),
            llm_api_key: String::new(),
            llm_model: default_llm_model(),
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
            security: SecurityConfig::default(),
            ai: AiConfig::default(),
        }
    }

    /// Overlay fields from a parsed file config. The environment name always
    /// comes from the caller argument, never from the file.
    pub fn merge(&mut self, other: AppConfig) {
        self.http = other.http;
        self.database = other.database;
        self.worker = other.worker;
        self.log = other.log;
        self.security = other.security;
        self.ai = other.ai;
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
        if let Ok(v) = std::env::var("SINCY_JWT_SECRET") {
            self.security.jwt_secret = v;
        }
        if let Ok(v) = std::env::var("SINCY_JWT_TTL_MINUTES")
            && let Ok(m) = v.parse()
        {
            self.security.jwt_ttl_minutes = m;
        }
        if let Ok(v) = std::env::var("SINCY_ALLOW_MOCK_ADAPTERS") {
            self.security.allow_mock_adapters = v == "1" || v.eq_ignore_ascii_case("true");
        }
        if let Ok(v) = std::env::var("SINCY_MONTHLY_BUDGET_USD")
            && let Ok(b) = v.parse()
        {
            self.security.monthly_budget_usd = Some(b);
        }
        if let Ok(v) = std::env::var("SINCY_LLM_PROVIDER") {
            self.ai.llm_provider = v;
        }
        if let Ok(v) = std::env::var("SINCY_LLM_BASE_URL") {
            self.ai.llm_base_url = v;
        }
        if let Ok(v) = std::env::var("SINCY_LLM_API_KEY") {
            self.ai.llm_api_key = v;
        }
        if let Ok(v) = std::env::var("SINCY_LLM_MODEL") {
            self.ai.llm_model = v;
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
        if self.env != "local" {
            if self.security.jwt_secret.is_empty()
                || self.security.jwt_secret == "dev-only-not-for-production"
            {
                return Err(AppError::non_retryable(
                    "config_invalid",
                    "SINCY_JWT_SECRET must be set to a real secret outside local",
                ));
            }
            if self.security.allow_mock_adapters {
                return Err(AppError::non_retryable(
                    "config_invalid",
                    "SINCY_ALLOW_MOCK_ADAPTERS must not be enabled outside local",
                ));
            }
        }
        Ok(())
    }
}
