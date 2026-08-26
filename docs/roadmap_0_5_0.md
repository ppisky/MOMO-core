# MOMO Core 0.5.0 路线图

**状态：** Release candidate verified（双部署 E2E、官方 SDK、故障矩阵与发布门禁已通过）

**更新日期：** 2026-08-25

0.5.0 的主题不是再增加一组彼此独立的模型接口，而是把现有聊天、向量、DMW、NSG
与后续视觉能力收进同一条可编排的响应链路。完成 0.5.0 后，如果下列 1.0 门槛同时
满足，下一个稳定版本可以直接发布为 1.0.0，不强制经过更多 0.x 版本。

## 1. “单一接口”的准确含义

0.5.0 同时保证两种单一性：

1. 应用层处理一轮对话时只发起一次请求，首选入口是 `POST /v1/responses`；调用方不再
   自己依次请求向量、记忆、语义网和对话模型。
2. 部署只需要配置一个逻辑模型网关 origin 和一套网关凭据。不同任务使用内部路由，
   不要求 GUI、CLI 或 Bot 保存每个真实服务商的地址和密钥。

“单一接口”不表示内部只能产生一次上游 HTTP 请求。Core 可以按开关和能力执行向量
检索、DMW/NSG 读取、MO State 编译、最终生成，并在成功回合后调度后台提炼。内部
fan-out、重试和降级必须对调用方透明，但要通过事件、错误和指标可观察。

兼容入口可以不止一个 URL：`/v1/chat/completions` 与 `/v1/messages` 是协议适配面，
它们必须归一化到同一套内部请求、事件和编排服务，不能形成三份业务实现。原有
`/v1/chat/*`、`/v1/embeddings/generate`、`/v1/memory/*` 和
`/v1/semantic-graph/*` 作为管理、调试和精细控制接口保留，不再要求普通聊天客户端
自己拼装它们。

## 2. 项目职责

### MOMO Core

- 定义唯一的内部 `MomoResponseRequest`、`MomoResponseEvent`、用量、错误与取消语义；
- 持有角色、会话、DMW、NSG、MO State、上下文预算和本地持久化规则；
- 按任务路由 `conversation`、`embedding`、`memory_distillation`、
  `semantic_graph_governance`，为 1.0 预留 `vision`；
- 提供 OpenAI Chat Completions、OpenAI Responses 和 Anthropic Messages 的客户端适配；
- 提供 OpenAI-compatible、Responses 与 Anthropic-compatible 的服务端兼容入口，并让
  这些入口共用同一编排实现；
- 定义并实现 MOMO LSB 图片载体。

### mobot

- 作为一个独立的模型网关进程/运行模式，保存服务商、真实模型、凭据引用和任务路由；
- 在一个 origin 下实现 Core 要求的 Responses、Chat Completions、Messages 与
  Embeddings 契约，并翻译到真实服务商；
- Discord/CLI 作为应用适配器时，只向 Core 发起一次高层响应请求；
- 网关子请求不得回流到 Core 的高层响应入口，避免递归调用；
- 通过共享的黑盒契约样例和集成测试适配 MOMO，而不是让 Core 猜测 mobot 的私有行为。

Core 仍允许直接连接单个上游，便于独立部署；推荐的多模型部署由 mobot 提供统一
origin。双方通过 HTTP/SSE 契约协作，不要求互相链接 Rust crate。

## 3. 0.4.2 已有基线

以下能力是 0.5.0 可以复用的基础，不按“从零开发”估算：

| 能力 | 当前状态 | 0.5.0 缺口 |
| --- | --- | --- |
| OpenAI-compatible Chat Completions 出站 | 已有非流式、SSE 解码、取消和基础参数保护 | 只抽取文本 choice；缺工具、usage、完整错误与协议抽象 |
| OpenAI-compatible Embeddings 出站 | 已对齐请求/响应元数据、维度、usage、上限和向量空间身份 | 需要纳入统一 provider/route registry |
| Core 本地 HTTP/SSE | 已有 chat、embedding、memory、NSG、MO State 与 context 低级端点 | 普通客户端仍需多次调用；没有标准 `/v1/responses` 服务端入口 |
| capability discovery | 已有模型窗口、tokenizer、streaming 与参数白名单 | 缺 protocol、modalities、tools 与多类输出能力 |
| mobot 多模型配置 | 已有 provider、chat/embedding category 和 chat/memory/NSG/embedding 用途选择 | 只有宽泛 `openai_compatible`；没有 Responses、Anthropic 或服务端网关 |
| mobot 一轮流水线 | 已能串联 Core 低级接口、向量和后台 DMW/NSG 维护 | 流水线在宿主侧重复；需要收敛为一次 Core 高层请求 |
| 外部图片/角色容器 | 已导入 PNG/APNG metadata、CHARX 与 CHARX-JPEG | 没有 LSB 编解码和 MOMO LSB profile |

