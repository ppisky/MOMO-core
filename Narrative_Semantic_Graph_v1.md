Internal Request for Comments: DMW-RFC-0012             July 30, 2026
Category: Implementation Guide                       Status: Final
Obsoletes: N/A                                       Updates: DMW-RFC-0011

# DMW-RFC-0012: Narrative Semantic Graph (NSG) Implementation Specification

## 摘要 (Abstract)

本文档是 DMW-RFC-0011 (NSG 模型规范) 的实现级补全。RFC-0011 确立了 NSG 的核心方向——存储"有推理影响力的事实"的叙事语义图谱。本文档不再扩展架构能力，而是将 RFC-0011 从模型规范推进为**可落地的实现规范**。核心增补包括：引入 `@CONDITION` 标签以表达复杂规则的适用前提；确立 **Manual Authority Principle (人工权威原则)**，明确 NSG 是半静态、人工维护的世界设定层，自动系统仅辅助提议；定义完整的 **Retrieval Protocol**、**NSG Patch Schema**、**Edge Inclusion Rule** 与 **NSG Distiller Prompt**；并将"冲突自动解决"修正为 **Canon Revision Protocol (设定修订协议)**。本文档发布后，NSG 的协议闭环即告完成。

---

## 1. 核心定位修正：半静态人工维护层 (Semi-Static Manual Authority Layer)

### 1.1 与 DMW 的本质区别

系统 MUST 在架构层面严格区分 DMW 与 NSG 的维护模式：

| 维度     | DMW (动态叙事记忆)             | NSG (叙事语义图谱)         |
| ------ | ------------------------ | -------------------- |
| 记录内容   | 谁在何时做了什么                 | 世界通常如何运行             |
| 变化频率   | 高频（每轮对话可能变化）             | 低频（数十章可能不变）          |
| 维护主体   | **自动维护**（Distiller 自动写入） | **人工维护**（作者/用户主导）    |
| 自动系统角色 | 执行者                      | 辅助者（仅提议，不直接修改 Canon） |
| 典型生命周期 | 创建 → 活跃 → 衰减 → 归档        | 创建 → 长期稳定 → 作者主动修订   |

### 1.2 Manual Authority Principle (人工权威原则)

NSG 的最终修改权 MUST 属于用户/作者，而非自动系统。

- **剧情变化 ≠ NSG 变化**。角色在对话中声称"我发现黑炎可以在圣湖使用"，不意味着 NSG 中的约束自动失效。
- 自动系统（Distiller）MAY 检测到潜在的设定冲突，但 MUST 仅生成 **修订建议 (Revision Candidate)**，不得直接修改 `# MODE: canon` 的节点。
- 只有用户/作者通过显式操作（GUI 编辑、确认修订建议、或手动修改 `.nsg` 文件）才能改变 Canon 设定。

### 1.3 Canon Mode

`.nsg` 文件元数据 MUST 新增 `MODE` 字段：

- `# MODE: canon` — 作者确认的正式设定。Distiller MUST NOT 直接修改此类节点，仅可提议修订。
- `# MODE: draft` — 自动系统或 Distiller 生成的候选设定，等待作者确认。Distiller MAY 直接更新此类节点。
- 默认值 MUST 为 `canon`。

---

## 2. `.nsg` 格式增补：@CONDITION 标签

### 2.1 设计意图

`@CONDITION` 用于表达复杂规则的**适用前提**，是静态规则描述的一部分（类似 DND 规则书中的"普通施法者状态下"），而非由 MFM 维护的动态状态字段。MFM MUST NOT 将 Condition 解析为表达式或在运行时以程序逻辑求值；Condition 是否适用于当前叙事，由 LLM 在阅读注入文本时结合上下文理解。该判断不会反向修改 NSG 节点。

### 2.2 推理链条

NSG 节点的语义结构形成完整的条件推理链：

```
@CONDITION  (在什么前提下，此规则存在)
    ↓
@TRIGGER    (什么动作/情境激活此规则)
    ↓
@CONSEQUENCE (激活后产生什么后果)
```

### 2.3 完整 `.nsg` 格式示例

**文件：`/lore/black_flame_magic.nsg`**

