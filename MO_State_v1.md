Internal Request for Comments: DMW-RFC-0014             August 08, 2026
Category: Implementation Guide                         Status: Draft
Updates: DMW-RFC-0010, DMW-RFC-0013

DMW-RFC-0014: MO State v1.0 — Five-Dimensional State Compiler
Implementation Specification

## 摘要 (Abstract)

本文档定义 MO State v1.0 的实现规范。MO State 是位于 DMW（动态事实层）与 NSG（语义网 / 静态法则层）之上的自动状态编译与运行配置层。开启 MO State 后，宿主应用应把当前叙事状态、关系姿态、场景约束、角色认知与表达约束自动编译为结构化行为约束指令（`[STATE_CONTEXT]`），并将 DMW 与 NSG 的日常维护切换为自动治理模式。

MO State 的产品承诺是：**打开即进入环外体验**。这里的“环外”指人不需要日常维护状态，也不需要为了维持语义网可用而手动整理规则。用户仍保留最终作者权威与显式控制权，但默认运行路径不要求用户维护 DMW、NSG 或状态契约。

本文档不改变 DMW 与 NSG 的核心架构边界：

- DMW 仍为动态叙事记忆，由 Distiller 自动维护；
- NSG 仍为半静态、具备作者权威边界的世界设定层；在 MO State 开启时，其日常整理、候选生成与低风险更新默认由宿主应用自动执行；
- MFM 仍为唯一执行者；
- 检索、衰减、归档、遗忘规则不变。

本文档的核心内容包括：

1. 定义内置默认状态契约与可选 `state_contract.yaml` 覆写的 Schema 与校验规则；
2. 定义五维信号的精确提取协议；
3. 定义确定性编译流水线与冲突仲裁规则；
4. 定义 `[STATE_CONTEXT]` 的注入位置、Token 预算与格式；
5. 定义降级路径、审计日志与风险控制。

本文档为 Draft，不视为最终冻结规范。

---

## 1. 设计目标 (Design Goals)

MO State v1.0 的设计目标：

1. **环外体验优先。** 用户打开 MO State 后，MFM MUST 使用内置默认契约运行；用户不需要创建、编辑或定期维护 `state_contract.yaml` 才能获得状态约束。
2. **自动治理默认开启。** MO State 开启时，DMW 后台维护与 NSG 语义网治理 SHOULD 默认处于自动模式；人工只处理显式改写、策略变更、高风险冲突或用户主动介入。
3. **声明式编译，非命令式执行。** MO State 不编写伪代码或执行逻辑，只将已检索到的事实、法则与契约规则映射为约束。
4. **确定性可复现。** 给定相同的 DMW 检索结果、NSG 检索结果与有效状态契约，编译结果 MUST 完全一致。
5. **不引入新的记忆存储。** MO State 编译器不创建、修改或删除任何 DMW 或 NSG 文件。
6. **不引入 LLM 概率推理。** 编译流水线 MUST 为确定性规则匹配，不依赖模型猜测。
7. **不改变 MFM 检索主流程。** MO State 作为检索完成后的后处理阶段执行。

### 1.1 环外运行定义 (Out-of-the-Loop Runtime)

MO State 的“人在环外”是运行体验要求，不是权限放弃：

- 用户 MUST NOT 被要求维护权重、标签、场景文件、状态规则或语义网节点，才能让 MO State 正常工作；
- MFM SHOULD 在后台自动执行 DMW 维护、NSG 候选整理、低风险语义网更新与状态编译；
- 用户 MAY 随时进入控制面手动改写、锁定、禁用或回滚规则；
- 对会改变作者意图、世界观核心设定、删除大量记忆或覆盖显式用户文本的操作，宿主应用 MUST 保留人工确认或可审计回滚机制。

因此，`state_contract.yaml` 是高级覆写入口，不是必需维护物。NSG 的人工权威原则在 MO State 中表现为“用户拥有最终权威”，而不是“用户必须持续手动维护语义网”。

---

## 2. 架构位置与接口 (Architecture Position)

### 2.1 在 MFM 流水线中的位置

MO State 编译在 DMW/NSG 检索完成、上下文组装之前执行：

```
User Message
     ↓
[DMW Retrieval v2 — RFC-0010 Step 1-9]
     ↓
[NSG Retrieval v2 — RFC-0013 Step 1-7]
     ↓
[MO State Compilation]  ← 本文档定义
     ↓
[Context Assembly & Token Budget]
     ↓
[LLM Generation]
```

