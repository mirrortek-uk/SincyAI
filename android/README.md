# Sincy Android Agent (WP-11/12 骨架)

Kotlin 外壳 + Rust core（JNI，ADR-002）。当前为**结构骨架**：无 Android SDK/真机环境，
未在本仓库 CI 中编译，真机矩阵验证（OS/机型/权限）待设备环境（指导书 §8 Device 层）。

- `app/` — Kotlin：MainActivity、AgentForegroundService（WSS 控制通道 + 前台服务）、
  SincyAccessibilityService（控件树观察层，ADR-013）、AdapterRuntime（JNI 门面）、AgentGateway（OkHttp WSS）
- `rust-core/` — Rust cdylib，复用云端同一套 sincy-execution 状态机与 sincy-adapter 解释器，
  设备端行为不可能偏离服务器策略

红线（指导书 DoD）：不实现验证码处理、设备指纹伪造、多账号防关联；挑战界面一律
`challenge.detected` → 云端置 `WAITING_USER` 转人工；Adapter 版本越界零副作用拒绝并报 `ui_drift`。
