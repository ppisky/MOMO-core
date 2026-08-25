# 向量化模型接口（0.4.1）

**状态：** Implemented Profile
**更新日期：** 2026-08-25

## 结论

0.4.1 补齐了文本到向量、NSG 索引重建和查询文本向量化的闭环。低级原始向量接口仍然
保留给 Rust 宿主和高级调用方，但普通 HTTP 调用方不再需要自行生成
`query_vector` 或手工声明向量空间。

## Rust 接口

向量供应商通过独立接口接入：

```rust
pub trait EmbeddingProvider {
    fn embed_batch(
        &self,
        profile: &EmbeddingProfile,
        inputs: &[EmbeddingInput],
    ) -> impl Future<Output = Result<EmbeddingBatch, EmbeddingError>> + Send;
}
```

内置 `OpenAiEmbeddingProvider` 调用 OpenAI-compatible `POST /embeddings`。
`EmbeddingProfile` 固定以下兼容性字段：

- `provider_id` 与 `endpoint_id`；
- `model` 与可选 `model_revision`；
- `dimension`；
- `normalization`（`l2` 或 `none`）；
- 是否向供应商发送 `dimensions`；
- 查询和文档的可选前缀。

Core 对这些字段的规范 JSON 做 SHA-256，生成
`momo-embedding-v1:<digest>` 形式的 `vector_space_id`。端点 URL 和 API Key 不参与
空间标识，避免部署位置或凭据变化破坏向量兼容性。

## HTTP 接口

### 生成向量

`POST /v1/embeddings/generate`

```json
{
  "embedding": {
    "endpoint": {
      "base_url": "http://127.0.0.1:8080/v1",
      "api_key": "optional"
    },
    "profile": {
      "provider_id": "local-openai-compatible",
      "endpoint_id": "primary",
      "model": "text-embedding-model",
      "dimension": 1024,
      "normalization": "l2",
      "send_dimensions": true
    }
  },
  "inputs": [
    { "id": "node-1", "text": "source text", "purpose": "document" }
  ]
}
```

返回值包含确定性的 `vector_space_id`、模型、维度以及按输入 ID 恢复顺序后的向量。

### 重建 NSG 向量索引

`POST /v1/semantic-graph/vectors/rebuild`

请求包含同样的 `embedding` 配置以及 `mode`；目标 scope 由当前本地服务实例的
`scope_id` 决定：

- `full`：重新向量化该 scope 的全部 NSG 节点；
- `incremental`：复用相同空间内 `node_id`、`source_hash` 和维度均匹配的记录，只生成
  已新增或已变化的节点。

所有批次成功后，Core 才会在一个事务中替换该 scope 与向量空间的快照。任一上游请求
失败时，旧快照保持不变；已从 NSG 删除的节点不会残留在新快照中。

### 使用查询文本检索

`POST /v1/memory/retrieve-scoped` 可传 `embedding` 配置。Core 会把请求中的 `query`
按 `query` 用途向量化一次，并把得到的空间 ID 和查询向量用于所有目标 scope。
`embedding` 不能与低级 `vector_space_id`/`query_vector` 同时出现。

低级原始向量路径继续受同模型空间、维度、有限值和非零向量检查约束。

### 查看索引状态

`GET /v1/semantic-graph/vector-status?vector_space_id=...`

目标 scope 同样来自当前本地服务实例。响应明确回显 `vector_space_id`，并报告记录数、
维度和 missing/stale 节点。

## 验证与限制

- 单批最多 128 条输入；
- 单条文本最大 1 MiB，单批文本最大 4 MiB；
- 供应商成功响应最大 64 MiB，错误响应最多保留 2000 字节；
- 向量维度范围为 1–8192；
- 请求超时范围为 1–600 秒，索引批大小范围为 1–128；
- 输入 ID 必须唯一；供应商响应索引必须完整且唯一；
- 返回数量、维度、模型、有限值和非零范数均经过校验；
- `l2` 模式在 Core 内统一归一化，避免供应商默认行为差异。

Core 从当前本地请求接收 API Key，并只把它转发到供应商请求的 `Authorization` 头；
调试输出会将其脱敏。它不进入 `vector_space_id`、向量记录、MOC 或持久化配置。调用方
仍应从本机安全凭据来源注入，而不是把密钥提交到仓库或可移植文档中。