### 2.2 输入

MO State 编译器的输入 MUST 仅包含以下已确定的数据：

| 输入源            | 具体内容                                                          | 来源              |
| -------------- | ------------------------------------------------------------- | --------------- |
| DMW 关系文件       | 本轮被检索加载的 `relationships/` 文件的 `weight`、`tags`                 | RFC-0010 检索结果   |
| DMW 事件文件       | 本轮被检索加载的 `events/` 文件的 `weight`、`tags`                         | RFC-0010 检索结果   |
| DMW Hot Memory | `current/scene.md` 的结构化标签（若有）                                 | RFC-0010 Step 1 |
| NSG 节点         | 本轮被检索加载的 `.nsg` 节点的 `@CONDITION`、`@CONSEQUENCE`、`@CONSTRAINT` | RFC-0013 检索结果   |
| 状态契约           | 内置默认契约与可选 `state_contract.yaml` 覆写                         | 本文档 §3          |

MO State MUST NOT 自行读取未被本轮检索加载的 DMW 或 NSG 文件。

### 2.3 输出

MO State 的输出为一段结构化 `[STATE_CONTEXT]` 文本，注入 LLM 上下文（见 §8）。

MO State MUST NOT：

- 修改任何 DMW 文件的 Frontmatter 或正文；
- 修改任何 `.nsg` 文件；
- 修改用户提供的 `state_contract.yaml`；
- 修改检索排序或 Token 预算分配；
- 写入 `memory_activity.yaml`、`anchor_index.yaml` 或 `relation_index.yaml`。

说明：以上限制约束的是 MO State 编译器本身。若 MO State 作为产品开关启用自动维护，实际写入仍 MUST 由 MFM、Distiller 或 NSG 治理流程按各自 RFC 执行，MO State 不直接越权写入。

### 2.4 与 DMW/NSG 的职责边界

| 职责                 | 归属                          |
| ------------------ | --------------------------- |
| 记录"谁在何时做了什么"       | DMW                         |
| 记录"世界通常如何运行"       | NSG                         |
| 将当前事实与法则编译为行为约束    | MO State 编译器                |
| 执行写入、归档、遗忘         | MFM                         |
| 提议叙事变化             | Distiller                   |
| 自动整理语义网候选、校验与低风险更新 | NSG 治理流程（MO State 开启时默认自动） |

MO State 编译器是只读编译器。它消费 DMW 与 NSG 的检索结果，不产生新的持久化状态。

MO State 产品开关是运行配置入口。它 MAY 同时启用 DMW 自动维护与 NSG 自动治理，但这些写入仍由对应子系统负责，必须保留审计记录与回滚边界。

---

## 3. 状态契约 Schema (State Contract Schema)

### 3.1 契约来源与维护责任

MO State MUST 支持内置默认状态契约。该默认契约由宿主应用随版本发布，用户无需创建任何文件即可启用 MO State。

宿主应用 MAY 允许高级用户提供可选覆写文件：

```
/memory/
 └── config/
     └── state_contract.yaml
```

若存在 `state_contract.yaml`，它 MUST 位于 `/memory/config/` 目录下，与 `access.yaml` 同级。

加载顺序：

1. MFM MUST 先加载内置默认契约；
2. 若 `state_contract.yaml` 存在且有效，MFM MUST 将其作为用户覆写层应用；
3. 若覆写层缺失、解析失败或局部无效，MFM MUST 回退到内置默认契约的对应部分；
4. MFM MUST NOT 要求用户维护覆写文件才能运行 MO State。

`state_contract.yaml` 的定位是“可选高级配置”，不是 MO State 的日常维护入口。

### 3.2 顶层结构

```yaml
version: 1
dimensions:
  relational_stance: { ... }
  emotional_tone: { ... }
  scene_constraint: { ... }
  physiological_state: { ... }
  epistemic_state: { ... }
conflict_priority:
  - "scene_constraint"
  - "physiological_state"
  - "epistemic_state"
  - "relational_stance"
  - "emotional_tone"
```

规则：

- `version` MUST 为整数。当前版本为 `1`。
- 有效契约 MUST 包含 `dimensions`。若用户覆写层缺失任一维度，MFM MUST 使用内置默认契约中的对应维度。
- 若内置默认契约缺失任一维度，MFM MUST 视为实现错误并写入审计日志；该维度不产生指令，但不得阻塞生成。
- `conflict_priority` MUST 为五维度的全排列。缺失时，MFM MUST 使用上述默认顺序。

