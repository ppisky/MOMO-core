# MOMO Chat Runtime v1

**状态：** Implementation Baseline  
**更新日期：** 2026-08-10

本文记录当前聊天运行时的确定性行为，避免客户端与 Rust Core 分别猜测上下文、
取消和本地持久化语义。

## 1. 请求路径

```text
用户输入
  → Rust Core 生成消息 UUIDv7
  → SQLite message 本地写入
  → Rust Core 编排角色与历史上下文
  → OpenAI 兼容模型端点
  → 客户端桥接流事件
  → 客户端增量渲染
```

消息数据只以本地 SQLite 和 `.moc` 导入导出为主线。模型端点不可用时，
本地历史不会丢失；不存在后台 outbox、账号同步或服务端重试队列。

## 2. 上下文顺序与预算

请求消息顺序固定为：

1. 角色卡 `character_markdown` 与 `user_markdown`；
2. 以最新用户消息检索到的 DMW 记忆和一跳关联；
3. 在预算内保留的最近历史消息。

记忆检索拥有独立上限，当前为上下文窗口的八分之一，并限制在 256–2048 Token。没有命中或记忆文件暂时不可读时，聊天继续执行，不允许让补充记忆成为模型请求的单点故障。

输入预算为：

```text
context_window - reserve_output_tokens - 128 safety margin
```

已知模型可以通过 capability profile 选择精确 tokenizer；未知模型使用供应商无关的保守估算：ASCII 约四字符一个 Token，非 ASCII 字符按一个 Token 计，并为每条消息增加固定结构开销。保守估算只用于避免明显溢出，不代表供应商计费 Token。超出预算时从最早消息开始省略，客户端必须显示省略数量。

## 3. 流事件

Rust Core 的普通 Rust 适配 API 向调用方输出 UTF-8 JSON 字符串。GUI、CLI、TUI、
本地应用、服务端接口或自动化脚本都可以通过自己的适配层消费这些事件：

```json
{
  "type": "delta",
  "request_id": "UUIDv7",
  "sequence": 1,
  "delta": "增量文本",
  "finish_reason": null
}
```

终止事件为 `done` 或 `cancelled`。同一请求的 `sequence` 单调递增。SSE 解码不得假设 TCP 分块与 UTF-8 字符或 SSE 事件边界一致。

用户停止生成时，客户端以 `request_id` 调用取消接口。Rust Core 停止继续消费并发送 `cancelled`；不完整助手文本只用于临时显示，不写入正式消息表。

## 4. 消息幂等性

客户端为消息生成 UUIDv7。同一 ID 重复写入时，只有会话、角色、创建时间等
不可变字段完全一致才视为幂等；不可变字段不同必须返回本地冲突错误，避免一条
消息被静默改写成另一条消息。允许编辑的部分是消息正文。

删除是本地最近删除语义：角色、会话、消息删除后写入本地 tombstone 和恢复快照；
恢复只恢复本地对象，不产生任何远端删除记录。

当前流式代码在客户端取消、Rust 取消标记或客户端流接收失败时停止读取上游，
但尚无长响应/慢消费者压力测试，不能证明桥接队列始终有界。应用重启后继续
同一次生成可以作为增强项，但不属于当前 Rust Core 主线。
