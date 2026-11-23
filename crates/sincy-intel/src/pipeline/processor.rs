//! Processor SPI + 注册表 (设计 §6/§11/§16):
//! 三类 Processor —— Source-specific / Type-specific / Universal,
//! 按 record_type 路由到 pipeline, pipeline 内按序执行多个 processor。

use serde_json::Value;

use super::record::{RawRecord, RecordType};
use super::signals::{IntelEntity, Signal};

/// 处理上下文 (可携带历史快照等, 供 growth/momentum 计算)
#[derive(Debug, Clone, Default)]
pub struct ProcessingContext {
    /// 历史快照: key = (dataset_id, external_id) → 上次指标 payload
    pub history: std::collections::HashMap<String, Value>,
    /// 品牌画像 (相关性评分用), 从 cloud-api 注入
    pub brand_profile: Option<Value>,
}

/// 处理结果: 一条 raw record 可能产生多个实体 + 多个信号
#[derive(Debug, Clone, Default)]
pub struct ProcessingResult {
    pub entities: Vec<IntelEntity>,
    pub signals: Vec<Signal>,
    /// 附加存储动作 (如 ranking_snapshot / trend_series upsert), 由持久层执行
    pub side_effects: Vec<Value>,
}

/// Processor SPI
#[async_trait::async_trait]
pub trait Processor: Send + Sync {
    fn processor_id(&self) -> &str;
    fn version(&self) -> &str {
        "1.0.0"
    }
    /// 该 processor 处理哪些 record_type
    fn supported_record_types(&self) -> Vec<RecordType>;
    async fn process(
        &self,
        record: &RawRecord,
        ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String>;
}

/// Processor 注册表 (设计 §11)
#[derive(Default)]
pub struct ProcessorRegistry {
    processors: Vec<Box<dyn Processor>>,
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        Self {
            processors: Vec::new(),
        }
    }
    pub fn register(&mut self, p: Box<dyn Processor>) {
        self.processors.push(p);
    }
    pub fn for_record_type(&self, rt: RecordType) -> Vec<&dyn Processor> {
        self.processors
            .iter()
            .filter(|p| p.supported_record_types().contains(&rt))
            .map(|p| p.as_ref())
            .collect()
    }
    pub fn list(&self) -> Vec<(String, String, Vec<RecordType>)> {
        self.processors
            .iter()
            .map(|p| {
                (
                    p.processor_id().to_string(),
                    p.version().to_string(),
                    p.supported_record_types(),
                )
            })
            .collect()
    }
}