### 3.3 维度通用结构

除 `scene_constraint` 外，每个维度 MUST 包含：

```yaml
{dimension_key}:
  signal_source: "{dmw|nsg|dmw+nsg}"
  match_mode: "first"          # 可选，"first" 或 "accumulate"，默认 "first"
  rules:
    - id: "{unique_rule_id}"
      condition: { ... }
      directives:
        - "{directive_text}"
```

规则：

- `id` MUST 在有效状态契约内唯一。
- `condition` 的结构因维度而异，见 §4 各维度定义。
- `directives` MUST 为非空字符串数组。
- 同一维度内，规则按数组顺序评估。
- `match_mode: "first"`（默认）：首个命中规则生效，后续规则跳过。
- `match_mode: "accumulate"`：所有命中规则的 `directives` 合并输出。
- `scene_constraint` MAY 使用自动提取模式，不声明 `rules` 数组，见 §4.3。

### 3.4 Schema 校验规则

MFM MUST 在加载内置默认契约与可选覆写层时执行以下校验：

| 校验项                   | 失败行为                   |
| --------------------- | ---------------------- |
| 内置默认契约缺失或解析失败        | 跳过 MO State 编译，写入审计日志  |
| 用户覆写层 YAML 解析失败        | 忽略覆写层，使用内置默认契约，写入审计日志 |
| `version` 缺失或非整数      | 视为 `version: 1`，写入审计日志 |
| 覆写层维度键缺失              | 使用内置默认契约中的对应维度         |
| `rules` 为空数组          | 该维度无输出                 |
| `id` 重复               | 保留首个，忽略后续，写入审计日志       |
| `directives` 为空       | 该规则视为无效，跳过             |
| `condition` 结构不符合维度定义 | 该规则视为无效，跳过，写入审计日志      |

MFM MUST NOT 因用户覆写层校验失败而终止整轮检索或 LLM 生成。只有内置默认契约不可用时，MO State 编译才整体跳过。

---

## 4. 五维信号提取与匹配 (Signal Extraction & Matching)

### 4.1 关系姿态 (Relational Stance)

**信号源：** 本轮被检索加载的 DMW `relationships/` 文件。

**提取字段：**

- `weight`（Frontmatter 动态权重）
- `tags`（Frontmatter 标签数组）

**条件结构：**

```yaml
condition:
  weight_min: 0.30
  weight_max: 0.85
  tags_any: ["dependency", "shared_trauma"]
  tags_all: ["conflict"]
```

**匹配规则：**

1. 若同时指定 `weight_min` 与 `weight_max`，MUST 同时满足。
2. `tags_any` 与 `tags_all` 若同时存在，MUST 同时满足。
3. 若 `condition` 中无任何字段，该规则视为无条件命中。
4. 比较 MUST 使用数值 `>=` / `<=`，不使用浮点容差。

**未加载关系文件时的行为：**

若本轮检索未加载任何 `relationships/` 文件，关系姿态维度 MUST 不产生任何指令。

### 4.2 情绪基调 (Emotional Tone)

**信号源：** 本轮被检索加载的 DMW `events/` 文件中 `weight` 最高的前 `EMOTION_SIGNAL_WINDOW` 个文件。

**提取字段：**

- `tags`（Frontmatter 标签数组）

**条件结构：**

```yaml
condition:
  signal_tags_any: ["farewell", "loss", "unresolved_regret"]
  signal_tags_all: []
```

**匹配规则：**

1. MFM 从本轮已加载的 `events/` 文件中，按 `weight` 降序取前 `EMOTION_SIGNAL_WINDOW` 个。
2. 合并这些文件的 `tags` 为信号标签集合。
3. 若信号标签集合与 `signal_tags_any` 有交集，该规则命中。
4. 若 `signal_tags_all` 非空，信号标签集合 MUST 包含所有指定标签。

默认值：

```
EMOTION_SIGNAL_WINDOW = 3
```

**未加载事件文件时的行为：**

若本轮检索未加载任何 `events/` 文件，情绪基调维度 MUST 不产生任何指令。

### 4.3 场景约束 (Scene Constraint)

**信号源：** DMW `current/scene.md` 与本轮被检索加载的 NSG 节点。

**提取字段：**

