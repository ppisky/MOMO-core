# 重新认识 MOMO Core

这份导航基于 2026-09-25 迁入的工作树和本轮阅读，不代替协议规范，也不表示所有实现已经审计通过。

## 它做什么

MOMO Core 是 AI 角色对话的本地运行底座。调用方给它角色、会话、输入和 Space 选择，它负责整理上下文、请求外部模型、保存回复，并在后台更新长期记忆。它还管理角色卡、图片输入、工具调用和可移植数据容器。模型推理由外部模型网关承担。

## 一次对话的主线

```text
HTTP / SSE 或 Rust 调用方
  → MomoApiService::execute：校验、请求去重、会话串行化
  → 读取角色与历史，处理图片输入
  → 按权重检索多个 Space 的 DMW / NSG
  → 观察并计算 MO State，组织受预算限制的上下文
  → 模型网关生成文本或工具调用
  → 保存回复、操作重放记录和待维护证据
  → 后台提炼 DMW、治理 NSG
```

主要入口是 `crates/momo-core/src/orchestration/execution.rs`；生成、响应组装、状态和维护分别在同目录的其他模块。Server 的路由在 `crates/momo-server/src/routes/`。

## 几个容易混淆的名字

| 名称 | 作用 |
| --- | --- |
| Space | 隔离会话、记忆等数据的命名空间；一个实例可有多个 Space，一次请求可从多个来源检索 |
| DMW | Dual-Mem Wiki，用 Markdown/YAML 文档保存长期记忆 |
| NSG | Narrative Semantic Graph，用图结构保存叙事语义，并支持向量检索 |
| MO State | 根据记忆、语义图和场景等资料形成当前状态，有版本、快照和审计记录 |
| DDM | 可选的角色倾向状态投影；应结合专门状态报告理解其实验性边界 |
| Prompt Spaces | 可在运行时替换的具名提示词；与数据命名空间 Space 是不同概念 |
| MOC | 导入导出的可移植容器，可包含所选角色、会话及记忆模块 |

Space 的授权由宿主承担。Core 的本地管理接口不能直接当成已经具备多租户权限管理的公网服务。

## 数据在哪里

| 数据 | 保存位置 | 地位 |
| --- | --- | --- |
| 角色、会话、消息、审核、操作记录 | `momo.sqlite3` | 业务持久数据 |
| DMW / NSG 文档 | `spaces/<space_id>/memory/` | 记忆与语义图的权威内容 |
| NSG 向量 | `nsg-vectors.db` | 可重建的检索缓存 |
| CHARX 原始资产 | `character-packages/<character_id>/source.charx` | 保留导入来源，支持往返导出 |
| 提示词覆盖 | `prompt-spaces.json` | 当前服务持久化的提示词配置 |

SQLite 和记忆文件需要共同完成操作，因此代码有 prepared commit 日志与启动恢复。并发锁、调用取消和恢复一致性是本轮重点审查区域。

## 配置怎么生效

当前 `MomoRuntime` 以 `MomoRuntimeSettings::default()` 初始化，通过本地管理 API 替换运行策略；服务不自动读取 `momo.example.toml`。运行策略由 Runtime 在内存中统一持有，重新启动会重新采用启动默认值，宿主需要按自己的生命周期重新应用策略。Prompt Spaces 同样由 Runtime 持有，但持久化到 `prompt-spaces.json`；新建服务门面不会重置两者。

服务地址、数据目录和模型网关等由 `MOMO_SERVER_BIND`、`MOMO_DATA_DIR`、`MOMO_MODEL_GATEWAY_ORIGIN` 等环境变量配置。Prompt Spaces 单独加载并保存覆盖值。排查配置问题时应先辨认属于哪一条路径。

## 从哪里继续读

1. `README.zh-CN.md`：功能与使用边界。
2. `docs/architecture_1_0.md`：模块依赖、资源所有权和恢复约束。
3. `docs/space_model_1_0.md`：Space、角色归属及读写空间。
4. `docs/http_api_1_0.md`：产品协议和本地管理接口。
5. `crates/momo-core/src/orchestration/execution.rs`：对话主流程。
6. `docs/code_review_2026_09_25.zh-CN.md`：本轮发现、验证证据与限制。

当前理解足以定位主要执行链和开展有针对性的审查；角色卡兼容、加密容器、完整状态投影与真实模型效果仍需各自验证。离线测试通过不能替代真实模型效果评测。
