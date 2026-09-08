# DMW-RFC-0014: MO State v2.0 — Autonomous State Runtime

```text
Standard: DMW-RFC-0014                              September 05, 2026
Category: Runtime Specification
Status: Implemented baseline in MOMO Core 1.0
Updates: MO State v1.0
Depends on: DMW v2, NSG v2, MOMO native response runtime
```

## 摘要 (Abstract)

MO State v2 将 MO State 从“读取 DMW 与 NSG 后生成 `[STATE_CONTEXT]` 的只读编译器”扩展为一个长期存在的**自治状态运行时**（MO State Runtime）。它位于宿主与 DMW/NSG 之间，接收用户消息、模型结果、工具结果和维护事件，统一管理场景生命周期、动态记忆、语义网治理、状态一致性与面向模型的状态快照。

MO State v2 的目标产品形态是一个封闭的用户侧系统，例如只向用户暴露聊天界面的 `mobot`：用户只需要开启 MO State 并进行对话，不需要理解或日常操作 DMW、NSG、状态契约、场景文件、权重、索引或修订候选。系统在既定权限和作者权威边界内自行观察、更新、校验、修复和维护。

“自治”不表示让语言模型直接修改文件，也不表示取消用户最终权威。MO State Runtime 拥有的是**协调权和生命周期管理权**；DMW 与 NSG 仍拥有各自的数据语义、校验规则和写入约束。所有模型输出均为不可信提案，只有通过运行时验证的命令才能落盘。

---

## 1. 从 v1 到 v2

### 1.1 v1 已经定义的能力

MO State v1 已经定义：

- 内置默认状态契约；
- 五维状态信号与 `[STATE_CONTEXT]`；
- 打开即进入“人在环外”的产品体验；
- DMW 自动维护与 NSG 自动治理档位；
- 用户保留最终作者权威。

这些要求在 v2 中继续有效。

### 1.2 v1 未定义清楚的主体

v1 将实际职责分散给 MFM、Distiller、NSG 治理流程和只读 MO State 编译器，但没有定义“谁对两个系统的共同状态负责”。因此实现可以满足一次状态编译，却仍然缺少：

- 统一事件入口；
- 跨 DMW/NSG 的操作排序；
- 场景的开启、推进、切换和结束；
- 失败后的恢复与重试；
- 一致的版本与快照；
- 自治运行状态和可观测性。

v2 将这个缺失主体定义为 **MO State Runtime**。

### 1.3 术语

| 术语 | 定义 |
| --- | --- |
| MO State Runtime | 管理 DMW、NSG、场景和状态生命周期的运行时服务 |
| MO State Manager | Runtime 中按 Space 串行处理事件的逻辑管理者 |
| MO State Snapshot | 某个确定版本的 DMW、NSG 与场景所形成的当前状态视图 |
| State Projector | 将已确定快照映射为五维状态和 `[STATE_CONTEXT]` 的确定性组件 |
| Reconciler | 检查并修复 DMW、NSG、场景引用和运行元数据之间的不一致 |
| State Event | 可能影响当前状态的用户、模型、工具、控制面或维护事件 |

“编译”在 v2 中只描述 `State Projector` 的局部工作，不再描述整个 MO State 产品能力。

---

## 2. 封闭自治运行档位 (Closed Autonomous Profile)

### 2.1 “封闭”的含义

本文档中的封闭系统是指：

- 宿主只暴露有限的用户交互面，例如聊天、重置、导出和少量控制；
- Core 是 DMW、NSG 和 MO State 的唯一受信任管理入口；
- 宿主不会在 Core 之外直接修改记忆或语义网；
- 用户不需要运行单独的 DMW/NSG 管理工作流。

“封闭”不等同于离线。模型提供商、向量服务或平台网络可以位于系统之外，但它们不拥有本地状态写入权。

### 2.2 仅开启 MO State 时的规范行为

对于 v2-native 封闭宿主，原生请求设置 `momo.mo_state = true` 时，宿主 MUST 默认采用：