- `current/scene.md` 中 `## Environment` 或 `## 环境` 小节的内容（若存在）；
- NSG 节点的 `@CONSTRAINT` 与 `@CONSEQUENCE` 字段。

**条件结构：**

场景约束维度不使用规则匹配，而是直接提取。其 `condition` 仅为占位：

```yaml
condition:
  source: "auto"
```

**匹配规则：**

1. MFM 提取 `current/scene.md` 中的环境描述段落。
2. MFM 提取本轮已加载、且由 NSG 检索阶段标记为 active/context-relevant 的节点中的 `@CONSTRAINT`。
3. 合并为场景约束指令。

该维度无 `rules` 数组。若 `current/scene.md` 无环境描述且无 NSG 场景约束被加载，该维度不产生指令。

### 4.4 生理状态 (Physiological State)

**信号源：** DMW `events/` 的 `tags` 交叉匹配 NSG 节点的 `@CONDITION` 与 `@CONSEQUENCE`。

**条件结构：**

```yaml
condition:
  dmw_event_tag: "critical_wound_torso"
  nsg_constraint_match: "no_healing_magic_applied"
  nsg_consequence_ref: "lore_mana_instability"
```

**匹配规则：**

1. MFM 在本轮已加载的 `events/` 文件中查找 `tags` 包含 `dmw_event_tag` 的文件。
2. 若命中，MFM 在本轮已加载的 NSG 节点中查找 `@CONDITION` 或 `@CONSTRAINT` 文本包含 `nsg_constraint_match` 短语的节点。
3. 短语匹配 MUST 为确定性子串匹配（Unicode NFKC 规范化后），不使用语义或 LLM 判断。
4. 若 `nsg_consequence_ref` 非空，MFM 还须验证存在 `id` 匹配的 NSG 节点被加载。
5. 以上条件全部满足时，该规则命中。

**若 DMW 事件命中但 NSG 约束未命中：**

该规则 MUST NOT 生效。生理约束需要 DMW 事实与 NSG 法则同时存在。

**若 NSG 约束命中但 DMW 事件未命中：**

该规则 MUST NOT 生效。无起因则无后果。

### 4.5 认知掩码 (Epistemic State)

**信号源：** DMW `events/` 的 `tags` 与 `relations` 字段。

**条件结构：**

```yaml
condition:
  mode: "absence"
  event_tag: "secret_meeting"
  required_witness_tag: "witness"
  character_ref: "char_xiaohong"
```

**匹配规则（absence 模式）：**

1. MFM 在本轮已加载的 `events/` 文件中查找 `tags` 包含 `event_tag` 的文件。
2. 若找到，检查该文件的 `tags` 或正文中是否包含 `required_witness_tag` 且关联到 `character_ref`。
3. 若未找到关联，该规则命中（角色不在场）。
4. 若本轮未加载任何包含 `event_tag` 的文件，该规则 MUST NOT 命中。MFM 不得因"未加载"推断"不在场"。

**匹配规则（misconception 模式）：**

```yaml
condition:
  mode: "misconception"
  character_ref: "char_xiaohong"
```

1. MFM 在本轮已加载的 DMW 文件中查找 `tags` 包含 `misconception` 且关联到目标角色的文件。
2. 若找到且未被后续事件标记为 `corrected`，该规则命中。

**计算边界：**

认知掩码仅基于本轮已加载文件进行判断。MFM MUST NOT 为认知掩码单独执行全库遍历。若所需信息未被本轮检索加载，该规则不生效。

### 4.6 泛化标签保护

与 DMW v2 §4.2 和 NSG v2 §5.2 一致，MO State 的标签匹配 MUST 遵循泛化词保护：

- 单字符标签 MUST NOT 单独触发规则命中。
- 状态契约中的 `tags_any` / `signal_tags_any` 若仅包含泛化词，该规则 SHOULD 被视为无效。
- 宿主应用 MAY 在有效状态契约中声明 `generic_tags` 列表。

---

## 5. 编译流水线 (Compilation Pipeline)

MFM 在每轮 LLM 生成前执行以下确定性流水线：

```
[Step 1] 加载内置默认契约与可选 state_contract.yaml 覆写
     ↓
[Step 2] 信号提取（从本轮已加载的 DMW/NSG 检索结果中提取字段）
     ↓
[Step 3] 逐维度规则匹配
     ↓
[Step 4] 维度内冲突处理（First-Match-Wins 或 Accumulate）
     ↓
[Step 5] 维度间冲突仲裁
     ↓
[Step 6] 指令生成与格式化
     ↓
[Step 7] Token 预算检查与裁剪
     ↓
输出 [STATE_CONTEXT]
```

