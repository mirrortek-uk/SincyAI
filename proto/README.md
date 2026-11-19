# proto/

WP-02: 控制面 protobuf 定义（Envelope、任务、心跳、能力、App 版本上报、
步骤证据、Drift/挑战事件、传输消息、发布执行）。Rust 侧用 prost 生成，
Android 侧用 protoc gen-kotlin；两侧共用 golden fixtures 校验，
schema_version 不兼容时显式拒绝（见 sincy-protocol）。
