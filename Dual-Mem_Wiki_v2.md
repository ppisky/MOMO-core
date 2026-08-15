# DMW-STD-0010：Dual-Mem Wiki v2 Specification


Standard: DMW-STD-0010                                 August 04, 2026
Category: Specification                                Status: Implementation Baseline


## 摘要 (Abstract)

本文档独立定义 Dual-Mem Wiki v2（DMW v2）的数据结构、检索、访问追踪、1-Hop 扩展、预算注入与后台维护规则。实现方不需要读取其他版本的 DMW 文档即可实现本规范。

DMW v2 的核心架构为：

```text
Markdown 正文为记忆事实来源；
YAML Frontmatter 为轻量元数据；
MFM 为唯一执行者；
Distiller 仅提议叙事变化；
记忆生命周期仍为 active → archived → forgotten。
```

DMW v2 的规范能力包括：

1. **将“注入曝光”与“实质命中”分离**。  
   文件被加载进上下文不再自动刷新 `touch_at`，仅更新控制面 `injected_at`。只有当文件被当前 Query、Hot Memory 显式引用、Distiller 实质更新或用户显式恢复时，才刷新 `touch_at`。

2. **Hit-Based Decay**。  
   权重衰减使用 `touch_at`；`touch_at` 表示最近一次实质性命中或更新，而不是单纯读取或注入。

3. **引入 Hub-Controlled 1-Hop Expansion**。  
   对超级 Hub 节点限制扩展数量，并对来自 Hub 的扩展候选施加排序惩罚。扩展候选不得无限制挤占直接命中候选的 Token 预算。

4. **引入 Direct Pool 与 Expansion Pool 预算隔离**。  
   直接命中候选拥有保留预算，1-Hop 扩展候选仅能使用受限预算，避免 Hub 关联节点挤掉真正相关的长尾记忆。

5. **Hot Memory、索引、别名、关系完整性、并发与时钟安全要求**。

---

# 1. 设计目标 (Design Goals)

DMW v2 必须满足以下目标：

```text
重要事件保留；
普通经历淡化；
无意义且不再影响未来叙事的细节最终消失。
```

DMW v2 的检索与生命周期遵循：

1. **Exposure is not Relevance**  
   被注入 Prompt 不等于被当前叙事实际引用。

2. **Hit drives Lifetime**  
   只有实质性命中才应延长记忆的活跃生命周期。

3. **Expansion is Hypothesis, not Confirmation**  
   1-Hop 扩展只是图结构上的邻接假设，不应自动获得与直接命中相同的生命周期权益。

4. **Hub nodes must be controlled**  
   主角、主城、核心关系等高度数节点不得无限制拉入候选池。

5. **Deterministic and reproducible**  
   所有行为必须保持确定性、可复现性与轻量可实现性。

---

# 2. 核心语义 (Core Semantics)

## 2.1 `touch_at` 定义

```text
touch_at 记录该文件最后一次被实质性更新或实质性命中的时间。
单纯读取或注入上下文不得更新 touch_at。
```

## 2.2 注入与命中

```text
对本次成功加载到上下文中的文件，MFM MUST 更新控制面 injected_at。
仅当该文件满足 Relevance Hit 条件时，MFM MUST 更新其 touch_at。
```

## 2.3 1-Hop 扩展

```text
MFM MUST 对排名前 3 的文件执行 Hub-Controlled 1-Hop Expansion。
对 Hub 节点必须限制扩展数量，并对来自 Hub 的候选施加排序惩罚。
```

---

## 2.4 权限与生命周期

```text
Markdown 正文是记忆事实来源；
YAML Frontmatter 是轻量元数据；
importance 是静态重要度；
weight 是动态检索权重；
Distiller 不得直接写入 touch_at；
Distiller 不得输出 delete、forget、create_tombstone；
归档、遗忘、墓碑属于 MFM 控制面；
character、relationship、world 不得被后台自动遗忘；
event 满足严格条件后才可自动遗忘。
```

---

## 2.5 可选字段与控制面

DMW v2 实现 MUST 为缺失的可选字段使用本规范定义的默认策略。控制面索引不得被当作
记忆事实来源；未知的非冲突 Frontmatter 字段 SHOULD 在读写往返中保留。

