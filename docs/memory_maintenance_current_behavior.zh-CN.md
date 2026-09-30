# 记忆维护、来源与参数：当前代码定义

**核对日期：** 2026-09-30
**范围：** 当前 MOMO Core 与本机 HTTP 管理接口；不是宿主 UI 的功能清单。

本文记录当前工作树行为。身份、逐修订来源、共享与场景分区已接入，具体字段与新增管理
接口见 [来源运行时 Profile 1](memory_provenance_runtime.md)；生命周期见
[运行时生命周期 Profile](memory_lifecycle_runtime.md)。

## 1. “谁产生”需要拆开记录

“用户定义”“MO State 调度”“NSG 治理器修正”可能发生在同一条规则的不同阶段，不能只用
一个互斥的来源类型表示。例如用户在对话中提出规则，自动治理生成 Draft，用户批准其后续
修订：原始证据是用户陈述，提案者是治理器，执行者是 Core，批准者是用户。

| 路径 | 当前职责与入口 | 来源追踪的实际边界 |
| --- | --- | --- |
| 用户/宿主手工编辑 NSG | 本机节点写入、授权 Patch、候选批准/拒绝接口 | 来源文件逐修订保留提案、批准与父版本；缺少原始材料时保持未知，不把操作者补成事实作者 |
| 自动 NSG 治理 | `semantic_graph_governance` 路由读取待处理轮次及已有记忆，提出 Patch | 自动创建要求 Draft；对 Canon 的自动改动转候选，不能把模型当成作者批准者 |
| MO State 运行时 | 状态观察、投影、快照、一致性与维护恢复协调 | 当前投影器不是独立的 NSG 事实或 Canon 生成源；调度背景不能替代原始证据 |
| NSG Revision Candidate | 候选保存 `reason`、`suggested_changes`、`source_evidence`，管理接口批准/拒绝 | 原有 `source_evidence` 仍是说明文本；来源文件另存可信批次证据与提案历史，批准后沿用其证据和父版本 |

权限需与运行频率分开：MO State 自动运行，不要求用户逐轮确认；NSG 可以自动新增 Draft。
进入 Revision Candidate 的修正由用户通过控制面确认后应用，不能因为自动调度而自动批准。
当前代码还允许普通 Draft 内容在自动 Patch 路径更新；这与批准正式规则的修订不同。
自动 Patch 将 Draft 提升为 Canon 现在也会生成待确认候选，不能通过修改元数据绕过批准。
用户也可直接通过管理入口添加节点。不能概括成“NSG 每次改动都要确认”，也不能写成
“MO State 自动，所以它调度的 NSG 修正也自动获批”。

手工调用“立即提炼”仍然是模型提出记忆，不能登记成“用户亲自撰写了全部事实”。相反，用户
批准模型候选，也不应覆盖候选最初的证据来源。具体目标字段语义见设计文档第 4.1 节。

代码依据：[NSG 管理入口](../crates/momo-core/src/api/runtime_api/nsg.rs)、
[NSG Patch 与候选处理](../crates/momo-memory/src/nsg.rs)、
[维护编排](../crates/momo-core/src/orchestration/maintenance.rs)、
[状态编排](../crates/momo-core/src/orchestration/state.rs)。

## 2. 自动提炼的“12 轮”

`memory_distill_every_turns` 与 `nsg_govern_every_turns` 默认均为 **12**，可分别设置为 1–200。
该值同时决定正常自动执行所需的最少待处理数量，以及新批次读取的最大轮数。

一轮指维护队列中的一个 `MaintenanceTurn`：包含一次已完成响应操作对应的用户输入文本和
助手输出文本。普通聊天里就是一组问答，12 组通常对应 24 条聊天消息，不能解释成 12 条
消息即 6 组问答。原生响应仅在有非空输出文本且指定可写记忆模块时入队；工具调用步骤和
消息条数不能直接当作维护轮数。可信本机 replay 接口也可以显式录入一轮。

队列按 **写入 Space + 维护种类** 读取未完成记录，按 `created_at, request_id` 排序。
当前还按捕获的用户/对话/角色/连续情境/功能身份分组；不同对话或角色的轮次不会混成同一身份。
DMW 与 NSG 使用独立的 `memory_done` / `nsg_done` 标记。

在无积压、无失败、无手工排空、阈值不变时，序号示意为：

| 新完成的维护轮 | 自动处理 |
| --- | --- |
| 1–11 | 不足 12，等待 |
| 第 12 轮完成 | 处理 1–12，提交成功后标记该维护通道已完成 |
| 13–23 | 等下一批 |
| 第 24 轮完成 | 处理 13–24 |
| 第 36 轮完成 | 处理 25–36 |