因此 0.5.0 的主要工作是抽象、收口与补齐协议，不是重写 DMW、NSG、MO State、Turso
向量库或角色卡兼容层。

## 4. 0.5.0 TODO

### P0：统一请求和事件

- [x] 建立规范化内容块：文本、图片引用、工具调用、工具结果和拒绝；0.5.0 至少完整实现
  文本，图片类型先冻结结构，实际视觉路由留到 1.0。
- [x] 建立统一的非流式结果与流事件，保留 request ID、单调 sequence、finish/stop
  reason、上游 request ID、模型路由和分项 token usage。
- [x] 统一取消、超时、限流、上游 4xx/5xx、协议错误和本地编排错误；兼容层只负责映射
  状态码与错误 envelope。
- [x] 为任务路由使用稳定别名，外部调用方不依赖真实模型 ID。禁止把 embedding 模型
  当作生成模型，也禁止把视觉能力仅靠模型名猜出来。
- [x] 一轮请求幂等：重复的客户端 request ID 不得重复写入用户消息或重复调度维护任务。

### P0：协议适配

- [x] OpenAI Chat Completions：非流式、SSE、工具调用、usage、finish reason、错误格式和
  `[DONE]` 完整覆盖；替换当前只读取文本 choice 的实现。
- [x] OpenAI Responses：支持 `input`/`instructions`、文本输出、结构化 output item、
  流事件、工具调用、usage、取消；不能假设 `output[0]` 一定是文本消息。
- [x] Anthropic Messages：支持顶层 `system`、content blocks、工具调用、非流式与流式
  事件，并正确映射 stop reason 与 usage。
- [x] Anthropic 出站适配器从已解析路由补齐服务商要求的字段：`model`、未显式指定时的
  `max_tokens`（路由默认值，初始建议 1024）以及可配置的 `anthropic-version`
  （初始默认 `2023-06-01`）；认证使用 `x-api-key`。不得伪造缺失的用户输入。
- [x] 不默认发送 `temperature`、`top_p`、`top_k` 等可选采样参数；只转发调用方明确设置
  且 capability profile 允许的字段，避免把某家协议的默认值泄漏给另一家。
- [x] Embeddings 继续使用独立的标准 `/v1/embeddings` 操作，但属于同一个网关 origin
  和路由注册表；保留 0.4.2 的向量空间身份、维度和响应校验。
- [x] 为三种生成协议建立同一组 golden fixtures、分块 SSE 测试和跨协议语义测试。

### P0：单次响应编排

- [x] `POST /v1/responses` 成为首选高层入口；MOMO 的角色、会话、scope 和功能开关放入
  明确版本化的 `momo` 扩展对象，标准字段保持 OpenAI Responses 含义。
- [x] 同一服务完成：输入持久化 → 条件向量化 → DMW/NSG 检索 → MO State → 上下文预算
  → 最终生成 → 助手消息持久化。
- [x] DMW 提炼与 NSG 治理在成功回合后进入后台任务，不拖慢前台完成事件；失败可重试且
  不得把普通对话自动提升为 NSG Canon。
- [x] 定义每个子任务的 required/optional 策略。最终对话模型失败则本轮失败；可选
  embedding、记忆或语义信号不可用时按配置降级并报告 warning。
- [x] 逐步弃用“客户端自己依次调用多个低级端点”的推荐用法，但 0.5.0 不删除现有接口。

### P1：LSB 主载体

- [x] 发布独立的 `MOMO LSB Carrier v1` 文档，并明确声明它是 MOMO 扩展，不冒充
  Character Card、CHARX 或其他外部标准。
- [x] 使用同一张角色图片承载数据，不追加 JPEG/PNG 尾部文件，也不要求再附加一张
  预览图。默认只使用 RGB 通道的 1 个最低有效位，保持 alpha 不变。
- [x] 载荷头至少包含 magic、版本、flags、长度、载荷类型和完整性校验；容量不足时明确
  拒绝，不静默提高每通道位数或缩放图片。
- [x] 基线载荷应是压缩后的 MOMO 角色数据；完整 MOC/CHARX 只有在图片容量足够且调用方
  明确选择时才允许嵌入，不能假装所有资产都能塞进一张普通头像。