---

# 3. 元数据与控制面扩展 (Metadata & Control Plane Extensions)

## 3.1 长期记忆 Frontmatter

长期记忆文件仍 MUST 包含以下字段：

```yaml
id: "char_xiaohong"
type: "character"
importance: 0.9
weight: 0.85
touch_at: 1721350000
decay_at: 1721350000
relations:
  relationships: ["rel_player_xiaohong"]
tags: ["protagonist", "tsundere"]
status: "active"
```

v2 新增以下可选字段：

```yaml
aliases: ["小红", "xiaohong"]
```

说明：

```text
aliases 是检索别名。
aliases 是可选字段。
若文件 Frontmatter 中存在 aliases，memory_index.yaml SHOULD 以文件为准。
Distiller MAY 仅在对话中明确出现新称呼时添加 alias。
Distiller MUST NOT 添加泛化代词、单字、无区分度词汇作为唯一 alias。
```

## 3.2 `touch_at` 的 v2 语义

`touch_at` 是权重衰减的核心时间戳。

v2 中，`touch_at` MUST 仅在以下情况更新：

```text
1. Distiller 对该文件成功应用非空 Patch；
2. 当前 User Message 直接命中该文件，并满足 Relevance Hit 条件；
3. Hot Memory 通过显式引用语法引用该文件，并满足 Hot Reference Hit 条件；
4. 用户显式恢复归档文件；
5. 用户显式搜索并打开归档文件，且宿主应用将该行为暴露给 MFM。
```

以下行为 MUST NOT 更新 `touch_at`：

```text
1. 文件仅因 weight 排序进入 Token 预算；
2. 文件仅作为 1-Hop 扩展候选被加载；
3. 文件被 MFM 读取 Frontmatter；
4. 文件被索引扫描；
5. 文件被后台维护流程遍历；
6. 文件被审计或调试流程读取。
```

## 3.3 新增控制面索引：`indexes/memory_activity.yaml`

为避免每次注入都修改记忆文件 Frontmatter，DMW 使用可重建的控制面索引：

```yaml
version: 1
entries:
  char_xiaohong:
    last_injected_at: 1721350000
    injection_count: 42
  event_rainy_argument:
    last_injected_at: 1721340000
    injection_count: 7
```

规则：

```text
memory_activity.yaml 不是记忆事实来源。
memory_activity.yaml MUST NOT 注入 LLM 上下文。
memory_activity.yaml MUST NOT 被 Distiller 读取或写入。
MFM MUST 在成功加载文件后更新 last_injected_at。
MFM MAY 记录 injection_count。
memory_activity.yaml 损坏时可删除并重建。
```

实现方 MAY 将 `injected_at` 同步写入 Frontmatter，但：

```text
injected_at MUST NOT 参与权重衰减；
injected_at MUST NOT 参与归档判断；
injected_at MUST NOT 参与遗忘判断；
injected_at 仅用于审计、调试与曝光偏差监控。
```

---

# 4. Relevance Hit Protocol（实质命中协议）

## 4.1 Hit 类型

DMW v2 定义以下 Relevance Hit 类型。

### H1：Direct Query Hit

当满足以下全部条件时，文件构成 Direct Query Hit：

```text
1. 文件 status == "active"；
2. 当前 User Message 的规范化关键词命中该文件的 id、alias 或 tag；
3. 命中词至少包含一个非泛化词，或同时命中多个词；
4. 文件通过当前会话 read 权限检查。
```

### H2：Hot Memory Reference Hit

当满足以下全部条件时，文件构成 Hot Memory Reference Hit：

```text
1. 文件 status == "active"；
2. current/scene.md 或 current/active_threads.md 中存在显式引用；
3. 显式引用形式为 [[file_id]]；
4. 被引用文件通过 read 权限检查。
```

示例：

```markdown
当前场景依赖以下记忆：
[[char_xiaohong]]
[[rel_player_xiaohong]]
[[event_rainy_argument]]
```

若 Hot Memory 中仅出现自然语言别名，例如：

```markdown
小红现在很不安。
```