当前实现是未处理队列，并非模型上下文滑动窗口；不依赖旧消息是否被挤出前台上下文。
已有记忆仍可被检索作为维护参考，因此旧的记忆事实可以再次出现；已确认处理的旧轮次不会
作为下一批新证据重复消费。输出是结构化记忆/规则 Patch，也可以为空，并非必须写一段摘要。

自动任务在响应完成后调度；前台忙碌时可能推迟，`closed_autonomous` 的下一请求会尝试恢复
达到阈值的待办。每次 `maintain` 通常处理一个批次，不能保证在第 12 轮完成瞬间立即完成。
失败会保留原批次待重试；DMW 成功不代表 NSG 也成功。改阈值只影响后续新批次，持久化的
待恢复批次继续按原记录集合完成。

关闭 `runtime.memory_distillation_enabled` 或 `runtime.semantic_graph_enabled` 会跳过
对应自动调度与恢复，但入队资格由响应写目标的 `memory` / `semantic_graph` 决定。
因此关闭自动维护不等于清空队列，也不等于禁止手工排空。省略 `memory_write_space_id`
不会自动把新响应送入个人 Space 的提炼队列；状态观察使用的个人 Space 回退不是写入授权。

代码依据：[默认值与范围](../crates/momo-core/src/governance.rs)、
[完成响应并入队](../crates/momo-core/src/orchestration/execution.rs)、
[批次读取与确认](../crates/momo-storage/src/local/maintenance.rs)、
[批次执行](../crates/momo-core/src/orchestration/maintenance.rs)。

### 三条处理路径的频率

| 路径 | 当前触发与作用 |
| --- | --- |
| MO State | 启用时在每次响应的生成前自动投影状态；自治档还观察并持久化快照。不等待 12 轮，不等于每轮都由模型改写 `scene.md` |
| DMW 提炼 | 默认累计 12 个未处理轮次，结合已有相关记忆提出增量 Patch，包含需要的场景更新；可无修改 |
| NSG 治理 | 独立累计默认 12 个未处理轮次，提议新增 Draft 或修正等；可无修改，候选批准另走用户控制面 |

12 轮是新证据批次，不是重新处理全部历史、重写整个 DMW/NSG，或一次统一执行三个系统。
两种批次阈值可以不同，成功标记也独立。

### “移出旧两轮”属于上下文窗口

用户澄清的六轮指清除旧的六轮上下文，不是只提炼六轮。本说明将上下文清除与持久删除
分开：移出发给模型的历史不删除数据库消息、长期记忆或待处理证据。

当前 `history_window` 默认在 12 个完整问答后移出最旧 2 轮，随后每增加 2 轮再次淘汰。
策略可关闭或调整，token 预算仍是最终上限。移出只影响发送给模型的历史副本；维护队列、
持久消息和失败恢复游标不受影响。

代码依据：[状态编排](../crates/momo-core/src/orchestration/state.rs)、
[上下文预算裁剪](../crates/momo-core/src/context.rs)。

## 3. 手工提炼与生命周期维护是不同操作

| 操作 | 当前行为 |
| --- | --- |
| `POST /v1/momo/maintenance/drain`，请求 `{"space_id":"<UUID>"}` | 处理该 Space 已入队的 DMW/NSG 待办，包括不足 12 轮的尾批；使用模型 |
| `POST /v1/memory/maintenance`，同样传 Space | 处理持久化的待办交互事件，执行轮次驱动的衰减/归档/遗忘；不调用模型，不凭空增加轮次 |
| 直接编辑记忆、节点或提交 Patch | 控制面修改内容；不是自动重读全部聊天记录 |

`drain` 先处理 DMW，再处理 NSG；每种最多 64 批，每批上限取该种类的配置值，默认 12，
并受服务端响应超时限制。它不会自动回溯未入队的历史对话，也不是任意选择起止消息的
“重新提炼所选对话”接口。失败不返回完成，重试只继续未确认的工作。
该接口目前是本机管理/评测能力，不属于稳定 1.0 用户响应协议；宿主可据此设计受控入口。

代码依据：[手工排空与生命周期路由](../crates/momo-server/src/routes/memory.rs)。

## 4. 哪些参数现在能改