```toml
[mo_state]
profile = "closed_autonomous"
scene_management = true
```

其余运行参数采用以下默认值：

```toml
[mo_state]
profile = "closed_autonomous"
scene_management = true
max_reconcile_steps = 4
max_agent_steps = 8
operation_timeout_ms = 30000
```

DMW 与 NSG 自治开关沿用 `[runtime] memory_distillation_enabled` 与 `semantic_graph_enabled`；默认均开启。状态契约使用内置默认契约，Canon 权威沿用 NSG v2 的 protected 边界，不额外暴露重复配置项。

启用后，系统 MUST：

1. 为目标个人 Space 启动或恢复 MO State Manager；
2. 校验 DMW、NSG、索引、当前场景和未完成操作；
3. 使用内置默认契约，不要求用户创建配置文件；
4. 将所有用户轮次交给 Manager 编排；
5. 自动维护当前场景与 DMW 动态事实；
6. 自动执行 NSG 的安全治理、去重、Draft 和 Revision Candidate 管理；
7. 在每次生成前提供版本一致的 MO State Snapshot；
8. 自动调度可恢复的后台维护；
9. 对无法自动确认的高风险冲突采用保守降级，不阻塞普通聊天；
10. 保留审计与回滚信息，但不要求用户日常查看。

### 2.3 用户可见体验

默认用户路径 SHOULD 只有：

```text
开启 MO State -> 对话 -> 系统自行维护
```

宿主 MAY 提供高级控制面，但 MUST NOT 将下列内容作为正常使用前提：

- 手动编辑 `current/scene.md`；
- 手动整理记忆权重或标签；
- 手动修复索引；
- 逐条批准低风险 DMW 更新；
- 定期清理 NSG Draft 或重复候选；
- 编写 `state_contract.yaml`。

### 2.4 没有用户事件时

自治管理不表示系统应当无条件推进故事。

- 没有用户、工具或显式时钟事件时，Manager MUST NOT 自行创造新的叙事事实；
- 后台维护 MAY 衰减、归档、重建索引、去重和整理候选；
- 后台维护 MUST NOT 把“时间经过”推断为角色行动或场景结果；
- 主动剧情推进需要宿主显式提供受治理的 `clock_tick` 或其他世界事件。

---

## 3. 架构与所有权

### 3.1 组件关系

```text
User / Host Events
        |
        v
MO State Runtime
  |- Event Inbox and Operation Journal
  |- Per-Space MO State Manager
  |- Scene Controller
  |- DMW Controller
  |- NSG Controller
  |- Reconciler
  |- State Projector
  `- Maintenance Scheduler
        |
        +----> DMW v2
        +----> NSG v2
        `----> MO State Snapshot -> AI Harness
```

### 3.2 职责矩阵

| 职责 | 所有者 |
| --- | --- |
| 接收和排序状态事件 | MO State Manager |
| 决定事件应影响 DMW、NSG 还是场景 | MO State Manager |
| 动态事实、经历、关系变化与当前场景存储 | DMW |
| 稳定世界规则、语义关系与 Canon/Draft 边界 | NSG |
| DMW 文件校验、补丁、归档和遗忘执行 | DMW Controller / MFM |
| NSG 节点校验、候选和治理执行 | NSG Controller / MFM |
| 跨系统排序、重试、补偿和一致性 | MO State Reconciler |
| 五维状态与模型指令生成 | State Projector |
| 最终自然语言或工具决策 | Conversation Model / AI Harness |

MO State Manager MAY 向 DMW/NSG 发出写命令，但 MUST NOT 绕过它们的校验器直接修改底层文件。

### 3.3 数据所有权

MO State v2 不建立第三套叙事事实库：

- 已发生的动态事实 MUST 存在 DMW；
- 稳定规则 MUST 存在 NSG；
- 当前场景的可移植表示 MUST 存在 DMW `current/scene.md`；
- MO State Snapshot 是可重建的物化视图，不是新的权威事实源；
- Operation Journal、版本号和租约属于运行控制面，不得注入模型或被 Distiller 当作事实读取。

