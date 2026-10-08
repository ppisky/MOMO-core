# MOMO 0.5.0 代码审查与完成度

[English](review_0_5_0.en.md)

**审查状态：** 历史预发布记录；已由 Core 1.0 Space 模型取代

**更新时间：** 2026-08-27

本文记录当前实现，不把计划、占位接口或未来能力计入完成度。规范边界以
[`architecture_0_5.md`](architecture_0_5.md) 为准。

> 本文不再是当前接口契约。Core 1.0 不读取 0.5 wire/MOC 格式，也不提供兼容层。

本文保留 MOMO 的实现证据。宿主和模型适配器行说明职责边界，不记录外部产品的完成度
或验证结果。

## 四层边界

| 层 | 只负责 | 明确不负责 | 当前结论 |
| --- | --- | --- | --- |
| MOMO Core | 领域状态、MomoApi、上下文、DMW/NSG/MO State、请求治理、导入导出、MOC、LSB | 应用协议、provider 监听、HTTP 进程治理 | 已对齐 |
| momo-server | MomoApi 的本机 HTTP/SSE、请求体限制、并发、超时、指标和错误映射 | 对话编排、幂等状态、维护提示词、兼容格式推断 | 已对齐 |
| 宿主接入 | 用户/session 映射、本地配置、Core 连接 | 重复的对话或维护流水线 | 外部接入边界 |
| 模型适配器 | 上游协议转换与服务商连接 | MOMO 角色、记忆、状态和持久化语义 | 协议边界 |

适配器可以很薄，甚至主要是 HTTP 转发；其独立价值是外部协议、身份、生命周期和字段映射，
不是复制 Core 功能。

## 完成度矩阵

| 能力 | 实现 | 对外接口 | 生产链路接入 | 测试 | 文档 |
| --- | :---: | :---: | :---: | :---: | :---: |
| 原生 `POST /v1/momo/responses` | ✓ | ✓ | ✓ | ✓ | ✓ |
| SSE 生命周期、工具调用、取消 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 请求 ID 幂等、持久化重放、冲突检测 | ✓ | ✓ | ✓ | ✓ | ✓ |
| DMW/NSG/MO State 与后台维护 | ✓ | ✓ | ✓ | ✓ | ✓ |
| `config.toml` / `momo.toml` 双配置边界 | ✓ | ✓ | ✓ | ✓ | ✓ |
| CLI/请求参数 allow/ignore/reject 治理与审计 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 严格类型化 MOC 导入/导出计划 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 原始外部源字节保留与原样导出 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 显式生成 CCv2/CCv3/CHARX 兼容导出 | ✓ | ✓ | ✓ | ✓ | ✓ |
| MOC v2 snapshot 与兼容 profile | ✓ | ✓ | ✓ | ✓ | ✓ |
| PNG/无损 WebP LSB，单一 typed payload | ✓ | ✓ | ✓ | ✓ | ✓ |
| APNG/animated WebP/JPEG/AVIF 拒绝 | ✓ | ✓ | ✓ | ✓ | ✓ |

## 本轮发现并修正的边界问题

- Server 曾保存响应锁、内存重放缓存和取消标记，实际形成第二个响应状态机；现已全部收回
  `MomoApiService`。
- 已完成请求的锁曾长期残留，取消该 ID 会污染之后的幂等重试；现在只对活动 operation
  标记取消，并通过 RAII 在成功、失败、超时或 future 被丢弃时清理。
- 后台维护的提示词、patch 应用和轮次确认曾位于 Server；现在 Server/Core 调度边界是
  “Server 不再拥有维护语义，Core 根据 portable runtime 配置登记并运行维护”。
- 未使用的 Core `/v1/chat/*` 路由已删除，调用方使用原生响应编排入口。
- Core 内部 raw model completion/stream 与取消函数不再作为公共产品 API；对外使用
  `MomoApiService::execute` 和 `MomoApiService::cancel`。
- Server 响应传输、SSE 编码和 HTTP 错误分别位于独立模块；Core 编排可在不启动 HTTP
  服务的情况下调用。

## 明确不属于 0.5.0 的能力

- 不提供 MOC v1 迁移器、incremental MOC 或删除清单。
- 不支持 APNG、animated WebP、JPEG 或 AVIF 作为 LSB 载体。
- 不在 MOC 载体图片尾部追加第二份信息，也不同时嵌入第二个兼容 payload。
- 视觉图片输入执行链路留到 1.0；0.5 只冻结视觉描述提示词及其覆盖治理边界。
- 不承诺音频、视频、Realtime 或任意第三方扩展之间的无损转换。

## 验证证据与剩余风险

- MOMO Core workspace：160 项测试通过，严格 Clippy `-D warnings` 通过。
- MOMO `contracts/0.5` 的五个 fixture 已通过 SHA-256 校验。
- 当时 MOMO workspace 无 Python 运行时或 Python 测试依赖。

剩余项不再是代码边界缺口：发布前仍应在真实 provider 凭据下做一次本机 Core↔gateway
流式 smoke test；平台权限和应用会话行为属于宿主接入检查。仓库中被忽略的现有本地配置
也可能仍是旧的单文件格式；应由操作者备份后拆成两个 TOML，不能由
发布代码静默迁移或覆盖。