MFM MAY 将其用于候选召回，但 MUST NOT 单独作为 `touch_at` 更新依据。

### H3：Distiller Update Hit

当 Distiller 对文件成功应用非空 Patch 时，该文件构成 Distiller Update Hit。

包括：

```text
append；
replace；
create；
update_frontmatter 中至少一个持久字段发生变化。
```

### H4：User-Controlled Restore Hit

当用户通过控制面显式恢复归档文件时，该文件构成 Restore Hit。

---

## 4.2 泛化词保护 (Generic Alias Guard)

为防止“主角”“城市”“魔法”等高频泛化词导致全图激活，DMW 使用泛化词保护。

实现方 MUST 维护确定性泛化词策略。

以下词 SHOULD 被视为泛化词：

```text
1. 单字符词；
2. 人称代词；
3. 实现方内置 stopword；
4. 在活跃记忆索引中出现频率过高的 alias 或 tag；
5. 宿主应用显式标记为 generic 的 alias。
```

规则：

```text
若一个文件仅因泛化词被命中，MFM SHOULD NOT 更新其 touch_at。
该文件仍可进入候选池并参与 Token 排序。
若该文件同时被非泛化词命中，或被 Hot Memory 显式引用，则正常构成 Hit。
```

---

## 4.3 `touch_at` 更新规则

MFM MUST 按以下规则更新 `touch_at`：

| 场景                                | 是否更新 `injected_at` | 是否更新 `touch_at` |
| --------------------------------- | ------------------:| ---------------:|
| 文件被加载，且为 Direct Query Hit         | 是                  | 是               |
| 文件被加载，且为 Hot Memory Reference Hit | 是                  | 是               |
| 文件被加载，但仅因 weight 排序               | 是                  | 否               |
| 文件被加载，但仅来自 1-Hop Expansion        | 是                  | 否               |
| 文件未被加载，但为 Direct Query Hit 且排名靠前  | 否                  | SHOULD          |
| Distiller 成功更新文件                  | 否                  | 是               |
| 用户恢复归档文件                          | 否                  | 是               |
| 后台维护读取文件                          | 否                  | 否               |

补充规则：

```text
对于未被加载但构成 Direct Query Hit 的文件，
MFM SHOULD 更新排名前 HIT_REFRESH_LIMIT 的文件的 touch_at。
```

默认值：

```text
HIT_REFRESH_LIMIT = 5
```

目的：

```text
避免某些文件虽然被当前 Query 命中，
但因 Token 预算不足长期无法加载，
进而因无法曝光而持续衰减。
```

该机制是对“预算饥饿”的补偿，但不应过度扩大。实现方 MUST 限制每轮因该规则更新的文件数量。

---

# 5. Retrieval Protocol v2（检索流程 v2）

DMW v2 的 MFM 读取流程如下。

完整流程如下：

```text
User Message
    ↓
[Step 1] Hot Memory Loading
    ↓
[Step 2] Query Extraction
    ↓
[Step 3] Direct Candidate Recall
    ↓
[Step 4] Hot Reference Recall
    ↓
[Step 5] Direct Pool Ranking
    ↓
[Step 6] Hub-Controlled 1-Hop Expansion
    ↓
[Step 7] Expansion Pool Ranking
    ↓
[Step 8] Budgeted Injection
    ↓
[Step 9] Timestamp Update
    ↓
Context Ready
```

---

## 5.1 Step 1：Hot Memory Loading

MFM 首先读取：

```text
current/scene.md
current/active_threads.md
```

v2 新增 Hot Memory Token 上限：

```text
HOT_MEMORY_TOKEN_RATIO = 0.45
```

即：

```text
hot_memory_budget = MAX_CONTEXT_TOKENS * HOT_MEMORY_TOKEN_RATIO
```

若 Hot Memory 超过预算：

```text
MFM MUST 优先保留 scene.md；
随后按 Markdown 段落或 Section 边界从 active_threads.md 尾部裁剪；
该轮不得因 Hot Memory 超限而完全取消长期记忆预算，除非 Hot Memory 裁剪后仍超限。
```

若 Hot Memory 裁剪后仍超过 `MAX_CONTEXT_TOKENS`：

