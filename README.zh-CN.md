# MOMO Core

[English](README.md)

MOMO Core 是面向 AI 角色体验的本地优先 Rust 基础系统。角色数据、会话、长期记忆、
叙事语义、状态编译、可移植容器、加密、模型网关与本地 HTTP 接口都在同一个 workspace
中实现。

## 主要能力

- MOMO 独立 Character Card v2（`character.toml` + Markdown）
- Character Card v1/v2 JSON 与 PNG 导入
- Character Card v3 JSON、PNG/APNG 与 CHARX 导入
- 保留来源字段的 Character Card v2/v3 JSON 导出
- MOC v2 导入与导出
- Dual-Mem Wiki（DMW）长期记忆
- Narrative Semantic Graph（NSG）
- MO State 编译
- SQLite 业务存储与独立 Turso 向量存储
- OpenAI-compatible completion 与流式输出
- capability discovery 与上下文预算
- 向量存储契约与确定性检索

`crates/` 下的 crate 是 MOMO Core 的内部实现模块，不是彼此独立的产品。
`momo-server` 通过仅限本机的 HTTP/SSE 接口提供同一组 Core 能力。

## 角色卡格式边界

MOMO Character Card v2 是由本仓库定义的独立角色卡格式，规范见
[`Character_Card_v2.md`](Character_Card_v2.md)。其中的“v2”不表示外部生态的
`chara_card_v2` JSON/PNG 格式。当前 Core 实现的是 MOMO 格式随 MOC v2 的导入导出；
同时支持外部 CCv1/v2 JSON、PNG，以及 CCv3 JSON、PNG/APNG、CHARX 的导入，并支持
CCv2/CCv3 JSON 导出。未映射的外部字段作为来源元数据保留，也会随 MOC 往返。

外部格式兼容设计的规范来源固定为
[Character Card v2](https://github.com/malfoyslastname/character-card-spec-v2) 与
[Character Card v3](https://github.com/kwaroran/character-card-spec-v3)。来源快照、术语映射
和当前实现状态见[角色卡格式与兼容边界](docs/character_card_compatibility.md)。

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

DMW 与 NSG 的 YAML/Markdown 源文档仍位于 `memory/scopes/<scope_id>`，是记忆与语义图
的可移植事实来源。Turso 中的向量按来源哈希和向量空间校验，是可从源文档重新生成的
缓存，不进入 MOC。`NsgVectorStore` 只是隔离 Turso 实现细节的内部接口，不代表第三套
数据库。升级到 0.3.2 时，旧 SQLite `nsg_vectors` 表会被删除，宿主应按需重建向量缓存。

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