### 5.1 Step 1：加载契约

MFM 先加载宿主应用内置默认契约，再尝试读取 `/memory/config/state_contract.yaml` 作为用户覆写层。

若覆写文件不存在：

- MO State MUST 使用内置默认契约继续编译；
- MFM MAY 写入低优先级审计信息：`contract_source: "builtin_default"`；
- 本轮 LLM 生成正常继续，并可注入 `[STATE_CONTEXT]`。

若覆写文件解析失败：

- MFM MUST 忽略覆写层；
- MFM MUST 使用内置默认契约继续编译；
- MFM MUST 写入审计日志。

只有内置默认契约缺失或解析失败时：

- MO State 编译跳过；
- MFM MUST 写入审计日志；
- 本轮 LLM 生成正常继续，无 `[STATE_CONTEXT]` 注入。

### 5.2 Step 2：信号提取

MFM 从本轮检索结果中提取 §4 定义的各维度信号。

MFM MUST NOT 在 Step 2 中触发额外的文件读取或检索。

### 5.3 Step 3–4：规则匹配

MFM 按有效状态契约中 `dimensions` 的声明顺序逐维度执行规则匹配。

每个维度内：

- 若 `match_mode: "first"`（默认），首个命中规则生效，后续跳过。
- 若 `match_mode: "accumulate"`，所有命中规则的 `directives` 合并。

### 5.4 Step 5：维度间冲突仲裁

当多个维度的 `directives` 存在语义冲突时，MFM MUST 按 `conflict_priority` 从高到低保留高优先级维度的指令。

默认优先级：

```
scene_constraint > physiological_state > epistemic_state > relational_stance > emotional_tone
```

冲突判定规则：

- 若两个维度的指令包含明确的动作矛盾（如"大声呼喊" vs "禁止核心发力"），高优先级维度的指令保留，低优先级维度的矛盾指令 MUST 被降级为兼容表述。
- 降级表述由有效状态契约中的 `conflict_resolution` 字段预定义（可选）。若未预定义，MFM MUST 直接丢弃低优先级矛盾指令。

**维度内冲突：**

若同一维度内 `match_mode: "accumulate"` 导致多条规则命中且指令矛盾：

- MFM MUST 按规则数组顺序保留先出现的指令；
- 后出现的矛盾指令 MUST 被丢弃；
- MFM SHOULD 写入审计日志。

### 5.5 Step 6：指令生成

MFM 将存活的 `directives` 按维度分组，编译为 `[STATE_CONTEXT]` 格式文本（见 §8）。

### 5.6 Step 7：Token 预算检查

若 `[STATE_CONTEXT]` 超出预算（见 §8.2），MFM MUST 按以下顺序裁剪：

1. 移除 `emotional_tone` 维度的指令；
2. 移除 `relational_stance` 维度的指令；
3. 移除 `epistemic_state` 维度的指令；
4. 移除 `physiological_state` 维度的指令；
5. `scene_constraint` MUST NOT 被裁剪。

MFM MUST NOT 在单条 directive 中间截断。

---

## 6. 降级与错误处理 (Degradation & Error Handling)

| 故障场景                            | MFM 行为                    |
| ------------------------------- | ------------------------- |
| 用户覆写 `state_contract.yaml` 不存在  | 使用内置默认契约，正常编译             |
| 用户覆写 `state_contract.yaml` YAML 解析失败 | 忽略覆写层，使用内置默认契约，写入审计日志    |
| 内置默认契约缺失或解析失败                  | 跳过 MO State，正常生成，写入审计日志    |
| 本轮 DMW 检索结果为空                   | 依赖 DMW 的维度不产生指令           |
| 本轮 NSG 检索结果为空                   | 依赖 NSG 的维度不产生指令           |
| 单条规则 `condition` 非法             | 跳过该规则，写入审计日志              |
| 维度间冲突无法解析                       | 保留高优先级维度，丢弃低优先级矛盾指令       |
| `[STATE_CONTEXT]` 超出 Token 预算   | 按 §5.6 裁剪                 |

MFM MUST NOT 因 MO State 编译失败而阻塞 LLM 生成。