```text
MFM MUST 继续裁剪 active_threads.md；
若仍超限，MFM MAY 对 scene.md 按 Section 边界裁剪；
该轮不得加载长期记忆。
```

---

## 5.2 Step 2：Query Extraction

MFM MUST 对当前 User Message 执行：

```text
Unicode NFKC 规范化；
首尾空白清理；
不区分大小写；
确定性分词；
集合去重。
```

MFM SHOULD 使用确定性 stopword 列表过滤无信息量词元。

同一会话内 MUST 使用同一分词与 stopword 策略。

---

## 5.3 Step 3：Direct Candidate Recall

MFM 在 `indexes/memory_index.yaml` 中查找：

```text
id 精确匹配；
alias 匹配；
tag 匹配。
```

仅返回：

```text
status == "active"
且通过 read 权限检查
```

的文件。

所有直接命中候选构成：

```text
direct_candidates
```

---

## 5.4 Step 4：Hot Reference Recall

MFM 解析：

```text
current/scene.md
current/active_threads.md
```

中的显式引用：

```text
[[file_id]]
```

所有被显式引用且 `status == "active"` 的文件加入：

```text
hot_reference_candidates
```

若引用目标不存在或已归档：

```text
MFM MUST 忽略该引用；
MFM SHOULD 写入审计日志；
MFM MUST NOT 因悬挂引用加载归档文件。
```

---

## 5.5 Step 5：Direct Pool Ranking

Direct Pool 定义为：

```text
direct_pool = direct_candidates ∪ hot_reference_candidates
```

排序规则：

```text
1. weight 降序；
2. importance 降序；
3. touch_at 降序；
4. id 升序。
```

该排序必须保证可复现。

---

## 5.6 Step 6：Hub-Controlled 1-Hop Expansion

MFM MUST 从 `direct_pool` 排序结果中选择前 `EXPANSION_SOURCE_LIMIT` 个文件作为扩展源。

默认值：

```text
EXPANSION_SOURCE_LIMIT = 3
```

若 `direct_pool` 为空：

```text
MFM MUST NOT 执行 1-Hop Expansion。
```

也就是说，DMW v2 不允许仅凭 Hot Memory 全量关系或历史高权重节点无限制扩散。

---

## 5.7 Hub 判定

对每个扩展源，MFM MUST 计算：

```text
source_degree =
  source.relations 中所有 active 且可读的目标 ID 总数
```

若：

```text
source_degree > HUB_THRESHOLD
```

则该源节点为 Hub。

默认值：

```text
HUB_THRESHOLD = 15
```

---

## 5.8 扩展配额

扩展配额如下：

```text
NORMAL_EXPANSION_PER_SOURCE = 8
HUB_EXPANSION_PER_SOURCE = 3
MAX_EXPANSION_TOTAL = 15
```

规则：

```text
若扩展源不是 Hub：
  最多扩展 NORMAL_EXPANSION_PER_SOURCE 个目标。

若扩展源是 Hub：
  最多扩展 HUB_EXPANSION_PER_SOURCE 个目标。

所有扩展源合计扩展目标数不得超过 MAX_EXPANSION_TOTAL。
```

扩展目标 MUST 满足：

```text
status == "active"；
通过 read 权限检查；
未出现在 direct_pool 中；
未在本轮扩展池中重复。
```

---

## 5.9 Hub 扩展优先级

若扩展源是 Hub，MFM MUST 优先选择以下目标：

```text
1. 同时被当前 Query 命中的关系目标；
2. 被 Hot Memory 显式引用的关系目标；
3. 与当前 scene.md 或 active_threads.md 中显式引用目标相邻的关系目标；
4. 其余目标按自身 weight、importance、touch_at、id 排序。
```

若仍超过 `HUB_EXPANSION_PER_SOURCE`，MFM MUST 截断。

---

## 5.10 可选边强度索引

`relations` 可以只包含目标 ID。为支持更精细的扩展，DMW v2 定义可选控制面索引：

```yaml
version: 1
edges:
  char_player:
    - target: "event_rainy_argument"
      strength: 0.85
    - target: "rel_player_xiaohong"
      strength: 0.95
```

规则：