```text
# ID: lore_black_flame
# TYPE: lore
# IMP: 0.9
# MODE: canon
# STATUS: active
# ZONE: auto

@ANCHORS: 黑炎, 禁忌, 魔法, 反噬, 家族, 圣湖
@CONDITION: 施法者未获得圣湖祝福 / 施法者处于普通魔力状态
@TRIGGER: 施法者连续使用超过三次 / 进入圣湖区域
@CONSEQUENCE: 导致不可逆的魔力枯竭与肉体反噬 / 黑炎魔法完全失效
@CONSTRAINT: 被魔法师家族明文禁止，视为最高禁忌；需燃烧自身生命力作为燃料。

> constraint:forbidden_by [0.95] -> lore_mage_family
> causal:causes [0.9] -> status_life_drain
> narrative:changed_after [0.8] -> event_lake_incident
```

**解读**：`@CONDITION` 表达的是"这条规则在什么前提下成立"。如果未来作者决定主角获得了圣湖祝福，作者应主动修订此节点的 `@CONDITION`（如删除"未获得圣湖祝福"），而非由系统自动判断。

---

## 3. Edge Inclusion Rule (边准入规则)

### 3.1 核心判断标准

Distiller 与用户在创建边时，MUST 通过以下测试：

> **该边是否会直接影响 LLM 的以下行为之一？**
>
> 1. **行为生成** (Behavior Generation)：角色能否执行某动作。
> 2. **剧情走向** (Plot Direction)：某事件是否会导致特定后果。
> 3. **世界逻辑** (World Logic)：某法则是否约束物理/魔法规则。

若三项均为"否"，该边 MUST NOT 进入 NSG。此类信息应保留在 DMW 的角色档案或事件文本中。

### 3.2 准入示例

| 边                                                     | 是否准入 | 原因                       |
| ----------------------------------------------------- | ---- | ------------------------ |
| `black_flame → causal:causes → life_burn`             | ✅ 准入 | 直接影响行为后果                 |
| `black_flame → constraint:forbidden_by → mage_family` | ✅ 准入 | 影响世界逻辑与剧情冲突              |
| `xiaohong → structural:height → 165cm`                | ❌ 拒绝 | 不影响行为/剧情/逻辑              |
| `school → structural:near → cake_shop`                | ❌ 拒绝 | 除非位置关系触发剧情（如"蛋糕店是唯一安全屋"） |
| `xiaohong → narrative:betrayed_by → player`           | ✅ 准入 | 直接影响角色行为与关系动态            |

### 3.3 边语义分类（继承 RFC-0011）

所有准入的边 MUST 归属于以下四类之一，格式为 `> {category}:{relation_type} [{weight}] -> {target_id}`：

- **Structural**：`located_in`, `part_of`, `owned_by`, `contains`
- **Causal**：`causes`, `leads_to`, `trigger`, `prevents`
- **Constraint**： `forbidden_by`, `limited_by`, `requires`, `weak_against`
- **Narrative**： `betrayed_by`, `remembered_with`, `changed_after`, `allied_with`

Narrative 边仅用于表达已经形成、并会持续约束未来行为或剧情走向的叙事关系。一次性的情绪、事件经过或尚未稳定的人物关系变化 MUST 保留在 DMW；不得仅因某个事件发生过，就在 NSG 与 DMW 中重复存储同一动态事实。

---

## 4. Retrieval Protocol (检索协议)

### 4.1 完整检索流程

MFM 在处理 NSG 检索时，MUST 严格遵循以下流水线：

