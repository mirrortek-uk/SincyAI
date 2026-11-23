//! Connector SPI (设计 §4/§12): 新增 Source 只需要实现这一个 trait。
//! Connector 只负责"拿回来并解析"(接入), 不负责理解数据(那是 Processor 的事)。
//! 传输工具 (rss / http_json / html / browser) 是共享零件, 不是扩展单位。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::record::{Batch, ConnectorError, Cursor, RecordType};

/// 连接器声明: 数据驱动注册 (id / 产出哪些 record_type / 调度 / 限频)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorDescriptor {
    pub id: String,
    pub name: String,
    /// 连接器类型: rss / api / website / search / social / aggregator / custom
    pub connector_type: String,
    pub capabilities: Vec<String>,
    /// 该连接器覆盖的 datasets: dataset_id -> record_type
    pub datasets: Vec<DatasetDecl>,
    pub default_interval_secs: u64,
    pub rate_limit_per_min: u32,
    #[serde(default)]
    pub auth: AuthDecl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetDecl {
    pub id: String,
    pub record_type: RecordType,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthDecl {
    pub auth_type: String, // none / bearer / api_key / oauth
    #[serde(default)]
    pub ref_name: Option<String>, // 指向密钥管理, 不直接存 secret
}

/// 拉取参数
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchParams {
    pub dataset_id: String,
    #[serde(default)]
    pub query: Value,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Connector SPI —— 为新源实现的唯一接口
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;
    /// 拉取 cursor 之后的新记录; 返回记录批次 + 下一个 cursor (幂等轮询)
    async fn fetch(
        &self,
        params: &FetchParams,
        cursor: Option<Cursor>,
    ) -> Result<Batch, ConnectorError>;
}

/// 连接器注册表 (设计 §5: 注册在 registry, descriptor 驱动调度)
#[derive(Default)]
pub struct ConnectorRegistry {
    connectors: Vec<Box<dyn Connector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self {
            connectors: Vec::new(),
        }
    }
    pub fn register(&mut self, c: Box<dyn Connector>) {
        self.connectors.push(c);
    }
    pub fn get(&self, id: &str) -> Option<&dyn Connector> {
        self.connectors
            .iter()
            .find(|c| c.descriptor().id == id)
            .map(|c| c.as_ref())
    }
    pub fn list(&self) -> Vec<&ConnectorDescriptor> {
        self.connectors.iter().map(|c| c.descriptor()).collect()
    }
    pub fn record_type_of(&self, connector_id: &str, dataset_id: &str) -> RecordType {
        self.get(connector_id)
            .and_then(|c| {
                c.descriptor()
                    .datasets
                    .iter()
                    .find(|d| d.id == dataset_id)
                    .map(|d| d.record_type)
            })
            .unwrap_or(RecordType::Unknown)
    }
}