```text
relation_index.yaml 是可选索引。
relation_index.yaml 不是记忆事实来源。
relation_index.yaml MUST NOT 注入 LLM。
strength 范围为 0.0 到 1.0。
若缺失 strength，MFM MUST 使用配额策略，不得假定默认强关系。
```

若存在 `strength`：

```text
Hub 源扩展 MUST 优先选择 strength >= HUB_EDGE_MIN 的目标。
普通源扩展 SHOULD 优先选择 strength >= EDGE_MIN 的目标。
```

默认值：

```text
HUB_EDGE_MIN = 0.6
EDGE_MIN = 0.3
```

若某 Hub 的所有关系均低于 `HUB_EDGE_MIN`：

```text
MFM MAY 不扩展该 Hub。
```

---

# 6. Budgeted Injection（预算隔离注入）

## 6.1 长期记忆预算

```text
long_term_budget = MAX_CONTEXT_TOKENS - used_hot_tokens
```

若 `long_term_budget <= 0`：

```text
该轮不得加载长期记忆。
```

---

## 6.2 Direct Reserve 与 Expansion Cap

预算隔离规则如下：

```text
DIRECT_RESERVE_RATIO = 0.60
EXPANSION_MAX_RATIO = 0.35
```

即：

```text
direct_reserved_budget = long_term_budget * DIRECT_RESERVE_RATIO
expansion_max_budget = long_term_budget * EXPANSION_MAX_RATIO
```

规则：

```text
当 direct_pool 非空时，
MFM MUST 为 direct_pool 保留至少 DIRECT_RESERVE_RATIO 的长期记忆预算。

1-Hop Expansion Pool 默认不得超过 EXPANSION_MAX_RATIO。
```

若 Direct Pool 用尽保留预算后仍有剩余 Token：

```text
MFM SHOULD 继续加载 Direct Pool 中未加载候选。
```

若 Direct Pool 已空且 Expansion Pool 未达上限：

```text
MFM MAY 允许 Expansion Pool 使用剩余预算，
但总 Expansion Token SHOULD NOT 超过 long_term_budget * 0.50。
```

该溢出仅 SHOULD 在以下情况启用：

```text
扩展源不是 Hub；
或存在 relation strength 且 strength >= 0.7；
或扩展目标同时被 Hot Memory 显式引用。
```

---

## 6.3 Expansion Pool 排序惩罚

来自 1-Hop Expansion 的候选 MUST 使用 `effective_weight` 排序。

定义：

```text
effective_weight = weight * expansion_factor
```

其中：

```text
若候选不是来自 Hub：
  expansion_factor = 1.0

若候选来自 Hub：
  expansion_factor = HUB_FACTOR
```

默认值：

```text
HUB_FACTOR = 0.75
```

若存在边强度：

```text
expansion_factor = expansion_factor * (0.6 + 0.4 * strength)
```

最终：

```text
effective_weight = clamp(effective_weight, 0.0, 1.0)
```

Expansion Pool 排序规则：

```text
1. effective_weight 降序；
2. importance 降序；
3. touch_at 降序；
4. id 升序。
```

若同一候选来自多个扩展源：

```text
MFM MUST 使用最高的 expansion_factor；
不得重复注入。
```

---

## 6.4 加载与裁剪

MFM MUST 按以下顺序加载：

```text
1. Direct Pool；
2. Expansion Pool；
3. 若仍有预算且 Direct Pool 仍有候选，回到 Direct Pool；
4. 若仍有预算且策略允许 Expansion Overflow，再回到 Expansion Pool。
```

裁剪规则如下：

```text
若单个文件无法放入剩余预算，MFM MUST 跳过该文件并继续检查后续候选；
MFM MUST NOT 在 Markdown Token 中间截断长期记忆文件；
MFM MUST 使用当前推理模型对应的 tokenizer；
若无法取得 tokenizer，MUST 使用保守估算器，并在同一会话内保持一致。
```

---

# 7. Timestamp Update v2（时间戳更新）

本轮上下文组装完成后，MFM MUST 执行以下更新：

## 7.1 对所有成功加载文件

```text
memory_activity.entries[id].last_injected_at = current_timestamp
memory_activity.entries[id].injection_count += 1
```