```
User Input (当前用户消息)
    ↓
[Step 1] Query Extraction (查询提取)
    提取关键词与语义意图
    ↓
[Step 2] Dual-Engine Recall (双引擎召回)
    ├─ Engine A: Anchor Match (语义锚点匹配)
    │   计算 Query 词元与各 .nsg 文件 @ANCHORS 的标准化重叠度
    │
    └─ Engine B: Vector Similarity (向量相似度, 可选)
        将 Query 转化为 Vector，执行本地精确 top-k 或经基准验证的 ANN 检索
        ⚠ Vector DB 仅作为语义召回工具，不是事实源
    ↓
[Step 3] Candidate Merge (候选合并)
    若双引擎同时启用，使用 RRF (Reciprocal Rank Fusion) 融合排序
    若仅 Engine A，直接使用锚点得分排序
    ↓
[Step 4] Weight Ranking (权重排序)
    按以下优先级排序：
    1. 检索得分 (Anchor/Vector Score) 降序
    2. IMP (Importance) 降序
    3. ID 升序 (保证可复现)
    ↓
[Step 5] 1-Hop Expansion (单跳扩散)
    对排名前 N 的节点（N 受 Token 预算约束），读取其 > 边
    将 target_id 加入候选池（去重）
    ⚠ MUST NOT 进行 2-Hop 或更深遍历
    ↓
[Step 6] Token Budget Truncation (Token 预算裁剪)
    按排序依次编译 [GRAPH_CONTEXT] 格式
    累加 Token，超出预算则停止
    ↓
[Step 7] Auto-Zone Injection (自动分层注入)
    按 RFC-0011 §4.2 的 Auto-Zone 规则分配至 Zone 0/2/3
    ↓
Context Ready (注入 LLM)
```

本规范继承 RFC-0011 的四个逻辑 Context 区域。为避免实现时混淆，各区域职责明确如下：

1. **Zone 0 — System & Global Rules**：头部最高注意力区，承载绝对不可违背的系统级与全局世界规则。
2. **Zone 1 — Hot Memory**：中前部高注意力区，承载 DMW 的 `current/scene.md`、`current/active_threads.md` 等当前叙事状态，不属于 NSG 检索结果。
3. **Zone 2 — Active Lore Context**：中部语义上下文区，承载当前 Query 命中并经 1-Hop 扩展后的 NSG 节点，使用 `[GRAPH_CONTEXT]` 格式。
4. **Zone 3 — Tail Reinforcement**：用户消息前的尾部强化区，仅承载对当前动作具有直接约束力的核心规则提醒。

### 4.2 Auto-Zone Decision Protocol (自动区域判定协议)

`# ZONE` 表达的是节点的注入策略。允许值为 `0`、`2`、`3` 与 `auto`；Zone 1 专属于 DMW Hot Memory，因此 `.nsg` 节点不得声明 `# ZONE: 1`。

- `# ZONE: 0`：由用户明确指定为全局规则，每轮固定注入 Zone 0。
- `# ZONE: 2`：由用户明确指定为普通 Active Lore，仅在检索命中时注入 Zone 2。
- `# ZONE: 3`：由用户明确指定为尾部核心约束，仅在检索命中时注入 Zone 3。
- `# ZONE: auto`：用户将本轮注入位置交给 Auto-Zone Resolver，根据当前场景决定进入 Zone 2，或在 Zone 3 进行强化。

Auto-Zone Resolver MUST 在候选召回、1-Hop 扩展与排序完成后执行。其判定输入只包含：

1. 当前 User Message；
2. DMW Hot Memory 中的当前场景与活动剧情线；
3. 当前 NSG 候选节点的 `@ANCHORS`、`@CONDITION`、`@TRIGGER`、`@CONSEQUENCE` 与 `@CONSTRAINT`；
4. Engine A 已计算的 Anchor Match 分数。

对于 `# ZONE: auto` 的候选，默认位置为 Zone 2。满足以下任一条件时，节点触发 Zone 3 自动强化：

- 当前输入与该节点的标准化 Anchor Match 分数大于 `0.6`；
- Auto-Zone Resolver 判断该节点会直接约束当前动作的可执行性、直接后果或世界规则。

Anchor Match 分数 MUST 复用 Step 2 Engine A 的标准化 `[0.0, 1.0]` 得分，不得为 Auto-Zone 再计算另一套不兼容的重叠度。Auto-Zone Resolver SHOULD 由轻量 LLM 实现，以处理字面锚点较少但语义上直接相关的场景；当该模型不可用或输出无法通过 Schema 校验时，MFM MUST 仅使用 `> 0.6` 的 Anchor Match 规则作为确定性降级路径。

Resolver 对每个 `auto` 节点只能输出：

```yaml
id: "lore_fire_magic"
zone: 2                       # 仅允许 2 或 3
reason: "default_zone_2"      # 仅允许下述枚举，不写回 NSG
```

`reason` 只允许为 `default_zone_2`、`anchor_match` 或 `direct_constraint`。未知字段、未知枚举、缺失候选 ID 或非整数 Zone 均视为无效输出，并触发确定性降级。