---

## 4. MO State Event Loop

### 4.1 Loop 的性质

MO State Loop 是一个受限的事件处理循环，而不是要求模型不断自问自答的无限推理循环：

```text
receive event
  -> recover/checkpoint
  -> observe DMW + NSG + scene
  -> classify impact
  -> plan validated commands
  -> apply commands
  -> reconcile
  -> publish snapshot
  -> acknowledge event
```

只有当模型产生工具调用且工具结果会改变可观察世界时，AI Harness MAY 在同一个用户轮次中执行额外的 Observe/Act 步骤。

### 4.2 事件类型

Manager MUST 至少识别：

| 事件 | 典型影响 |
| --- | --- |
| `user_message` | 查询、显式事实、偏好、场景行动或控制意图 |
| `assistant_committed` | 已接受的叙事结果、承诺或场景变化提案 |
| `tool_result` | 外部世界观测或已执行动作的结果 |
| `maintenance_due` | DMW 蒸馏、衰减、归档、索引或 NSG 治理 |
| `control_change` | 用户显式锁定、回滚、删除、批准或策略修改 |
| `clock_tick` | 宿主明确授权的时间或世界事件 |
| `recovery` | 崩溃后继续未完成的操作 |

来自用户文本或模型输出的内容 MUST 先作为证据进入分类过程，不能直接作为底层写命令执行。

### 4.3 每个 Space 单写

- Manager MUST 以目标 Memory Space 为一致性边界；
- 同一 Space 的状态事件 MUST 有稳定顺序；
- 同一 Space 同时最多有一个状态变更提交者；
- 不同 Space MAY 并发处理；
- 共享会话读取多个 Memory Space 时，Manager MUST 记录每个来源的独立版本，不得将其隐式合并成一个所有者。

### 4.4 有界收敛

一次事件 MAY 触发 DMW 更新、NSG 候选和场景更新，但 Reconciler MUST 有界运行：

```text
MAX_RECONCILE_STEPS = 4
```

若在限制内不能收敛，Manager MUST：

1. 保留最后一个已提交的一致快照；
2. 将未完成工作持久化为 pending operation；
3. 标记 `degraded` 并安排重试；
4. 不执行未确认的重复副作用；
5. 在安全时继续普通对话。

---

## 5. 场景是一等运行对象 (First-Class Scene)

### 5.1 场景定义

场景是当前对话正在发生的局部叙事范围。它不是历史日志，也不是完整世界状态。一个有效场景 Snapshot SHOULD 包含：

```yaml
scene:
  scene_id: "scene_opaque_id"
  revision: 7
  status: "active"           # active | transitioning | closed
  location: "客厅"
  timeframe: "深夜"
  participants:
    - "character:momo"
    - "user:owner"
  focus: "讨论明天的出行计划"
  open_threads:
    - "等待用户确认目的地"
  constraints:
    - source: "nsg:rule_quiet_hours"
      text: "不能制造过大噪音"
  epistemic_partitions:
    - subject: "character:momo"
      excludes: ["event:hidden_booking"]
  source_refs:
    - "dmw:event_arrived_home"
    - "nsg:rule_quiet_hours"
```

该结构是逻辑 Schema。可移植文本仍由 DMW `current/scene.md` 表示；实现 MAY 在 SQLite 中缓存结构化投影，但缓存不得成为事实源。

### 5.2 场景生命周期

```text
no scene
  -> active
  -> transitioning
  -> active(new revision or new scene)
  -> closed
```

Manager SHOULD 在以下证据出现时评估场景切换：

- 用户显式改变时间、地点、参与者或目标；
- 已提交的工具结果改变现实环境；
- 已接受的叙事结果结束当前目标；
- 当前场景的全部开放线程已关闭；
- 新事件与当前场景存在明确的时间或空间断裂。

单纯的话题漂移、模型猜测或一次含糊提及 MUST NOT 自动关闭场景。