不得因加载修改：

```text
weight；
importance；
touch_at；
decay_at；
status。
```

## 7.2 对命中文件

若文件满足以下任一条件：

```text
Direct Query Hit；
Hot Memory Reference Hit；
Distiller Update Hit；
Restore Hit；
Direct Query Hit 但未加载，且位于 HIT_REFRESH_LIMIT 内。
```

则：

```text
file.frontmatter.touch_at = current_timestamp
```

该更新 SHOULD 异步执行，但 MUST 不阻塞主 LLM 生成。

## 7.3 对纯扩展文件

若文件仅因 1-Hop Expansion 被加载：

```text
MFM MUST NOT 更新 touch_at。
```

---

# 8. Maintenance Flow（后台维护）

## 8.1 权重衰减

权重衰减公式保持：

```text
weight = weight * 0.9
```

触发条件保持：

```text
current_timestamp - touch_at > 7 days
且
current_timestamp - decay_at >= 7 days
```

但由于 `touch_at` 不再被纯注入刷新，衰减不再被曝光免疫破坏。

## 8.2 时钟回拨保护

若：

```text
current_timestamp < touch_at
或
current_timestamp < decay_at
```

且差值超过：

```text
CLOCK_SKEW_TOLERANCE = 300 seconds
```

则 MFM MUST NOT 执行衰减。

MFM SHOULD 写入审计日志：

```text
reason: "clock_skew_guard"
```

## 8.3 归档规则

归档条件如下：

```text
weight < 0.2；
importance < 0.8；
status == "active"。
```

`importance >= 0.8` 的核心记忆不得自动归档。

v2 明确：

```text
归档判断 MUST 使用文件 Frontmatter 中的 weight 与 importance。
memory_activity.yaml 不得参与归档判断。
```

## 8.4 遗忘规则

遗忘条件如下；Hot Memory 引用检查仅识别：

```text
[[file_id]]
```

自然语言提及不应阻止遗忘，除非 Distiller 已将其提炼为长期记忆或显式引用。

---

# 9. 完整性与安全要求 (Integrity and Safety Requirements)

本节规定实现必须处理的完整性与安全风险。

---

## 9.1 Hot Memory 膨胀风险

### 风险

`current/scene.md` 与 `current/active_threads.md` 不参与权重衰减。若 Distiller 持续追加而不重写，Hot Memory 可能变成隐形长期记忆，并持续挤占 Token 预算。

### 规范要求

```text
current/scene.md SHOULD 表示当前状态，而不是历史日志。
current/active_threads.md SHOULD 仅保留正在推进的剧情线。
Distiller SHOULD 使用 replace 更新过时场景，而不是不断 append。
已结束剧情线 MUST 被移出 active_threads.md，或提炼为 events/ 文件。
```

建议结构：

```markdown
## Current Scene
...

## Open Threads
...

## Waiting Signals
...
```

不建议在 `active_threads.md` 中保留大段历史经过。

---

## 9.2 Hot Memory 显式引用污染风险

### 风险

如果 Hot Memory 中频繁引用大量长期记忆 ID，这些文件会持续获得 Hit，从而延长生命周期。这可能导致 Hot Memory 成为另一套“免衰减白名单”。

### 规范要求

```text
Hot Memory Reference Hit 仅识别显式 [[file_id]]。
Distiller MUST 仅引用下一轮叙事必需的文件。
Distiller SHOULD 在场景切换后删除不再相关的引用。
MFM SHOULD 在审计日志中记录每轮 hot_reference_candidates 数量。
```

建议宿主应用设置软上限：

```text
HOT_REFERENCE_SOFT_LIMIT = 8
```

超过该值时，MFM SHOULD 记录警告，但不强制失败。

---

## 9.3 别名泛化与别名冲突风险

### 风险

若多个文件共享高频 alias，例如：

```text
主角
城市
魔法
学院
```

则检索可能被泛化词污染，并进一步触发 Hub 扩展。

### 规范要求

```text
MFM MUST 在索引重建时记录 alias 冲突。
若同一 alias 映射多个 active 文件，MFM MUST 将其标记为 ambiguous_alias。
ambiguous_alias 可作为候选召回依据，但 SHOULD NOT 单独触发 touch_at 更新。
```

