# 向量化模型接口（0.4.2）

**状态：** Implemented Profile
**更新日期：** 2026-08-25

## 两层接口

0.4.2 明确区分两层，避免把“能调用 OpenAI-compatible 上游”误写成“本地服务本身就是
OpenAI-compatible embeddings 服务端”。

### 上游供应商接口

`OpenAiEmbeddingProvider` 根据
[OpenAI Create embeddings API](https://developers.openai.com/api/reference/resources/embeddings/methods/create/)
调用 `POST /embeddings`：

- 发送字符串数组 `input`、`model` 与 `encoding_format: "float"`；
- 仅在 profile 要求时发送 `dimensions`；
- 要求响应包含 `data[].index`、`data[].embedding` 与 `model`；
- `object`、`data[].object` 和 `usage` 在常见本地兼容服务缺省时可省略，但出现时必须
  分别符合 `list`、`embedding` 以及 `total_tokens >= prompt_tokens`；
- 按 `index` 恢复输入顺序，并验证索引完整唯一、模型、数量、维度、有限值和非零范数；
- 将官方响应的 `usage.prompt_tokens` 与 `usage.total_tokens` 保留在 `EmbeddingBatch`。

OpenAI 文档还允许单字符串、token ID 输入、`base64` 输出与可选 `user`。MOMO 的文本
索引场景只使用字符串数组和浮点输出，因此没有把这些可选形态冒充为已实现能力。

### MOMO 本地编排接口

`POST /v1/embeddings/generate` 是 MOMO 自定义接口。它额外接收供应商端点、确定性
embedding profile、输入 ID 和 `query`/`document` 用途，因此不是标准 OpenAI 请求结构，
也不能仅通过替换 OpenAI SDK 的 base URL 直接调用。

返回结构为 `EmbeddingBatch`：

```json
{
  "vector_space_id": "momo-embedding-v1:<sha256>",
  "model": "text-embedding-model",
  "dimension": 2,
  "vectors": [
    { "id": "node-1", "vector": [0.1, 0.2] }
  ],
  "usage": {
    "prompt_tokens": 8,
    "total_tokens": 8
  }
}
```

`usage` 在上游兼容服务没有返回时省略。调用方配置或输入错误返回 HTTP 400；上游超时
返回 504；上游网络、状态码、响应协议或响应大小错误返回 502；MOMO 内部序列化错误才
返回 500。

NSG 编排接口保持不变：

- `POST /v1/semantic-graph/vectors/rebuild`：全量/增量原子重建；
- `POST /v1/memory/retrieve-scoped`：查询文本向量化后检索；
- `GET /v1/semantic-graph/vector-status?vector_space_id=...`：检查当前 scope 的空间状态。

## 本地策略与上游限制

- MOMO 每批最多 128 条，每条文本最多 1 MiB、整批最多 4 MiB；
- OpenAI 文档的模型 token 上限由具体供应商/模型执行。Core 没有假装能用统一 tokenizer
  在请求前精确验证任意第三方模型；
- 成功响应最大 64 MiB，错误正文最多保留 2000 字节；
- 向量空间身份覆盖供应商/端点逻辑 ID、模型及修订、维度、归一化、用途前缀和
  `dimensions` 行为，不包含 URL 或 API Key；
- API Key 从当前本地请求注入，只转发到上游 Authorization 头，不持久化且调试输出脱敏。

0.4.1 的原始契约和建库设计记录仍保留在
`vectorization_model_interface_0_4_1.md`；本文件是 0.4.2 起的准确接口说明。