- [x] 编解码只依赖解码后的像素顺序，不依赖元数据、PNG chunk 或 IEND/EOI 之后的尾部。
- [x] 增加“删除全部图片尾部字节后仍可解码”和“无损重封装且像素样本不变后仍可解码”
  的回归测试；同时声明任何缩放、有损重压缩、滤镜或像素改写都可能破坏 LSB。
- [x] PNG chunk、标准 CHARX、CHARX-JPEG 和已有导入能力继续作为兼容路径；LSB 是 MOMO
  推荐的图片传播路径，不等于删除文件型交换格式。

### P1：安全、配置与可观察性

- [x] 密钥只存在于服务端环境/凭据存储，不进入请求日志、事件、MOC、LSB 或
  `vector_space_id`。
- [x] 对输入体、图片、单个事件、累计流、工具参数和上游错误正文分别设置上限。
- [x] 统一超时、有限重试、断路与取消传播；有本地写入的操作不得盲目自动重放。
- [x] 指标按逻辑任务和路由别名聚合，真实供应商信息仅在受控诊断输出中出现。
- [x] capability discovery 增加 protocol、modalities、tools、streaming、context window、
  output limit 与允许参数，不再只有 Chat Completions 视角。

## 5. 明确不进入 0.5.0

- 视觉模型的实际推理与图片理解；0.5.0 只冻结多模态内容块和路由位，1.0 实现；
- 音频、实时语音、视频模型；
- 自动负载均衡、按价格择优、跨供应商静默故障转移；先保证显式路由和可诊断失败；
- 宣称所有 OpenAI/Anthropic 扩展字段都可无损互转。无法表达的字段必须拒绝或报告
  compatibility warning，不能静默丢弃。

## 6. 0.5.0 发布条件

- [x] 普通客户端只调用一次 `/v1/responses` 就能完成含记忆与 NSG 的一轮对话；
- [x] Core 独立直连与经 mobot 单一 origin 两种部署都有端到端测试；
- [x] Chat Completions、Responses、Messages 的非流式、流式、取消、工具与错误测试通过；
- [x] embedding 不再由 mobot 绕开 Core 的向量 profile 和身份规则；
- [x] LSB 编解码、容量拒绝、尾部剥离和像素无损重封装测试通过；
- [x] 旧 0.4.2 客户端迁移说明、兼容期和弃用项写清楚；
- [x] 全 workspace fmt、clippy、test、doc 与跨仓契约测试通过。

发布候选的可重复验证入口：

- `cargo fmt --all -- --check`、workspace clippy/test/doc；
- `contracts/0.5/SHA256SUMS` 与 CI checksum 校验；
- `scripts/cross_repo_e2e.py` 启动真实 Core、真实 mobot gateway 和 mock 上游，覆盖 Core
  直连、单一 origin、DMW/NSG 检索、持久化和 request ID 重放；
- mobot `scripts/sdk_smoke.py` 使用官方 OpenAI 与 Anthropic Python SDK 验证四个公开入口。

## 7. 直接进入 1.0.0 的门槛

0.5.0 发布后，只有以下条件都满足才直接进入 1.0.0：

- 0.5 的公共请求、事件、路由、LSB 载荷头和错误语义不再需要破坏性修改；
- 完成真实视觉模型路由，并让图片输入通过同一 `/v1/responses` 编排链路；
- 至少一个 OpenAI Responses 上游、一个 Anthropic Messages 上游和一个 embedding 上游
  通过真实集成测试；
- mobot 与一个非 mobot 客户端都能只依赖公开契约接入；
- 数据迁移、备份恢复、并发、取消、长流与安全上限完成发布级验证。

如果上述任一项仍会改变公共协议，应继续发布 0.5.x，而不是为了版本号提前承诺 1.0。

## 8. 资料与来源声明

- OpenAI Responses 字段和事件以
  [OpenAI 官方 Create a model response 文档](https://developers.openai.com/api/reference/resources/responses/methods/create/)
  为准。
- Anthropic Messages 的 `max_tokens`、顶层 `system`、content blocks、请求头与流事件以
  [Anthropic 官方 Create a Message 文档](https://platform.claude.com/docs/en/api/messages/create)
  为准。
- [cc-switch（固定提交 `9a596158`）](https://github.com/farion1231/cc-switch/tree/9a596158ca926e74b56243c08af67d9dd13fc27c)
  仅作为本地路由、协议转换、故障处理与可观察性方面的设计参考；该提交的
  [LICENSE](https://github.com/farion1231/cc-switch/blob/9a596158ca926e74b56243c08af67d9dd13fc27c/LICENSE)
  是 MIT。截至本文档，本项目未复制或翻译其代码。如果以后复制实质代码，必须在
  `NOTICE` 中记录固定提交、文件和 MIT 版权声明。