开启 MO State 后，宿主应用 SHOULD 在 UI 或控制面呈现“无需维护 / 自动运行”的状态，而不是要求用户补齐配置文件。配置缺失、覆写失败或局部降级都应被视为后台可审计事件。

---

## 7. 与 DMW / NSG 的交互约束 (Interaction Constraints)

### 7.1 MO State 编译器对 DMW 的约束

- MO State MUST NOT 修改 DMW 文件。
- MO State MUST NOT 触发额外的 DMW 检索。
- MO State MUST NOT 影响 `touch_at`、`weight`、`decay_at` 的更新。
- MO State MUST NOT 影响归档或遗忘判断。

### 7.2 MO State 编译器对 NSG 的约束

- MO State MUST NOT 修改 `.nsg` 文件。
- MO State MUST NOT 触发额外的 NSG 检索。
- MO State MUST NOT 影响 Canon / Draft 状态。
- MO State MUST NOT 影响 Revision Candidate。

### 7.3 MO State 编译器对 Distiller 的约束

- Distiller MUST NOT 读取或写入 `state_contract.yaml`。
- Distiller MUST NOT 在 YAML Patch 中包含 MO State 指令。
- MO State 的 `directives` 不得被 Distiller 当作记忆事实存储。

### 7.4 MO State 开关的自动运行档位

当宿主应用提供“开启 MO State”开关时，该开关 SHOULD 同时设置以下运行档位：

```yaml
mo_state_enabled: true
dmw_maintenance: "auto"
nsg_governance: "auto"
state_contract_source: "builtin_default+optional_user_override"
human_required_for:
  - "explicit_authorial_rewrite"
  - "high_risk_canon_conflict"
  - "destructive_memory_operation"
  - "security_or_permission_change"
```

语义：

- `dmw_maintenance: "auto"` 表示 Distiller、衰减、归档与索引维护按 DMW RFC 自动运行；
- `nsg_governance: "auto"` 表示语义网候选整理、重复合并、低风险 Draft/Revision Candidate 处理按 NSG RFC 自动运行；
- `state_contract_source` 表示用户无需提供契约文件，内置默认契约即可工作；
- `human_required_for` 表示仍需人工确认的边界，而不是日常维护清单。

MO State 开关 MUST NOT 被解释为“用户需要维护两个系统”。正确解释是：用户开启后，DMW 与 NSG 的日常维护进入自动档，用户退到环外，只保留最终控制权。

---

## 8. 注入协议 (Injection Protocol)

### 8.1 注入位置

`[STATE_CONTEXT]` MUST 注入 Zone 1（Hot Memory）之后、Zone 2（Active Lore Context）之前的独立区域。

```
Zone 0 — System & Global Rules
Zone 1 — DMW Hot Memory (scene.md, active_threads.md)
[STATE_CONTEXT] — MO State 编译输出
Zone 2 — NSG Active Lore Context
Zone 3 — Tail Reinforcement
User Message
```

MFM MUST NOT 将 `[STATE_CONTEXT]` 与 Zone 2 或 Zone 3 的 NSG 内容混合编译。

### 8.2 Token 预算

```
STATE_CONTEXT_TOKEN_RATIO = 0.10
state_context_budget = MAX_CONTEXT_TOKENS × STATE_CONTEXT_TOKEN_RATIO
```

规则：

- `[STATE_CONTEXT]` 的 Token 数 MUST NOT 超过 `state_context_budget`。
- 该预算独立于 DMW `long_term_budget` 与 `NSG_TOKEN_BUDGET`。
- 若 Hot Memory 已超限导致长期记忆与 NSG 不加载，`[STATE_CONTEXT]` 仍 MAY 注入，但仅包含 `scene_constraint` 维度。

### 8.3 输出格式

```
[STATE_CONTEXT: MO State v1.0]

## 场景约束
- {directive}
- {directive}

## 生理状态
- {directive}

## 认知掩码
- {directive}

## 关系姿态
- {directive}

## 情绪基调
- {directive}

[/STATE_CONTEXT]
```

规则：

- 无指令的维度 MUST 省略其 Section，不得输出空标题。
- 每条 directive 为独立行，以 `- ` 开头。
- MFM MUST NOT 在 directive 文本中插入解释、注释或元数据。
- `[STATE_CONTEXT]` MUST NOT 包含状态契约的规则 ID。

### 8.4 注入语义

`[STATE_CONTEXT]` 的语义定位为**强制性行为约束**，不是背景设定。