### 5.3 场景更新规则

场景变更 MUST 同时完成以下逻辑工作：

1. 将已经发生且值得保留的变化记录为 DMW 事件；
2. 用替换语义更新 `current/scene.md`，不得无限追加历史；
3. 移除不再相关的 Hot Memory 引用；
4. 重新验证与场景有关的 NSG 约束；
5. 对可能揭示稳定新规则的变化生成 NSG Revision Candidate，而不是复制动态事实；
6. 发布递增 revision 的新 Snapshot。

### 5.4 场景与五维状态

v1 的 `scene_constraint` 只负责从当前场景与 NSG 提取模型约束。v2 中：

- Scene Controller 负责维护“场景是什么”；
- Reconciler 负责确认 DMW 与 NSG 是否支持该场景；
- State Projector 负责把场景映射为 `[STATE_CONTEXT]`；
- Conversation Model 负责在该约束下生成语言或动作。

模型不得通过输出一段 `[STATE_CONTEXT]` 来修改真实场景。

---

## 6. DMW 与 NSG 的自治管理

### 6.1 事件分类

Manager MUST 按语义归属路由变化：

| 变化 | 目标系统 |
| --- | --- |
| 一次性发生的事件 | DMW |
| 用户偏好、关系变化、承诺及经历 | DMW |
| 当前时间、地点、参与者和开放线程 | DMW current scene |
| 稳定世界规则或长期语义关系 | NSG Draft / Revision Candidate |
| 对现有 Canon 的挑战 | DMW 证据 + NSG Revision Candidate |
| 状态编译和表达约束 | MO State Snapshot，不持久化为事实 |

### 6.2 一致性提交

一个事件同时影响 DMW 与 NSG 时，Manager MUST 使用一个稳定的 `operation_id`：

```text
stage operation
  -> validate DMW commands
  -> validate NSG commands
  -> apply idempotently
  -> reconcile references and versions
  -> publish snapshot
  -> mark operation complete
```

若两个系统可以共享同一事务，MFM SHOULD 原子提交。若不能共享事务，Manager MUST 使用持久化 Saga/Outbox，并保证：

- 每条命令可以安全重放；
- 已成功步骤不会重复产生副作用；
- 部分失败不会发布为完整成功快照；
- 重试或补偿结果可审计。

### 6.3 Canon 权威

`profile = "closed_autonomous"` 默认不取消 NSG v2 的 Canon 保护：

- Manager MAY 自动创建、更新、合并和归档允许范围内的 Draft；
- Manager MAY 自动创建、去重和整理 Revision Candidate；
- Manager MUST NOT 默认覆盖用户确认的 Canon；
- 高风险 Canon 冲突 MUST 被隔离并采用保守解释；
- 普通聊天 MUST NOT 因等待人工处理而永久阻塞。

完全委托 Canon 修改权属于更高风险的独立策略能力，只有宿主与 NSG 规范共同定义明确权限后才能启用；本规范不通过一个布尔开关隐式授予该权限。

### 6.4 自治不等于模型直写

- Distiller、Resolver 和 Conversation Model 只能产生结构化提案；
- Manager 负责选择、排序和提交提案；
- DMW/NSG Controller 负责最终 Schema、权限和不变量校验；
- 非法、越权或引用不存在对象的提案 MUST 被拒绝；
- 被拒绝的提案 MUST NOT 通过自然语言再次尝试绕过控制面。

---

## 7. MO State Snapshot 与 v1 Projector

### 7.1 版本一致快照

每个 Snapshot MUST 至少记录：

```yaml
mo_state_snapshot:
  snapshot_id: "opaque"
  space_id: "uuid"
  dmw_revision: 41
  nsg_revision: 18
  scene_revision: 7
  contract_version: 2
  generated_at: 1788451200
  degraded: false
```

State Projector MUST 从同一 Snapshot 中的输入生成状态，不得在投影过程中重新读取变化中的 DMW/NSG 文件。

### 7.2 五维投影

v1 的以下五维继续保留：