Resolver 不得把节点放入 Zone 0 或 Zone 1，不得修改 Canon、Importance、边或任何 `.nsg` 内容。Zone 0 只能来自用户的显式维护；自动系统不得将普通候选提升为全局绝对规则。

#### 4.2.1 Anchor Match 标准化

Engine A MUST 对 Query 关键词集合 `Q` 与节点锚点集合 `A` 执行与 DMW 索引相同的 Unicode NFKC、首尾空白清理与不区分大小写规范化，并在集合内去重。分数定义为：

```text
anchor_score =
  |Q ∩ A| / sqrt(|Q| × |A|)
```

该公式等价于二值词项向量的余弦相似度，结果范围为 `[0.0, 1.0]`。任一集合为空时，`anchor_score = 0.0`。中文等无空格语言可以使用实现方的确定性分词器；同一会话内 MUST 使用同一分词与短语匹配规则。Engine A 不执行 LLM 语义判断，近义表达与隐含场景由 Auto-Zone Resolver 补充判断。

阈值比较 MUST 使用严格大于，即 `anchor_score > 0.6`；等于 `0.6` 不自动触发 Zone 3，但 Resolver 仍可因直接语义约束将其判定为 Zone 3。

#### 4.2.2 强化与重复的定义

Zone 3 的“强化”允许同一节点同时以完整 `[GRAPH_CONTEXT]` 出现在 Zone 2，并以与当前动作直接相关的规则摘录出现在 Zone 3。这是跨区域的有意强化，不属于错误的重复注入。

本规范所禁止的重复注入仅指：同一节点因 Anchor Match、Vector Recall 与 1-Hop Expansion 等多条召回路径，在**同一个 Zone 内**被编译两次或更多次。MFM MUST 在区域编译前按节点 `ID` 去重。

实现方 MAY 为节省 Token，将被强化节点从 Zone 2 移至 Zone 3，而不保留 Zone 2 全文；也 MAY 保留 Zone 2 全文并在 Zone 3 注入较短的核心约束。宿主应用必须在同一会话中固定选择一种策略，避免注入行为随轮次随机变化。

跨区域强化产生的摘录 MUST 计入同一轮 NSG Token 预算。最终编译超出预算时，MFM MUST 优先保留 Zone 3 的直接约束，再从 Zone 2 排名最低的节点开始移除；不得截断单条规则的 Token。

Auto-Zone 只决定本轮编译结果的注入位置与强化方式，不修改任何持久状态。

### 4.3 Vector DB 角色声明

- Vector DB **不是事实源**。所有事实 MUST 从 `.nsg` 文件读取。
- Vector DB **仅是语义召回工具**。其作用是扩大候选池，提高召回率。
- 向量检索返回的 `ID` 列表 MUST 回到文件系统读取对应 `.nsg` 文件内容。
- 若 Vector DB 不可用，系统 MUST 无缝降级至 Engine A，功能不受影响。

---

## 5. NSG Patch Schema (NSG 专用补丁模式)

### 5.1 与 DMW Patch 的关系

NSG Patch 是 DMW YAML Patch 协议的扩展。传输层仍使用 YAML，MFM 接收后编译为 `.nsg` 纯文本。NSG Patch 的操作类型与 DMW Patch 独立，MFM MUST 根据 `target_file` 的后缀（`.nsg` vs `.md`）自动路由至对应的解析器。

后缀路由只决定单个 Patch 使用哪套操作白名单，不改变响应级事务规则：同一 `patches` 数组中的 `.nsg` 与 `.md` Patch 仍须先全部完成解析、权限检查与语义校验，任一 Patch 失败时整个响应不得产生持久化修改。`revision_candidate` 仅写入待审核区，不视为对目标 Canon 节点的修改。

### 5.2 操作类型定义

#### `create_node` — 创建新 NSG 节点

```yaml
- type: "create_node"
  metadata:
    id: "lore_holy_lake"
    type: "lore"
    importance: 0.85
    mode: "draft"              # 自动系统创建 MUST 为 draft
    status: "active"
  anchors: "圣湖, 祝福, 净化, 水域"
  condition: "施法者未获得圣湖认可"
  trigger: "携带暗属性魔法进入湖区"
  consequence: "暗属性魔法被净化失效，施法者受到圣光灼烧"
  constraint: "圣湖是大陆唯一能净化暗属性的天然水源。"
  edges:
    - category: "constraint"
      relation: "limited_by"
      weight: 0.9
      target: "lore_black_flame"
```