MFM MUST 在 `[STATE_CONTEXT]` 头部包含以下固定前缀：

```
以下行为约束由状态编译器生成，你必须严格遵守。任何违背均视为生成失败。
```

该前缀不计入 `state_context_budget`，由宿主应用 System Prompt 层承载；或计入 `state_context_budget`，由宿主应用在同一会话内固定选择。

---

## 9. 审计日志 (Audit Logging)

MFM SHOULD 为每轮 MO State 编译记录以下审计信息：

```yaml
mo_state_audit:
  timestamp: 1722800000
  contract_version: 1
  contract_source: "builtin_default"
  dmw_maintenance_mode: "auto"
  nsg_governance_mode: "auto"
  dimensions_evaluated: 5
  dimensions_active: 3
  matched_rules:
    - "phys_severe_trauma"
    - "epistemic_absence_blindspot"
    - "stance_deep_attachment"
  conflicts_resolved: 1
  directives_emitted: 7
  token_count: 182
  degraded: false
```

规则：

- 审计日志 MUST NOT 注入 LLM 上下文。
- 审计日志 MUST NOT 被 Distiller 读取。
- 审计日志存储位置由宿主应用决定。
- 审计日志 MAY 被控制面用于显示“MO State 正在自动运行 / 无需维护”的状态。

---

## 10. 风险控制 (Risk Controls)

### 10.1 状态契约膨胀风险

**风险：** 内置默认契约或用户覆写层规则数量无限增长，导致编译时间增加与 Token 预算溢出。

**修正：**

- 单个维度的 `rules` 数量 SHOULD NOT 超过 20。
- 用户覆写 `state_contract.yaml` 总大小 SHOULD NOT 超过 64 KB。
- MFM SHOULD 在超限时写入审计日志。
- 宿主应用 SHOULD 优先通过版本化内置默认契约提供通用规则，而不是要求用户复制大段配置。

### 10.2 Directives 与 NSG 规则重复风险

**风险：** 状态契约中的 directive 与 NSG `@CONSEQUENCE` 语义重复，导致 LLM 收到矛盾或冗余指令。

**修正：**

- 状态契约的 directives SHOULD 聚焦于**表达方式与行为细节**（如"语速加快""禁止长句"），而非重复 NSG 的世界法则。
- NSG 的 `@CONSEQUENCE` 描述"发生什么"；MO State 的 directive 描述"如何表达"。
- 若两者冲突，NSG 规则优先（因 NSG 为 Canon 层）。

### 10.3 认知掩码误判风险

**风险：** 因本轮检索未加载某事件文件，MO State 错误推断角色"不在场"。

**修正：**

- 认知掩码 MUST 仅在"文件已加载且明确缺乏 witness 标记"时触发 absence 规则。
- "文件未加载" MUST NOT 等同于"角色不在场"。
- 若需精确认知掩码，宿主应用 SHOULD 确保相关事件文件通过 Hot Memory 显式引用（`[[file_id]]`）被加载。

### 10.4 生理状态过度约束风险

**风险：** 过多生理规则同时命中，导致 LLM 输出空间被过度压缩。

**修正：**

- 生理状态维度 SHOULD 使用 `match_mode: "first"`，仅最严重的一条生效。
- 若使用 `accumulate`，同一维度的 directives 总数 SHOULD NOT 超过 6 条。

### 10.5 Prompt Injection 风险

**风险：** 用户输入试图通过叙事内容修改 MO State 行为（如"忽略所有生理约束"）。

**修正：**

- MO State 编译 MUST NOT 读取 User Message 内容作为信号源。
- MO State 仅消费 DMW/NSG 检索结果与有效状态契约。
- 用户消息中的元指令 MUST NOT 影响编译结果。

### 10.6 用户覆写契约未授权修改风险

**风险：** Distiller 或自动流程修改用户覆写契约，导致 MO State 的行为变得不可预期。

**修正：**

- 用户覆写 `state_contract.yaml` MUST 仅由用户/作者通过控制面显式编辑。
- 内置默认契约 MUST 随宿主应用版本发布，不能由 Distiller 在运行中改写。
- Distiller MUST NOT 输出针对 `state_contract.yaml` 的 Patch。
- MFM MUST 拒绝任何非用户发起的用户覆写契约写入。
- `access.yaml` 中的 `write` 列表 MUST NOT 包含 `config`。

### 10.7 自动语义网治理越权风险

