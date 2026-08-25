# 向量化模型接口审计与 0.4.1 修复计划

**状态：** Confirmed Interface Gap / 0.4.1 Planned
**审计日期：** 2026-08-25

## 结论

0.4.0 的向量能力不是完整的“向量化模型接口”。当前实现只有向量记录、Turso 持久化、
余弦排序与状态检查；文本转向量仍完全依赖宿主。Rust 宿主可以直接调用底层存储接口，
但只使用本地 HTTP API 的调用方无法完成建库闭环。

因此 0.4.0 不得宣称支持 OpenAI-compatible embeddings，也不得把
`NsgVectorStore` 描述成向量化模型适配器。

## 0.4.0 的实际接口

- `NsgVectorStore::upsert_nsg_vectors` 写入调用方已经生成的向量；
- `NsgVectorStore::rank_nsg_vectors` 对调用方已经生成的查询向量进行排序；
- `POST /v1/memory/retrieve-scoped` 可选接收成对的 `vector_space_id` 与
  `query_vector`；
- `GET /v1/semantic-graph/vector-status` 只报告指定空间的索引状态；
- `OpenAiGateway` 只实现 `chat/completions`，没有 `embeddings` 请求；
- HTTP API 没有向量写入、批量生成或 NSG 索引重建入口。

## 已确认的问题

1. 文本查询与浮点向量同时由客户端负责，Core 不能保证索引向量与查询向量来自同一
   模型、维度、归一化规则或模型修订版；
2. `vector_space_id` 是未结构化字符串，调用方可自行声明，不能可靠证明兼容性；
3. HTTP 调用方能够请求向量排序，却不能通过 HTTP 建立该索引；
4. 维度从记录中间接推断，没有独立的模型能力发现或空间注册结果；
5. 当前向量使用 `f64` JSON 边界，尚未明确供应商 `float`/base64 编码、批处理和输入
   上限；
6. NSG 源文档变化后只会表现为 stale/missing，Core 没有负责重新向量化的公开编排。

## 0.4.1 目标契约（草案，不在 0.4.0 实现）

0.4.1 应增加独立的 `EmbeddingProvider`，至少包含批量接口：

```rust
pub trait EmbeddingProvider {
    async fn embed_batch(
        &self,
        profile: &EmbeddingProfile,
        inputs: &[EmbeddingInput],
    ) -> Result<EmbeddingBatch, EmbeddingError>;
}
```

`EmbeddingProfile` 必须结构化记录模型 ID、端点能力、维度、输入用途、编码格式、
归一化规则和模型修订标识。`vector_space_id` 应由这些兼容性字段确定性生成，不能继续由
HTTP 调用方任意拼接；API Key 仍只来自本机安全凭据存储，不进入 ID、日志、TOML 或 MOC。

计划增加的编排能力：

- 文本批量向量化；
- NSG 全量/增量索引重建；
- 查询文本在 Core 内向量化后检索；
- 模型维度与响应数量验证；
- 限流、超时、批大小和部分失败报告；
- 保留低级“传入原始向量”Rust 接口用于测试和高级宿主，但不再把它作为普通 HTTP
  客户端的主路径。

0.4.1 实现前，具体 HTTP 路径与 JSON 字段均未冻结。