别名治理规则：

```text
单个 alias SHOULD 至少具有两个有效字符；
单字 alias SHOULD NOT 成为唯一 alias；
人称代词 MUST NOT 成为唯一 alias；
过于宽泛的 alias SHOULD 与 tag 联合使用。
```

---

## 9.4 索引漂移风险

### 风险

`memory_index.yaml` 可能与文件 Frontmatter 不一致，尤其是 aliases、tags、status、path 变化后。

### 规范要求

```text
memory_index.yaml 是可重建索引。
文件 Frontmatter 是元数据事实来源。
当索引与文件不一致时，MFM MUST 以文件为准并重建该索引项。
MFM SHOULD 在空闲时执行全量索引校验。
```

v2 建议新增审计字段：

```yaml
version: 1
validated_at: 1721350000
entries:
  char_xiaohong:
    path: "characters/xiaohong.md"
    type: "character"
    aliases: ["小红", "xiaohong"]
    tags: ["protagonist", "tsundere"]
```

---

## 9.5 Distiller 权重通胀风险

### 风险

Distiller 可能因短期戏剧性事件过度提高 `weight`，导致记忆排序被情绪强度而非叙事持续价值支配。

### 规范要求

权重更新必须满足：

```text
Mention frequency does not equal importance.
```

v2 进一步建议：

```text
Distiller 对已有文件的单次 weight 增长 SHOULD NOT 超过 0.25。
Distiller MUST NOT 仅因角色被多次提及而提高 weight。
Distiller SHOULD 仅在关系状态、剧情线、世界状态发生持续变化时提高 weight。
```

MFM MAY 实施软限制：

```text
WEIGHT_INCREASE_SOFT_LIMIT = 0.25
```

超过时，MFM SHOULD 记录审计日志，但不一定拒绝。

---

## 9.6 Importance 通胀风险

### 风险

`importance` 是静态重要度，不应随每轮对话波动。若 Distiller 频繁提高 `importance`，会导致核心记忆判定失真。

### 规范要求

```text
Distiller SHOULD NOT 修改已有文件的 importance。
若必须修改，MFM SHOULD 要求该变化来自明确重大剧情节点。
importance 跨越 0.8 阈值时，宿主应用 SHOULD 要求用户确认。
```

建议策略：

```text
自动 Distiller 可将 importance 提高至多 0.05；
超过该幅度或跨越 0.8 时，转为待确认建议。
```

---

## 9.7 关系悬挂风险

### 风险

文件 `relations` 可能引用不存在、已归档或已遗忘的 ID。

### 规范要求

```text
MFM MUST 在 1-Hop Expansion 中忽略不存在或不可读的目标。
MFM MUST NOT 因悬挂引用报错终止整轮检索。
MFM SHOULD 写入审计日志。
```

对于已遗忘 ID：

```text
MFM MUST NOT 恢复其内容；
MFM MUST NOT 将其加入候选；
MFM MUST NOT 允许 Distiller 通过 relations 重建已遗忘内容。
```

---

## 9.8 并发写入风险

### 风险

同一会话中可能存在多个 Distiller 任务、维护任务或用户编辑流程。

### 规范要求

```text
MFM MUST 对每个记忆根目录维护单写锁。
Distiller Patch、归档、遗忘、索引重建不得并发写入同一文件。
调用方 MUST 使用会话提炼任务 ID 去重。
临时文件写入、刷新、同卷重命名仍是唯一原子提交方式。
```

---

## 9.9 Token 计算漂移风险

### 风险

不同模型 tokenizer 不一致，可能导致预算裁剪不稳定。

### 规范要求

```text
MFM MUST 使用当前推理模型对应的 tokenizer。
若不可用，MUST 使用保守估算器。
同一会话内 MUST 固定 tokenizer 或估算器。
MFM MAY 按文件内容 hash 缓存 Token 数。
文件内容变化后缓存 MUST 失效。
```

---

## 9.10 审计日志缺失风险

### 风险

若无法解释某文件为何被加载、为何未衰减、为何归档，系统将难以调试。