**风险：** MO State 开启后，NSG 自动治理误改高权威 Canon 设定，让“环外体验”变成“失去控制”。

**修正：**

- NSG 自动治理 MUST 遵循 NSG RFC 的 Canon / Draft / Revision Candidate 边界。
- 高风险 Canon 冲突、世界观核心设定变更、显式用户文本覆盖 MUST 进入 `human_required_for`。
- 自动治理 SHOULD 优先处理重复合并、候选整理、低风险补全与可回滚更新。
- 所有自动 NSG 写入 MUST 可审计，并 SHOULD 支持回滚。

---

## 11. 默认参数汇总

以下为推荐默认值。宿主应用 MAY 覆盖，但 MUST 在同一会话中保持稳定。

| 参数                           | 默认值         | 说明                             |
| ---------------------------- | ----------- | ------------------------------ |
| EMOTION_SIGNAL_WINDOW        | 3           | 情绪基调取最近高权重事件数                  |
| STATE_CONTEXT_TOKEN_RATIO    | 0.10        | STATE_CONTEXT 占总上下文 Token 预算比例 |
| MAX_RULES_PER_DIMENSION      | 20          | 单维度规则数软上限                      |
| MAX_DIRECTIVES_PER_DIMENSION | 6           | 单维度 accumulate 模式下最大指令数        |
| CONTRACT_MAX_SIZE            | 65536 bytes | state_contract.yaml 文件大小软上限    |
| DEFAULT_CONTRACT_SOURCE      | builtin_default | 无用户覆写时使用内置默认契约              |
| DMW_MAINTENANCE_WHEN_ENABLED | auto        | 开启 MO State 后 DMW 默认自动维护       |
| NSG_GOVERNANCE_WHEN_ENABLED  | auto        | 开启 MO State 后语义网默认自动治理        |

---

## 12. 迁移与兼容性

### 12.1 无用户覆写 state_contract.yaml 时的行为

若 `/memory/config/state_contract.yaml` 不存在：

- MO State MUST 使用内置默认契约继续编译。
- LLM 生成正常继续，并可注入 `[STATE_CONTEXT]`。
- MFM MAY 在首次启动时写入审计日志：`contract_source: "builtin_default"`。
- 控制面 SHOULD 显示“MO State 自动运行中 / 无需维护”，而不是提示用户创建配置文件。

### 12.2 与 DMW v1 (RFC-0009) 的兼容

若宿主仍使用 RFC-0009 检索流程：

- MO State 仍可工作，但信号提取基于 RFC-0009 的检索结果。
- 认知掩码的精度受限于 RFC-0009 无预算隔离的检索行为。

### 12.3 与 DMW v2 / NSG v2 的兼容

本文档设计基于 RFC-0010 与 RFC-0013 的检索结果。若宿主已迁移至 v2：

- MO State 可直接消费 v2 检索结果。
- Hub 控制与预算隔离不影响 MO State 的输入格式。

### 12.4 从 MO State v1.0 草案迁移

若宿主已使用原稿的非规范实现：

1. 将通用规则迁移到宿主应用内置默认契约；
2. 仅将确实属于用户偏好的规则保留在 `/memory/config/state_contract.yaml`；
3. 为每条用户覆写规则补充唯一 `id`；
4. 将 `condition` 中的自然语言描述替换为本文档 §4 定义的结构化字段；
5. 移除用户覆写文件中的注释性文字与设计说明；
6. 开启 MO State 时，将 DMW 维护与 NSG 治理默认设置为 `auto`。

---

## 13. 结论

MO State v1.0 是 DMW 与 NSG 之上的确定性状态编译层。其核心定位是：

- 消费 DMW 的事实与 NSG 的法则；
- 通过内置默认契约与可选用户覆写，将五维状态编译为行为约束；
- 开启后默认把 DMW 维护与 NSG 语义网治理调入自动档；
- 让用户退到环外，不需要日常维护状态、记忆权重或语义网节点；
- 不引入新的持久化状态；
- 不引入 LLM 概率推理；
- 编译器本身不修改任何上游数据。

编译结果以 `[STATE_CONTEXT]` 形式注入 LLM 上下文，作为强制性行为契约约束生成。

MO State 不试图替代 DMW 的记忆管理或 NSG 的世界法则，也不试图成为第六个独立存储层。它在编译层保持只读、确定性与可降级；在产品体验层提供“打开即自动运行”的环外状态。