#### `update_node` — 更新节点语义内容

```yaml
- type: "update_node"
  fields:
    condition: "施法者未获得圣湖认可且未持有净化护符"
    trigger: "携带暗属性魔法进入湖区"
    consequence: "暗属性魔法被净化失效"
    constraint: "圣湖的净化力量在月圆之夜增强三倍。"
    anchors: "圣湖, 祝福, 净化, 水域, 月圆"
```

仅更新指定字段，未指定的字段保持不变。

#### `add_edge` — 添加关系边

```yaml
- type: "add_edge"
  edge:
    category: "narrative"
    relation: "changed_after"
    weight: 0.85
    target: "event_holy_blessing"
```

#### `remove_edge` — 移除关系边

```yaml
- type: "remove_edge"
  edge:
    category: "constraint"
    relation: "limited_by"
    target: "lore_holy_lake"
```

MUST 精确匹配 `category`、`relation` 与 `target` 三元组。

#### `update_frontmatter` — 更新元数据

```yaml
- type: "update_frontmatter"
  fields:
    importance: 0.95
    mode: "canon"
    status: "active"
```

可修改字段：`importance`, `mode`, `status`。`id` 与 `type` 创建后不可修改。

#### `archive_node` — 归档节点

```yaml
- type: "archive_node"
  reason: "设定已被新版本替代，保留历史参考"
```

将 `status` 改为 `archived`，移动至 `/archive/lore/`。MUST NOT 使用 `delete`，保留历史可追溯性。

### 5.3 校验规则

- 一个 Distiller 响应中的 NSG Patch MUST 先整体校验，再整体提交，禁止部分成功。
- `create_node` 要求目标文件不存在；其余操作要求目标文件已存在。
- 对 `# MODE: canon` 的节点，Distiller 生成的 `update_node`、`remove_edge`、`archive_node` 操作 MUST 被 MFM 拦截并转为 **Revision Candidate**（见 §7），不得直接执行。

---

## 6. NSG Distiller Prompt (NSG 专用提炼约束)

### 6.1 核心原则

NSG Distiller 比 DMW Distiller 更危险，因为错误的设定一旦写入 Canon，将长期污染所有后续推理。因此，NSG Distiller MUST 遵循更严格的约束。

### 6.2 独立 NSG Distiller System Prompt

NSG Distiller MUST 使用独立于 DMW Distiller 的 System Prompt。以下内容构成完整的 NSG 专用提炼约束模块，不作为 RFC-0008 §5.1 Distiller Prompt 的后续章节，也不继承其章节编号。宿主应用 MUST 将本模块注入独立的 NSG Distiller 模型或独立调用流程：

```text
### NSG-1. Creation Criteria

You MUST only propose NSG node creation when ALL of the following are met:
- The fact is an EXPLICIT world rule, law, or constraint confirmed by the narrative.
- The fact will DIRECTLY influence future behavior generation, plot direction, or world logic.
- The fact is NOT a temporary state, character opinion, or unconfirmed speculation.

NEVER create NSG nodes for:
- Character physical attributes (height, weight, birthday) unless they directly constrain plot.
- Geographic trivia (e.g., "school is near cake shop") unless the location triggers narrative consequences.
- User guesses or hypothetical statements (e.g., "I feel like the cake might have chili").
- Temporary emotional states (these belong in DMW).

### NSG-2. Edge Generation Rules
NEVER create an edge simply because two entities co-occur in the conversation.
An edge MUST pass the Narrative Impact Test:
- Will this relationship change how the LLM generates character behavior?
- Will this relationship alter plot outcomes?
- Will this relationship enforce or relax a world rule?
If all answers are "no", do NOT create the edge.

### NSG-3. Condition Semantics
@CONDITION describes the STATIC prerequisite under which a rule exists.
It is NOT a dynamic state to be evaluated at runtime.
Write conditions as you would write a rule in a tabletop RPG rulebook:
  Correct: "Caster has not received Holy Lake blessing"
  Incorrect: "Player HP < 50" (this is a state, not a rule)

### NSG-4. Canon Protection
- All NSG nodes you create MUST have `mode: "draft"`.
- You MUST NOT directly modify nodes with `mode: "canon"`.
- If you detect a potential conflict between current narrative and a canon NSG node, output a `revision_candidate` instead of a direct patch (see Canon Revision Protocol).

### NSG-5. No State Tracking
NSG is NOT a state engine. You MUST NOT track:
- Character levels, stats, or numeric values.
- World time or calendar progression.
- Relationship affinity scores.
These belong in DMW or external systems, not NSG.
```

