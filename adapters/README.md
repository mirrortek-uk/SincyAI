# adapters/

声明式 App Adapter 定义（YAML/JSON）与回归 fixtures，与代码分离（TDD 9.3）。
Adapter 由 Sincy 签名发布；设备只接受签名有效且 version_range 匹配当前
App 版本的 Adapter。schema 校验工具与签名流水线在 WP-13 落地。