1. `scene_constraint`；
2. `physiological_state`；
3. `epistemic_state`；
4. `relational_stance`；
5. `emotional_tone`。

v1 的确定性规则匹配、显式冲突声明、优先级、Token 预算、输出格式与注入位置继续适用于 Projector，除非未来的 v2 Schema 章节明确覆盖。

### 7.3 何时重新投影

- DMW、NSG、场景或状态契约版本变化后 MUST 重新投影；
- 只追加不改变状态的数据型工具结果 MAY 复用当前 Snapshot；
- 任何改变用户偏好、关系、场景、认知或世界规则的工具结果 MUST 触发重新观察和投影；
- 后台维护完成后 MUST 使旧 Snapshot 失效，但不得修改已完成响应的历史上下文。

---

## 8. 与 AI Harness 和 mobot 的关系

### 8.1 宿主边界

封闭 `mobot` SHOULD 每个普通用户轮次只提交一次高层请求。MO State Runtime 在 Core 内部完成：

```text
user input
  -> state event
  -> reconcile pending work
  -> obtain snapshot
  -> assemble context
  -> model step
  -> commit assistant result
  -> stage scene/memory maintenance
  -> return/stream result
```

`mobot` 不需要分别调用 DMW、NSG 或场景接口，也不需要知道它们的文件布局。

### 8.2 工具循环

如果模型只生成回复，一次用户轮次不需要 Agent Tool Loop。

如果模型调用工具：

1. Harness 将工具调用交给受信任 Executor；
2. Executor 使用 `run_id + call_id` 保证幂等；
3. 工具结果作为 `tool_result` 事件进入 Manager；
4. 若结果影响世界或记忆，Manager 更新并重新投影；
5. Harness 在步数、时间和 Token 预算内继续模型步骤。

因此 Tool Loop 位于 Harness/Turn Runner，MO State Event Loop 位于状态运行时。两者 MAY 协作，但不得合并为一个无法区分责任的无限循环。

### 8.3 外部宿主工具

若工具实现只能存在于 `mobot` 或平台侧，Core MAY 返回 `requires_action` 并持久化暂停点。宿主提交 `function_call_output` 后 MUST 恢复同一 `run_id`，而不是创建一个无法关联的新用户轮次。

---

## 9. 持久化、幂等与恢复

### 9.1 Operation Journal

每个改变状态的事件 MUST 有持久化记录：

```yaml
operation:
  operation_id: "uuid"
  space_id: "uuid"
  event_id: "uuid"
  phase: "staged"       # staged | applying | projected | completed | failed
  attempt: 1
  base_dmw_revision: 41
  base_nsg_revision: 18
  commands_hash: "sha256"
```

Journal 是运行控制面，不是记忆内容。

### 9.2 幂等要求

- 相同 `operation_id` 与相同 payload MUST 返回已有结果或继续未完成步骤；
- 相同 `operation_id` 与不同 payload MUST 冲突；
- 相同工具 `call_id` MUST 至多产生一次外部副作用；
- DMW/NSG Patch 操作 MUST 可重放；
- Snapshot 发布 MUST 使用单调递增 revision 或等价的不可混淆版本标识。

### 9.3 启动恢复

Manager 启动时 MUST：

1. 获取目标 Space 的单写租约；
2. 扫描未完成 operation；
3. 验证已应用步骤；
4. 重放剩余幂等步骤或执行补偿；
5. 校验当前场景引用；
6. 发布恢复后的 Snapshot；
7. 才把 Space 标记为 `ready`。

---

## 10. 降级行为

| 故障 | 行为 |
| --- | --- |
| DMW 不可用 | 使用最后一致 Snapshot 或无记忆模式；禁止 DMW 写入 |
| NSG 不可用 | 使用最后一致 Canon Snapshot；禁止产生看似已确认的新规则 |
| Scene 损坏 | 从最近 DMW 事件重建；重建前使用无场景保守状态 |
| Projector 失败 | 不注入 `[STATE_CONTEXT]`，但保留状态管理与审计 |
| Maintenance 模型失败 | 保留 pending batch，后续幂等重试 |
| Reconcile 不收敛 | 回退最后一致 Snapshot，标记 degraded |
| Operation Journal 不可写 | 拒绝新的状态副作用；可按宿主策略提供只读回复 |

