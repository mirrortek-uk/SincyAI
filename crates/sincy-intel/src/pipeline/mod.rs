//! Sincy Intelligence Ingestion & Processing Pipeline (Sincy-05 思路):
//! Connector(接入) 与 Processor(理解) 分离; 统一 RawRecord 信封;
//! 按 RecordType 路由到独立 pipeline; 统一输出 IntelEntity + Signal。
//!
//! 加新源 = 写一个 Connector (复用已有 record_type); 真正的新形状才加新 RecordType + 新 pipeline。

pub mod connector;
pub mod engine;
pub mod graph;
pub mod processor;
pub mod processors;
pub mod record;
pub mod signals;

pub use connector::{Connector, ConnectorDescriptor, ConnectorRegistry, DatasetDecl, FetchParams};
pub use engine::{PipelineEngine, default_engine};
pub use graph::{Pipeline, PipelineRegistry};
pub use processor::{ProcessingContext, ProcessingResult, Processor, ProcessorRegistry};
pub use record::{Batch, ConnectorError, Cursor, RawRecord, RecordMetadata, RecordType};
pub use signals::{IntelEntity, Signal, SignalType};