### 规范要求

MFM SHOULD 记录控制面审计日志：

```text
retrieval decisions；
direct candidates；
expansion sources；
hub sources；
expansion penalties；
loaded files；
injected_at updates；
touch_at updates；
decay events；
archive events；
forget events。
```

审计日志 MUST NOT 注入 RP 上下文。

---

# 10. Distiller Prompt Requirements

以下模块应注入 DMW Distiller System Prompt。它不是 NSG Distiller 的组成部分。

```text
### DMW-v2-1. Injection Is Not Relevance
Do not treat a memory as important merely because it was loaded into context.
Loading can result from ranking budget, not narrative relevance.

### DMW-v2-2. Hit-Based Updates
Do not request lifetime extension for memories that were only passively present.
Only update weight when the narrative actually changes.

### DMW-v2-3. Current Memory Hygiene
current/scene.md MUST describe the current scene state.
Do not append historical scene logs.
Replace stale scene state instead of accumulating obsolete states.

current/active_threads.md MUST contain only active threads.
Closed threads MUST be removed or distilled into long-term event files.

### DMW-v2-4. Explicit References
When current memory depends on a long-term file, reference it explicitly as:
[[file_id]]

Do not rely on vague natural-language mentions as system references.

### DMW-v2-5. Alias Discipline
Do not add generic pronouns, single-character words, or overly broad terms as aliases.
Only add aliases that are explicitly used in the conversation.

### DMW-v2-6. Weight Discipline
Do not increase weight because a character or event was mentioned frequently.
Increase weight only when a persistent narrative state changes.
```

---

# 11. 默认参数汇总

以下为推荐默认值。宿主应用 MAY 覆盖，但 MUST 在同一会话中保持稳定。

| 参数                            | 默认值  | 说明                          |
| ----------------------------- | ----:| --------------------------- |
| `HOT_MEMORY_TOKEN_RATIO`      | 0.45 | Hot Memory 占 DMW Token 预算上限 |
| `DIRECT_RESERVE_RATIO`        | 0.60 | Direct Pool 保留预算比例          |
| `EXPANSION_MAX_RATIO`         | 0.35 | Expansion Pool 默认最大预算比例     |
| `EXPANSION_OVERFLOW_RATIO`    | 0.50 | Expansion Pool 溢出上限         |
| `EXPANSION_SOURCE_LIMIT`      | 3    | 参与 1-Hop 扩展的源文件数            |
| `NORMAL_EXPANSION_PER_SOURCE` | 8    | 普通源最大扩展目标数                  |
| `HUB_EXPANSION_PER_SOURCE`    | 3    | Hub 源最大扩展目标数                |
| `MAX_EXPANSION_TOTAL`         | 15   | 单轮最大扩展候选总数                  |
| `HUB_THRESHOLD`               | 15   | source_degree 超过该值视为 Hub    |
| `HUB_FACTOR`                  | 0.75 | Hub 扩展候选排序惩罚系数              |
| `HUB_EDGE_MIN`                | 0.6  | Hub 扩展的可选边强度下限              |
| `EDGE_MIN`                    | 0.3  | 普通扩展的可选边强度下限                |
| `HIT_REFRESH_LIMIT`           | 5    | 未加载但直接命中的 touch_at 刷新上限     |
| `CLOCK_SKEW_TOLERANCE`        | 300s | 时钟回拨容忍值                     |
| `WEIGHT_INCREASE_SOFT_LIMIT`  | 0.25 | Distiller 单次 weight 增长软上限   |
| `HOT_REFERENCE_SOFT_LIMIT`    | 8    | Hot Memory 显式引用软上限          |

---

# 12. 结论

DMW v2 的核心保证是：

```text
不再把“被注入上下文”视为记忆活跃的证据；
不再让 1-Hop 扩展无差别挤占直接命中预算；
不再让 Hub 节点凭借度数中心性制造检索噪声；
不再让 Hot Memory 与索引漂移成为隐性风险源。
```

DMW 的最终目标是：

```text
像人的长期记忆一样，
只留下真正影响未来叙事的东西；
而不是留下最容易曝光、最容易扩散、最容易被误刷新的东西。
```
