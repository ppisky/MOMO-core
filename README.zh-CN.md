# MOMO Core

[English](README.md)

MOMO Core 是面向 AI 角色体验的本地优先 Rust 基础系统。角色数据、会话、长期记忆、
叙事语义、状态编译、可移植容器、加密、模型网关与本地 HTTP 接口都在同一个 workspace
中实现。

## 主要能力

- MOMO 独立 Character Card v2（`character.toml` + Markdown）
- Character Card v1/v2 JSON 与 PNG 导入
- Character Card v3 JSON、PNG/APNG 与完整 CHARX 容器导入
- 保留来源字段的 Character Card v2/v3 JSON 与 CHARX 导出
- MOC v2 导入与导出
- Dual-Mem Wiki（DMW）长期记忆
- Narrative Semantic Graph（NSG）
- MO State 编译
- SQLite 业务存储与独立 Turso 向量存储
- 面向上游的 OpenAI-compatible completion、流式输出与 embeddings 网关
- capability discovery 与上下文预算
- 向量存储契约与确定性检索

`crates/` 下的 crate 是 MOMO Core 的内部实现模块，不是彼此独立的产品。
`momo-server` 通过仅限本机的 HTTP/SSE 接口提供同一组 Core 能力。

## 角色卡格式边界

MOMO Character Card v2 是由本仓库定义的独立角色卡格式，规范见
[`Character_Card_v2.md`](Character_Card_v2.md)。其中的“v2”不表示外部生态的
`chara_card_v2` JSON/PNG 格式。当前 Core 实现的是 MOMO 格式随 MOC v2 的导入导出；
同时支持外部 CCv1/v2 JSON、PNG，以及 CCv3 JSON、PNG/APNG、CHARX 的导入，并支持
CCv2/CCv3 JSON 与 CHARX 导出。CHARX 的资产、`x_meta`、`module.risum` 和未知安全条目
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
- `momo-memory`：DMW、NSG、检索与 MO State
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

DMW 与 NSG 的 YAML/Markdown 源文档仍位于 `memory/scopes/<scope_id>`，是记忆与语义图
的可移植事实来源。Turso 中的向量按来源哈希和向量空间校验，是可从源文档重新生成的
缓存，不进入 MOC。`NsgVectorStore` 只是隔离 Turso 实现细节的内部接口，不代表第三套
数据库。升级到 0.3.2 时，旧 SQLite `nsg_vectors` 表会被删除，宿主应按需重建向量缓存。

0.4.2 已将上游 embeddings 适配器对齐到 OpenAI 文档中的请求/响应元数据，保留 token
usage，并让本地生成接口正确返回 400/502/504。该本地接口仍是 MOMO 编排 API，不是可由
OpenAI SDK 直接替换 base URL 的服务端接口。原始向量接口继续作为低级能力保留。详见
[向量化模型接口规范](docs/vectorization_model_interface_0_4_2.md)。

0.5.0 已加入版本化响应契约与 `POST /v1/responses`：一次请求完成消息持久化、
DMW/NSG 检索、MO State、上下文预算、逻辑路由和助手持久化，并支持真正的上游增量 SSE、
跨进程 request ID 幂等、Core 所有的 embedding profile，以及可重试的后台 DMW/NSG 维护。
三协议工具调用已有共享 golden contract 和异协议增量转换；
[MOMO LSB Carrier v1](docs/momo_lsb_carrier_v1.md) 已接入有界 PNG codec，并覆盖尾部剥离与
无损重封装。0.5.0 发布候选还统一了错误 envelope、请求与流上限、取消/超时/限流、
逻辑路由指标，以及 Core 直连和经 mobot 的真实进程 E2E。兼容变化和 1.0 门槛见
[迁移说明](docs/migration_0_4_2_to_0_5_0.md)与[0.5.0 路线图](docs/roadmap_0_5_0.md)。

## Scope 标识

`scope_id` 是公开领域模型、API、存储、向量记录、Patch Review 与 MOC 操作使用的
唯一命名空间标识。Scope 是一个不透明 UUID，其业务含义和访问策略由宿主应用决定。
Core 将每个记忆 workspace 存储在 `memory/scopes/<scope_id>` 下。

## 验证

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