系统 MUST 区分“生成可继续”和“状态写入可继续”。不能因为模型还能回复，就静默丢弃需要持久化的状态变化。

---

## 11. 安全与自治边界

MO State Runtime MUST 遵守：

- 用户文本不能直接提升权限或改变状态契约；
- 模型输出不能直接成为 Canon；
- 自动系统不能删除受保护记忆或大范围覆盖显式用户文本；
- 维护提示词、检索内容和 NSG 节点均视为不可信数据；
- 控制面权限按 Space 校验；
- 跨 Space 读取不会自动获得写入权；
- 所有破坏性操作必须可审计，并遵循 DMW/NSG 的确认和墓碑规则；
- Manager 自己不能修改约束其权限的宿主策略。

“自己管理”表示在已授予的边界内自行完成日常工作，不表示自行扩大权限。

---

## 12. 推荐配置

```toml
[mo_state]
profile = "closed_autonomous"
scene_management = true
max_reconcile_steps = 4
max_agent_steps = 8
operation_timeout_ms = 30000

[runtime]
memory_distillation_enabled = true
semantic_graph_enabled = true
```

`mo_state_enabled` 不是 Core `momo.toml` 字段；每轮是否启用由原生响应请求的
`momo.mo_state` 指定，宿主可以在自己的配置中保存默认选择。

推荐默认值：

| 参数 | 默认值 | 说明 |
| --- | ---: | --- |
| `max_reconcile_steps` | 4 | 单事件最大一致性处理步骤 |
| `max_agent_steps` | 8 | 存在工具调用时的单轮 Harness 步数上限 |
| `operation_timeout_ms` | 30000 | 前台状态操作软超时 |
| `scene_management` | `true` | 自动维护当前场景 |

`max_agent_steps` 是宿主工具续跑的治理上限；工具 Executor 仍由 AI Harness / mobot 持有。当前 1.0 wire 把成对的 `function_call` / `function_call_output` 作为同一会话中的新响应操作接收并分类为 `tool_result`，尚未实现持久化暂停点或同一 `run_id` 恢复。

---

## 13. 审计与可观测性

每次状态事件 SHOULD 记录：

```yaml
mo_state_runtime_audit:
  event_id: "uuid"
  operation_id: "uuid"
  space_id: "uuid"
  event_type: "user_message"
  profile: "closed_autonomous"
  dmw_revision_before: 41
  dmw_revision_after: 42
  nsg_revision_before: 18
  nsg_revision_after: 18
  scene_revision_before: 7
  scene_revision_after: 8
  commands_applied: 2
  reconcile_steps: 1
  snapshot_id: "opaque"
  degraded: false
```

审计内容 MUST NOT 注入模型上下文。普通用户界面 SHOULD 只呈现简洁状态，例如“MO State 正在自动管理”；详细日志属于诊断与高级控制面。

---

## 14. 迁移与实现状态

### 14.1 v1 兼容档位

为避免升级后静默获得新的写入行为，现有 v1 配置迁移到 v2 时 MUST 显式固定：

```toml
[mo_state]
enabled = true
profile = "v1_projection"
```

只有新建 v2-native 封闭宿主，或用户/宿主明确迁移后，才能采用 `closed_autonomous`。

### 14.2 Projector 兼容

v1 `compile_mo_state` MAY 作为 v2 `State Projector` 的初始实现继续使用。迁移不要求立即改变 `[STATE_CONTEXT]` wire 格式。

### 14.3 Core 1.0 已实现的基线

MOMO Core 1.0 当前实现了：

