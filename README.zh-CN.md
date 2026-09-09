# MOMO Core

[English](README.md)

MOMO Core 是面向 AI 角色体验的本地优先 Rust 基础系统。角色数据、会话、长期记忆、
叙事语义、自治状态运行时、可移植容器、加密、模型网关与本地 HTTP 接口都在同一个 workspace
中实现。

## 主要能力

- MOMO 独立 Character Card v2（`character.toml` + Markdown）
- Character Card v1/v2 JSON 与 PNG 导入
- Character Card v3 JSON、PNG 与完整 CHARX 容器导入；明确拒绝 APNG
- 保留来源字段的 Character Card v2/v3 JSON 与 CHARX 导出
- 严格类型化的多 Space MOC v3 导入导出与显式一对一 Space 转换
- PNG/无损 WebP 的 MOMO LSB 载体；不支持 APNG 或 AVIF
- Dual-Mem Wiki（DMW）长期记忆
- Narrative Semantic Graph（NSG）
- MO State v2 自治运行时：按 Space 管理 DMW、NSG、场景、版本与持久快照
- SQLite 业务存储与独立 Turso 向量存储
- 原生 `MomoApi` 编排与面向模型服务的适配器接口
- 通过可选视觉描述适配器处理的受治理图片输入
- capability discovery 与上下文预算
- 向量存储契约与确定性检索

`crates/` 下的 crate 是 MOMO Core 的内部实现模块，不是彼此独立的产品，也不会作为
独立 crates.io 包发布。1.0 的产品稳定面是版本化的原生 HTTP wire（`momo.responses/1.0`
与 `momo.control/1.0`）和已记录的可移植格式；其余 `/v1` 路由属于本机管理 profile。嵌入方仍可从同一固定 workspace revision
使用 Rust API，但这些 API 没有独立的 crates.io SemVer 承诺。详见
[HTTP 边界](docs/http_api_1_0.md)与[规范索引](docs/spec_index.md)。`momo-server` 通过仅限
本机的 HTTP/SSE 接口提供同一组 Core 能力。

## 角色卡格式边界

MOMO Character Card v2 是由本仓库定义的独立角色卡格式，规范见
[`Character_Card_v2.md`](Character_Card_v2.md)。其中的“v2”不表示外部生态的
`chara_card_v2` JSON/PNG 格式。当前 Core 实现的是 MOMO 格式随 MOC v3 的导入导出；
同时支持外部 CCv1/v2 JSON、PNG，以及 CCv3 JSON、PNG、CHARX 的导入，并支持
CCv2/CCv3 JSON 与 CHARX 导出；APNG 明确不支持。CHARX 的资产、`x_meta`、`module.risum` 和未知安全条目
作为原始容器来源保存，也会随 MOC 往返。