| 项目 | 当前值/行为 | 修改入口 |
| --- | --- | --- |
| 自动 DMW 提炼 | 默认开启，每 12 个待处理轮次一批 | `/v1/runtime-settings` 的 `runtime.memory_distillation_enabled`、`memory_distill_every_turns`，轮数 1–200 |
| 自动 NSG 治理 | 默认开启，每 12 个待处理轮次一批 | 同资源的 `semantic_graph_enabled`、`nsg_govern_every_turns`，轮数 1–200 |
| MO State | 默认 `closed_autonomous`、场景管理开启、注入 `active` | 同资源的 `mo_state`；部分限制目前仅治理/审计，见 MO State 实现边界 |
| 默认助手/提炼器/治理器提示词 | 进程级具名槽位 | `/v1/prompt-spaces/:id`，持久覆盖值 |
| DMW 衰减间隔与倍率 | 默认 48 个后续未命中交互，乘 0.9 | `mo_state.memory_lifecycle.decay_after_turns` / `decay_factor` |
| DMW 自动归档 | 活跃 event 在衰减时低于 weight 0.2，importance < 0.8，且无保护 | 生命周期自动执行 |
| DMW 自动遗忘 | 默认归档后 240 个后续未命中交互，并满足低权重等条件 | `forget_after_turns` / `auto_forget`；`enabled` 控制新生命周期事件 |

轮次驱动自动遗忘检查：

1. 记录已在归档区，类型为 `event`；
2. `importance < 0.2` 且 `weight < 0.05`；
3. 每个已登记使用情境都从归档或最后命中基线推进至少 `forget_after_turns` 个已完成交互；
4. 未被长期记忆的 `relations` 引用；
5. 未被长期正文、`current/scene.md` 或 `current/active_threads.md` 的显式 `[[id]]` 引用，且无承诺/待办保护标签。

MO State 在合格响应完成后自动处理生命周期事件，并在后续自治请求前恢复待办；它还承担
场景/状态、版本、快照、维护协调与恢复职责，不只是投影器。计数键为 Space/对话/助手，
闲置或其他对话不推进这里的计数；仅注入不刷新命中。未知旧记忆仅在匹配显式绑定或实质
命中后登记，不把旧时间戳转换成轮次。共享记忆要求全部已登记情境满足间隔，保护闲置情境。

自动生命周期只处理非核心 event，关系/角色/世界和 current 记录不自动淡化。承诺保护依赖
引用或 `commitment`、`promise`、`open`、`pending`、`unresolved` 标签，不声称能识别所有正文语义。
衰减/归档/遗忘与轮次文件、索引、墓碑和审计共同生成持久文件计划，恢复不重复计数。
参数随事件冻结，修改配置作用于新事件。轮数范围 1–1,000,000，倍率严格介于 0 和 1。

低层 Rust `run_maintenance[_at]` 保留旧日历算法供显式兼容调用；原生自动和 HTTP 维护入口
已改用轮次算法。手工归档只处理指定记录，不再附带整个空间的日历扫描。

代码依据：[生命周期判断](../crates/momo-memory/src/lifecycle.rs)、
[持久恢复](../crates/momo-core/src/recovery.rs)、
[管理维护入口](../crates/momo-core/src/api/runtime_api/nsg.rs)。

## 5. 删除对话后保留什么

**已提炼的 DMW 与 NSG 保留。** `delete_conversation` 不调用 `clear_memory`；记忆仍可按
既有读取配置使用。删除对话也不会自动重新提炼、撤回事实或重置遗忘时间。

当前删除走本地墓碑/删除快照机制：移除活动对话及其消息，但本地删除快照仍可能保存内容。
这不是“所有副本都已擦除”。响应证据现在保存实际对话、消息和发生时角色的关联；删除
后查询标为 `source_deleted`，不会把删除快照当作活动可读证据。待提炼队列默认仍保留，
停止来源提炼与撤销派生事实通过 `/v1/memory/evidence/control` 独立操作。

清除某 Space 的 DMW/NSG 使用独立 `clear_memory`，会清理选定模块及相应派生维护状态；
它不是“只删除某一对话贡献的记忆”。按单个来源撤销使用独立证据控制；存在其他支持的记录不会被整个 Space 清空。

代码依据：[控制操作](../crates/momo-core/src/api/runtime_api/control.rs)、
[删除快照](../crates/momo-storage/src/local/deletion.rs)、
[维护存储](../crates/momo-storage/src/local/maintenance.rs)。

## 6. 默认助手提示词的准确含义

这里指配置默认助手的通用系统提示词：`PUT /v1/prompt-spaces/assistant`，不是要求用户在
对话中途编辑角色卡。两者是不同的配置入口，不应混写成同一种操作。

当前只有在请求没有获准的 `instructions`，且所选角色的 `character_markdown` 为空时，
响应才使用 `assistant` 槽位。非空角色卡会抑制这一回退；获准的请求指令使用自身内容。
槽位是进程级资源，覆盖值持久保存，后续读取生效；`DELETE` 恢复内置默认值。
它不创建新身份、不删除记忆，也不等于给某个用户单独修改角色卡。

代码依据：[实际选择分支](../crates/momo-core/src/orchestration/response.rs)、
[Prompt Spaces](maintenance_prompts.zh-CN.md)。
