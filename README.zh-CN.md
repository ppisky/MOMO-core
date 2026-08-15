# MOMO Core 0.3

[English](README.md)

MOMO Core 是面向 AI 角色体验的本地优先 Rust 基础系统。角色数据、会话、长期记忆、
叙事语义、状态编译、可移植容器、加密、模型网关与本地 HTTP 接口都在同一个 workspace
中实现。

## 主要能力

- Character Card v2
- MOC v2 导入与导出
- Dual-Mem Wiki（DMW）长期记忆
- Narrative Semantic Graph（NSG）
- MO State 编译
- 本地 SQLite 持久化
- OpenAI-compatible completion 与流式输出
- capability discovery 与上下文预算
- 向量存储契约与确定性检索

`crates/` 下的 crate 是 MOMO Core 的内部实现模块，不是彼此独立的产品。
`momo-server` 通过仅限本机的 HTTP/SSE 接口提供同一组 Core 能力。

## Workspace

- `momo-core`：编排与面向调用方的 Rust API
- `momo-domain`：共享领域类型
- `momo-storage`：本地持久化与向量存储契约
- `momo-memory`：DMW、NSG、检索与 MO State
- `momo-moc`：MOC 容器
- `momo-crypto`：私有容器加密
- `momo-config`：可移植运行配置
- `momo-server`：本地 HTTP/SSE 接口

## 向量存储

`NsgVectorStore` 定义向量存储边界。MOMO 文档中的 Turso 始终指选作向量数据库后端的
独立数据库库。当前源码同时保留 0.3 测试使用的确定性本地精确排序实现。

## 验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

参见 [Core 0.3 状态](docs/core_progress_0_3.md)与
[开发指南](docs/development.en.md)。

## 参与方式

Issues 用于反馈可复现问题和具体建议。Pull Request 暂时关闭。提交 Issue 前请阅读
[CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

Apache License 2.0。参见 [LICENSE](LICENSE) 与
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。