---

## 7. Canon Revision Protocol (设定修订协议)

### 7.1 设计意图

当剧情发展与 Canon 设定产生潜在冲突时，系统 MUST NOT 自动修改 NSG。相反，系统生成**修订建议 (Revision Candidate)**，等待作者确认。

### 7.2 修订建议格式

当 Distiller 检测到潜在设定时，MUST 输出以下特殊 Patch 类型：

```yaml
patches:
  - target_file: "lore/black_flame.nsg"
    operations:
      - type: "revision_candidate"
        reason: "角色在对话中声称黑炎可在圣湖使用，与现有 constraint:limited_by 边冲突。"
        suggested_changes:
          - type: "update_node"
            fields:
              condition: "施法者未获得圣湖祝福且未持有净化护符"
          - type: "add_edge"
            edge:
              category: "narrative"
              relation: "changed_after"
              weight: 0.85
              target: "event_holy_blessing"
        source_evidence: "User: '我发现黑炎在圣湖也能燃烧！'"
```

### 7.3 修订生命周期

```
Distiller 检测到冲突
    ↓
生成 revision_candidate (mode: draft)
    ↓
MFM 将建议存入 /lore/.pending/ 目录
    ↓
宿主应用通知用户（GUI 弹窗 / 消息提示）
    ↓
用户审核：
  ├─ 确认 → MFM 执行 suggested_changes，节点保持 canon
  ├─ 修改 → 用户手动编辑后确认
  └─ 拒绝 → MFM 删除 revision_candidate，Canon 不变
```

### 7.4 规则变化 ≠ 删除

当作者确认设定变更时，修订操作 MUST 优先使用 `update_node`（修改 `@CONDITION`）和 `add_edge`（添加 `narrative:changed_after` 边），而非 `remove_edge` 或 `archive_node`。这确保了设定的历史演变可追溯。

---

## 8. 明确排除项 (Explicit Exclusions)

为防止实现过程中的概念膨胀，以下功能 MUST NOT 纳入 NSG：

### 8.1 ❌ State Engine (状态引擎)

NSG MUST NOT 追踪数值状态（角色等级、生命值、关系好感度、世界时间）。这些属于 DMW 或外部游戏系统的职责。

### 8.2 ❌ Multi-Hop Reasoning (多跳推理)

检索扩散 MUST 严格限制为 1-Hop。NSG 提供约束条件，推理过程交由 LLM 完成。系统 MUST NOT 实现 A→B→C→D 的自动链式推理。

### 8.3 ❌ Complete Ontology System (完整本体系统)

NSG MUST NOT 发展为分类学本体（如 Dragon → subtype → species → taxonomy）。除非分类本身直接触发剧情约束（如"龙族免疫火系魔法"），否则不建立分类层级。

### 8.4 ❌ Automatic Canon Mutation (自动 Canon 变更)

NSG 的 Canon 节点 MUST NOT 被任何自动流程直接修改。所有变更 MUST 经过作者确认。

---

## 9. 完整 RP 示例 (Complete RP Example)

### 9.1 初始 Canon 状态

`/lore/black_flame.nsg` (`# MODE: canon`)：

```text
@CONDITION: 施法者未获得圣湖祝福
@TRIGGER: 进入圣湖区域
@CONSEQUENCE: 黑炎魔法完全失效
> constraint:limited_by [0.9] -> lore_holy_lake
```

### 9.2 对话发生

User: 我站在圣湖中央，举起双手释放黑炎，火焰竟然在水面上燃烧了起来！
Xiaohong: 这不可能……圣湖的净化之力应该压制一切暗属性才对！

### 9.3 Distiller 行为 (遵循 Manual Authority Principle)