外部格式兼容设计的规范来源固定为
[Character Card v2](https://github.com/malfoyslastname/character-card-spec-v2/blob/8083fb388615ccbce768e97cbbd49d2b3214632c/spec_v2.md) 与
[Character Card v3](https://github.com/kwaroran/character-card-spec-v3/blob/f3a86af019fbd99f788f7a1155f399655b34ab35/SPEC_V3.md)。Risu 专用扩展以
[RisuAI 参考实现](https://github.com/kwaroran/Risuai/blob/c0ed1026de4b06a1c4600b79c789fea0616c297c/src/ts/characterCards.ts)确认，
[Character Foundry CHARX 文档](https://github.com/character-foundry/character-foundry/blob/322fe8d940d1b91c978b43330b80ab2e115002e4/docs/charx.md)
仅用于下游交叉检查；所有外部规范文档都使用固定提交链接，不在仓库内搬运正文。来源优先级、许可证边界和当前实现状态见
[角色卡格式与兼容边界](docs/character_card_compatibility.md)。

## Workspace

- `momo-core`：编排与面向调用方的 Rust API
- `momo-domain`：共享领域类型
- `momo-storage`：SQLite 业务持久化与 Turso 向量存储
- `momo-memory`：DMW、NSG、检索、场景解析与 MO State 投影
- `momo-moc`：MOC 容器
- `momo-crypto`：私有容器加密
- `momo-config`：可移植运行配置
- `momo-server`：本地 HTTP/SSE 接口

## 数据存储

Core 明确使用两个独立数据库，而不是把全部数据放进同一个 SQLite 文件：

- `momo.sqlite3` 由 SQLx/SQLite 管理，保存角色、会话、消息、删除记录、Patch Review
  与可移植元数据等业务数据；
- `nsg-vectors.db` 由官方 `turso` Rust 库管理，只保存 NSG 向量索引。
- `character-packages/<character_id>/source.charx` 保存导入 CHARX 的原始容器，使二进制
  资产和应用扩展能够随 MOC 往返。

DMW 与 NSG 的 YAML/Markdown 源文档位于 `spaces/<space_id>/memory`，是记忆与语义图
的可移植事实来源。Turso 中的向量按来源哈希和向量空间校验，是可从源文档重新生成的
缓存，不进入 MOC。`NsgVectorStore` 只是隔离 Turso 实现细节的内部接口，不代表第三套
数据库。升级到 0.3.2 时，旧 SQLite `nsg_vectors` 表会被删除，宿主应按需重建向量缓存。

0.4.2 已将上游 embeddings 适配器对齐到 OpenAI 文档中的请求/响应元数据，保留 token
usage，并让本地生成接口正确返回 400/502/504。该本地接口仍是 MOMO 编排 API，不是可由
OpenAI SDK 直接替换 base URL 的服务端接口。原始向量接口继续作为低级能力保留。详见
[向量化模型接口规范](docs/vectorization_model_interface_0_4_2.md)。

0.5.0 已加入原生 MomoApi 响应契约与 `POST /v1/momo/responses`：一次请求完成消息持久化、
DMW/NSG 检索、MO State、上下文预算、逻辑路由和助手持久化，并支持真正的上游增量 SSE、
跨进程 request ID 幂等、Core 所有的 embedding profile，以及可重试的后台 DMW/NSG 维护。
三协议工具调用已有共享 golden contract 和异协议增量转换；
[MOMO LSB Carrier v1](docs/momo_lsb_carrier_v1.md) 已接入有界 PNG/无损 WebP codec。
0.5.0 发布候选还统一了错误 envelope、请求与流上限、取消/超时/限流和逻辑路由指标。兼容变化和 1.0 门槛见
[迁移说明](docs/migration_0_4_2_to_0_5_0.md)与[0.5.0 路线图](docs/roadmap_0_5_0.md)。

本地 1.0 候选现已接入完整图片输入链路：单次最多八张用户图片；主对话模型声明
`image` 时直接接收原图，纯文本主模型才使用可选的逻辑 `vision` 描述路由。解析结果与
usage 会持久化，保持 request ID 重放的确定性。两仓 `momo.responses/1.0` fixture 已冻结，
当前以 `v1.0.0-rc.2` 预发布；更广的凭据测试与 MORP 覆盖仍是稳定版 `v1.0.0`
发布门槛。详见
[1.0.0 发布契约](docs/roadmap_1_0_0.md)。
本次候选变更见 [rc.2 发布说明](docs/release_notes_1_0_0_rc2.md)。

后台维护提示词使用可移植 Markdown 文件，不再把简化文本内联到 TOML。请从
[momo.example.toml](momo.example.toml) 开始，并阅读
[简体中文配置指南](docs/maintenance_prompts.zh-CN.md)或
[English guide](docs/maintenance_prompts.en.md)。

## Space 标识

一个 Core 实例根目录可以包含许多彼此独立的 Space，它本身不是某个人的 Space。公开
契约使用带职责的 UUID：`personal_space_id`、`conversation_space_id`、记忆源
`space_id` 与 `memory_write_space_id`。检索可以按权重读取多个已授权 Space，但一次后台
维护只能明确写一个 Space。角色卡使用全局唯一 `character_id`；`owner_space_id` 只表示
管理和导出归属，不存在角色目录 UUID。完整规则见
[Space 模型](docs/space_model_1_0.md)与[简体中文宿主指南](docs/spaces_and_controls.zh-CN.md)。

## 验证

角色扮演评测见 [MORP-Bench](benchmarks/morp/README.md)：64 个中英双语反事实场景直接评估
人物一致性、情绪与关系延续、用户主导权、场景具身、主动性、叙事连贯和角色视角。
记忆、检索与压力套件只作为需要显式选择的旧版诊断。Windows 运行
`./scripts/test-morp.ps1`，Linux/macOS 运行 `bash scripts/test-morp.sh`；默认全程离线，
需要模型的候选生成脚本只有显式 `--allow-ai` 才执行。rc.2 推荐只部署一个千问模型
（同时承担 conversation、DMW 提炼与 NSG 治理）和一个向量化模型，三场景 core 矩阵
复用这两个部署。需要先检查调用计划时，可用
`bash scripts/run-morp-model.sh --config <配置文件>`；它默认只生成数据、验证并打印预计调用数，
不访问模型。该入口支持按角色、维度、场景族或 case ID 生成轻量计划。全部主观题需要
一份身份绑定、带候选原文和理由的可审计 reviewer 评审；Codex 可以直接承担该评审，
不再要求两个外部裁判模型。未完成时总分保持 `null`，不冒充零分。真实运行可记录到
[结果表模板](benchmarks/morp/RESULTS_TEMPLATE.md)。Windows PowerShell 也保留对应的 `.ps1` 入口。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

参见[开发指南](docs/development.en.md)与
[角色卡兼容边界](docs/character_card_compatibility.md)。

## 参与方式

Issues 用于反馈可复现问题和具体建议，Pull Request 同样开放；较大改动建议先创建
Issue 讨论。参与前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

Apache License 2.0。参见 [LICENSE](LICENSE) 与 [NOTICE](NOTICE)。
