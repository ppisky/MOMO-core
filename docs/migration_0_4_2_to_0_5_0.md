# 从 MOMO Core 0.4.2 升级到 0.5.0

0.5.0 保留 0.4.2 的角色、会话、消息、memory、semantic-graph、MO State、chat 和
embeddings 低级端点；兼容期覆盖整个 0.5.x。普通对话的新推荐入口是一次调用
`POST /v1/responses`，旧的“客户端自行串联多个低级端点”进入弃用期，但 0.5.x 不删除。

升级步骤：

1. 备份数据目录并重新构建 `momo-server`；启动时会自动应用 response operation 与后台
   maintenance turn 的新增迁移。
2. 将客户端的普通对话改到 `/v1/responses`，并在 `momo.schema` 发送
   `momo.responses/0.5`。可重试请求应复用同一个 `momo.request_id`。
3. 多供应商部署只给 Core 配置 mobot gateway 的一个 origin 和一套凭据；Core 直连单个
   Chat-compatible 上游仍受支持。
4. 流客户端应按 `type` 分派事件，并使用单调 `sequence` 排序。工具调用会产生
   `response.output_item.added` 和 `response.function_call_arguments.delta`；不要假设输出
   第一项必定是文本。
5. 显式取消流式响应可调用 `POST /v1/responses/{request_id}/cancel`。旧
   `/v1/chat/cancel/{request_id}` 在 0.5.x 继续保留。
6. 错误现在统一为 `error.type/code/message/retryable/upstream_status/request_id` envelope；
   流式失败使用相同对象。调用方不应再解析自由文本错误。
7. `/v1/responses` 请求体上限为 2 MiB，文本输入为 1 MiB，单个 SSE 事件为 1 MiB，
   累计流为 64 MiB，工具 schema/参数/结果各有独立上限。0.5 只冻结图片引用结构，实际
   图片输入会明确返回不支持，视觉路由留到 1.0。
8. 可通过 `MOMO_RESPONSE_MAX_CONCURRENCY` 与 `MOMO_RESPONSE_TIMEOUT_SECONDS` 调整高层响应
   并发和总超时；`GET /v1/metrics` 只按逻辑路由公开聚合指标。

MOMO LSB Carrier v1 是新增的 MOMO 扩展，不替代 Character Card、CHARX 或 MOC。任何
缩放、有损重压缩、滤镜和像素改写都可能破坏载荷；PNG 无损重封装与元数据/尾部变化不
影响按解码后 RGB 像素承载的载荷。