Distiller 检测到对话内容与 Canon 设定冲突，但 **不直接修改 NSG**。输出：

```yaml
patches:
  - target_file: "lore/black_flame.nsg"
    operations:
      - type: "revision_candidate"
        reason: "角色在圣湖区域成功释放黑炎，与现有 @CONDITION 和 constraint:limited_by 冲突。可能存在未记录的设定变更（如主角已获得圣湖祝福）。"
        suggested_changes:
          - type: "update_node"
            fields:
              condition: "施法者未获得圣湖祝福且未持有净化护符"
          - type: "add_edge"
            edge:
              category: "narrative"
              relation: "changed_after"
              weight: 0.85
              target: "event_holy_blessing"
        source_evidence: "User: '我站在圣湖中央释放黑炎，火焰在水面上燃烧'"
  - target_file: "events/holy_lake_anomaly.md"
    operations:
      - type: "create"
        frontmatter:
          id: "event_holy_lake_anomaly"
          type: "event"
          importance: 0.8
          weight: 0.9
          decay_at: 1722300000
          status: "active"
        content: |-
          # 圣湖异变
          ## 关键事件
          - 主角在圣湖中央成功释放黑炎魔法，火焰在水面上燃烧，违背了已知的圣湖净化法则。
          - 小红对此表示极度震惊，认为这不可能发生。
```

**注意**：DMW 事件被自动记录（这是 DMW 的职责），但 NSG 的 Canon 设定未被自动修改，仅生成了修订建议。

### 9.4 作者确认修订

作者在 GUI 中审核修订建议，确认主角确实已获得圣湖祝福，点击"确认修订"。MFM 执行 `suggested_changes`，NSG 更新为：

```text
@CONDITION: 施法者未获得圣湖祝福且未持有净化护符
@TRIGGER: 进入圣湖区域
@CONSEQUENCE: 黑炎魔法完全失效（已获祝福者除外）
> constraint:limited_by [0.9] -> lore_holy_lake
> narrative:changed_after [0.85] -> event_holy_blessing
```

---

## 10. 最终架构总览 (Final Architecture Overview)

```
                        LLM
                         |
                     MFM Core
              /          |          \
           DMW          NSG        Index
      动态叙事记忆    叙事语义图谱    检索辅助
      (自动维护)     (人工维护)     (派生索引)

      characters      lore        memory_index
      events          rules       anchor_index
      relationships   constraints vector_db (可选)
      current         triggers

    NSG 节点结构:
    Entity
    + @CONDITION   (静态规则前提)
    + @TRIGGER     (激活动作/情境)
    + @CONSEQUENCE (直接后果)
    + @CONSTRAINT  (基础法则)
    + Semantic Edges (Structural/Causal/Constraint/Narrative)
    + # MODE: canon | draft
```

---

## 11. 结论 (Conclusion)

DMW-RFC-0012 完成了 NSG 从模型规范到实现规范的最终闭环。本协议的核心贡献在于：

1. **确立人工权威原则**：明确 NSG 是半静态、人工维护的世界设定层。剧情变化不等于 NSG 变化，自动系统仅辅助提议，不直接修改 Canon。
2. **引入 @CONDITION**：与 `@TRIGGER` / `@CONSEQUENCE` 形成完整的静态规则描述链条，使复杂世界观的表达成为可能，同时避免沦为动态状态引擎。
3. **定义 Edge Inclusion Rule**：通过"叙事影响力测试"严格控制边的准入，防止 NSG 膨胀为无用的世界百科。
4. **补全 Retrieval Protocol**：明确了从 Query 提取到 Context 注入的完整七步流水线，并严格界定了 Vector DB 的"语义召回工具"角色。
5. **规范 NSG Patch Schema**：定义了 `create_node`、`update_node`、`add_edge`、`remove_edge`、`archive_node` 等完整操作集，并引入 `revision_candidate` 实现 Canon 保护。
6. **提供 NSG Distiller Prompt**：通过严格的创建条件、边生成规则与 Canon 保护约束，防止自动系统污染世界设定。

至此，**DMW (RFC-0008)** + **NSG (RFC-0011 + RFC-0012)** 构成了一个概念清晰、边界明确、协议完整的轻量级 RP 叙事状态管理系统。核心架构与协议 hereby 冻结，系统正式进入工程实现阶段。