- `closed_autonomous` 与 `v1_projection` 两种运行档位；
- 进程内按 Space 的单写管理锁，不同 Space 可并行；
- DMW、NSG、Scene 三套独立内容指纹与单调 revision；
- SQLite Operation Journal，以及与响应完成原子关联的 `projected -> completed` 提交；
- 持久化、幂等发布和可重放的 MO State Snapshot；
- 一等结构化 Scene Snapshot，以及旧空白 `scene.md` 的无损模板迁移；
- 每轮场景蒸馏、下一轮前的待办恢复，以及原有可恢复 DMW/NSG 维护批次；
- `user_message` 与 `tool_result` 状态事件分类；
- 本机管理状态查询 `GET /v1/mo-state/runtime?space_id=...`；
- 投影失败、维护失败和快照写入失败的显式 degraded 状态与警告。

本实现将“Manager”落实为按请求唤醒、持久状态驱动的逻辑管理者，而不是每个 Space 常驻一个永不退出的线程。没有事件时它不会推进叙事；下次请求会先恢复待维护工作，再观察并发布新快照。

### 14.4 明确保留给宿主的边界

- Core 不执行平台或业务工具；AI Harness / mobot 负责受信任 Executor 与外部副作用幂等；
- Core 接收与先前调用成对的 `function_call_output`，把它作为新响应操作中的 `tool_result`；原请求 ID 不可用不同 payload 续跑；
- `max_agent_steps` 与 `operation_timeout_ms` 当前作为治理和审计配置，完整的宿主 `requires_action` 多步调度协议仍属于后续 wire 版本；
- 当前 Snapshot 只版本化受管写入 Space 的 DMW、NSG 与 Scene；多来源读取结果会保留来源标记进入 RP 上下文，但“每个来源独立版本”尚未写入 Snapshot；
- DMW 与 NSG 各自的维护批次可幂等恢复，但跨两套文件系统共享一个 Saga `operation_id` 的验收场景尚未实现。
- Canon 的最终批准权仍由 NSG v2 与用户控制面持有，不因自治档而放开。

这些边界不影响封闭聊天宿主的默认路径：只要宿主持续使用原生响应接口，Core 会自行管理场景、DMW/NSG 维护、一致版本与状态快照。

---

## 15. 验收场景

### 15.1 只有聊天界面的新用户

用户开启 MO State 后直接聊天。系统使用内置契约，自动建立当前场景、维护 DMW、治理 NSG 并提供状态快照；不得提示用户先创建场景文件或学习两个系统。

### 15.2 场景自然切换

用户明确表示离开当前地点并开始新活动。Manager 记录 DMW 事件、关闭旧场景开放线程或迁移仍有效线程、创建新场景 revision、重新加载相关 NSG 约束，并在下一次生成前发布一致 Snapshot。

### 15.3 新事实挑战 Canon

对话中出现与 Canon 冲突的事件。Manager 将事件记录到 DMW，创建去重的 NSG Revision Candidate，继续按受保护 Canon 或明确的保守冲突状态生成；不得静默覆盖 Canon，也不得要求普通用户立即审核才能继续聊天。

### 15.4 中途崩溃

DMW 更新成功而 NSG 候选尚未写入时进程退出。重启后 Manager 根据同一 `operation_id` 恢复剩余步骤，不重复写 DMW 事件，并且在一致性恢复前不发布虚假的完整 Snapshot。

### 15.5 无事件空闲

用户数日没有打开 `mobot`。系统可以执行衰减、归档与索引维护，但不得自行声称角色完成了行动或场景发生了剧情变化。

---

## 16. 结论

MO State v2 的核心定位是：

```text
MO State 不是 DMW 与 NSG 输出后的一个附加文本块；
MO State Runtime 是负责两个系统共同生命周期的管理者；
State Projector 只是这个管理者内部生成模型状态的一步。
```

对封闭的用户侧 `mobot`，开启 MO State 应当意味着系统进入受治理的自治运行：用户只需要对话，Runtime 自行维护场景、记忆、语义网、一致性和状态快照，同时保留权限、审计、回滚和作者权威边界。
