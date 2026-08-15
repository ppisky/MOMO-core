# MOMO Runtime Configuration 0.2

**状态：** Experimental Implementation Profile  
**更新日期：** 2026-08-10

本文记录当前 Rust Core 能够导入、导出并实际应用的 TOML 字段。它不是稳定的 v1 配置标准；未知字段会被保留，以便后续演进。

## 配置边界

运行配置分为两个可独立选择的逻辑模块：

- 模型配置：`active_model_profile` 与 `[[models]]`，支持多个模型配置；
- 系统配置：当前为 `[context]`。服务连接属于运行环境内部信息，不进入可移植配置。

用户可以只导出模型配置、只导出系统配置，或将两者合并到同一个 TOML/MOC 中。只选择一个模块时，另一个已知模块不会被基线配置重新带入；未知扩展字段仍尽可能往返保留。

```toml
schema_version = 2
active_model_profile = "019c0000-0000-7000-8000-000000000001"

[[models]]
profile_id = "019c0000-0000-7000-8000-000000000001"
name = "日常对话"
base_url = "https://provider.example/v1"
id = "model-id"
temperature = 0.7
top_p = 0.9
max_tokens = 1024
context_window = 8192
stream = true

[context]
window = 8192
```

`profile_id` 是客户端生成的 UUIDv7。内置配置固定显示为“Grok 4.5”，实际请求模型 ID 固定为 `grok-4.5`。界面只向用户说明“当前使用 Grok 4.5 模型”，不展示上游服务商或部署地址；`active_model_profile` 指向当前用于新一轮模型调用的配置。第三方 API Key 可以随自定义模型保存，但只进入本机操作系统安全凭据库，不会进入可移植 TOML 或 MOC；导入同一 `profile_id` 时，本机已有 API Key 会保留。

## 兼容与行为

- 导入要求扩展名为 `.toml`、UTF-8 文本且语法有效；
- 本 Profile 只定义 schema v2；旧 schema 的转换属于外部工具，不属于 Rust Core 主线；
- 缺少某个逻辑模块表示“不修改该模块”，而不是重置；
- 已知字段导入后立即应用；未知字段保存在本地基线文档，后续导出时继续保留；
- 任意层级出现 `api_key`、`*_api_key`、`password`、`secret`、`access_token` 或 `refresh_token` 时拒绝导入或导出；
- 导入模型配置不会清除或替换当前已安全保存的 API Key。

记忆提示词、记忆提炼模型、能力配置和上下文压缩尚未在当前 Rust Core 中形成稳定可移植配置契约。当前 Profile 已允许在 `[memory_system]` 中保存非凭据的 `distill_model_profile_id` 与 `distill_prompt`，但客户端不得把该扩展描述为完整配置标准。
