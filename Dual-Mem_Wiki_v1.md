**Internal Request for Comments: DMW-RFC-0009**             July 30, 2026
**Category: Implementation Guide**
**Status: Final**
**Obsoletes: DMW-RFC-0008**

# DMW-RFC-0009: MOMO Dual-Mem Wiki v1.0 最终实现指南 (Final Implementation Guide)

## 摘要 (Abstract)

本文档是 Dual-Mem Wiki (DMW) v1.0 的最终实现指南。基于前序 RFC 确立的“叙事优先、轻量管理”架构，本文档不再进行架构层面的扩展，而是专注于工程落地。本文档明确了 `importance` 与 `weight` 的分离机制、基于 `touch_at` 的访问追踪、YAML Patch 写入协议、物理目录结构、Token 预算算法，以及记忆的遗忘与保留策略。

DMW 的目标不是保存所有发生过的信息，也不是定期把全部历史压缩成越来越长的摘要，而是维护**当前叙事状态**：重要事件保留，普通经历淡化，无意义且不再影响未来叙事的细节最终消失。

---

## 1. 术语与核心修正 (Terminology & Core Amendments)

在正式进入实现细节前，本指南对 RFC-0007 中的部分概念进行最终工程化修正：

- **`importance` (静态重要度)**：0.0 到 1.0 的数字。由创建时设定或重大剧情定性，代表该记忆在叙事宇宙中的绝对价值（如“初次相遇” importance=0.9）。
- **`weight` (动态检索权重)**：0.0 到 1.0 的数字。代表该记忆在当前时刻的活跃程度与检索优先级。
- **`touch_at` (访问时间戳)**：Unix 时间戳（秒）。记录该文件最后一次被读取或实质性更新的时间。 **读取操作仅更新 `touch_at`，不增加 `weight`**，以防止错误记忆因频繁被提及而权重膨胀。
- **`decay_at` (衰减时间戳)**：Unix 时间戳（秒）。记录该文件最近一次完成权重衰减计算的时间，用于保证同一衰减周期不会被重复执行。
- **`archived_at` (归档时间戳)**：Unix 时间戳（秒）。仅在文件进入归档时由 MFM 写入，用于计算最短保留期。Distiller 不得创建或修改该字段。
- **`event` (事件)**：替代原 `episode`，用于记录具体的剧情事件（如 `event_rainy_argument.md`）。
- **YAML Patch**：替代 JSON Patch，作为 Distiller 输出和 MFM 执行的唯一指令格式，保持系统数据格式的纯粹性（仅 Markdown + YAML）。
- **`forgotten` (已遗忘)**：正文已经删除、仅留下最小墓碑的终态。它不是可被检索或恢复的 Markdown 文件状态。

---

## 2. 物理目录结构 (Physical Directory Structure)

系统 MUST 严格遵循以下目录布局。MFM (MemFS Manager) 在启动时需校验该结构，缺失则自动创建。

```text
/memory/
│
├── config/                     # 全局配置
│   ├── access.yaml             # 会话读写权限控制
│   └── schema.yaml             # 元数据字段约束（可选）
│
├── current/                    # 短期工作记忆 (Hot Memory)
│   ├── scene.md                # 当前场景状态
│   └── active_threads.md       # 正在推进的剧情线/伏笔
│
├── characters/                 # 角色档案 (Long-term Memory)
│   └── xiaohong.md             
│
├── relationships/              # 关系动态 (Long-term Memory)
│   └── player_xiaohong.md      
│
├── events/                     # 关键剧情事件 (Long-term Memory)
│   └── rainy_argument.md       
│
├── world/                      # 世界观与设定 (Long-term Memory)
│   └── city_map.md             
│
├── archive/                    # 归档区 (Cold Memory)
│   └── old_events/             
│
├── tombstones/                 # 遗忘墓碑（控制面，不参与 RP 检索）
│   └── forgotten.yaml
│
└── indexes/                    # 轻量级检索索引
    └── memory_index.yaml       # 标签、别名与文件映射
```

---

## 3. 文件元数据规范 (File Metadata Specification)

所有由 MFM 管理的 `.md` 文件 MUST 包含 YAML Frontmatter。`characters/`、`relationships/`、`events/` 与 `world/` 中的长期记忆文件 MUST 包含本节定义的全部通用字段；`current/` 中的热记忆文件仅 MUST 包含 `id`、`type: "current"`、`touch_at` 与 `status`，不参与权重衰减和归档。

长期记忆文件的 `id` MUST 在同一记忆库内唯一且创建后不可变。`relations` 与索引 MUST 使用 `id`，不得使用可能因移动或归档而变化的相对路径。

长期记忆 Markdown 文件的 `status` 只允许为 `active` 或 `archived`。进入遗忘终态后，原文件被删除，`forgotten` 仅记录在墓碑中。墓碑 ID 不得被自动复用。

### 3.1 角色文件 (`characters/xiaohong.md`)

```yaml
---
id: "char_xiaohong"
type: "character"
importance: 0.9                 # 核心角色，静态重要度高
weight: 0.85                    # 当前活跃度高
touch_at: 1721350000            # 最后访问/更新时间戳
decay_at: 1721350000            # 最近一次衰减计算时间戳
relations:
  relationships: ["rel_player_xiaohong"]
  events: ["event_rainy_argument"]
tags: ["protagonist", "tsundere"]
status: "active"
---
# 小红
(正文内容...)
```

### 3.2 关系文件 (`relationships/player_xiaohong.md`)

```yaml
---
id: "rel_player_xiaohong"
type: "relationship"
importance: 0.8
weight: 0.90                    # 近期有重大事件，权重高
touch_at: 1721350000
decay_at: 1721350000
relations:
  characters: ["char_player", "char_xiaohong"]
  events: ["event_rainy_argument"]
tags: ["conflict", "dependency"]
status: "active"
---
# 主角与小红
(正文内容...)
```

### 3.3 事件文件 (`events/rainy_argument.md`)

```yaml
---
id: "event_rainy_argument"
type: "event"
importance: 0.7
weight: 0.60                    # 事件发生后权重较高，随时间衰减
touch_at: 1721289600
decay_at: 1721289600
relations:
  characters: ["char_player", "char_xiaohong"]
  relationships: ["rel_player_xiaohong"]
tags: ["argument", "rainy_night"]
status: "active"
---
# 雨夜争吵
(正文内容...)
```

### 3.4 索引与访问配置 (`indexes/memory_index.yaml`, `config/access.yaml`)

`memory_index.yaml` 是可重建的检索加速数据，不是记忆事实来源。最小结构如下：

```yaml
version: 1
entries:
  char_xiaohong:
    path: "characters/xiaohong.md"
    type: "character"
    aliases: ["小红", "xiaohong"]
    tags: ["protagonist", "tsundere"]
```

- `entries` 的键 MUST 等于目标文件的 `id`，`path` MUST 是相对于 `/memory/` 的规范化路径。
- 索引与文件 Frontmatter 不一致时，以文件为准并重建该索引项。别名用于检索，不得被写回记忆正文成为叙事事实。
- 关键词匹配至少 MUST 对输入和别名执行 Unicode NFKC 规范化、去除首尾空白与不区分大小写比较。中文等无空格语言 MAY 使用实现方的分词器，但同一客户端版本必须保持确定性。
- 默认检索仅返回 `status: "active"` 的条目。归档条目只能通过明确的“搜索归档”流程恢复；恢复时移回原类型目录、设为 `active` 并更新索引。
- `tombstones/forgotten.yaml` 不属于检索索引。墓碑不得参与关键词匹配、1-Hop 扩展、Token 排序或上下文注入。

`access.yaml` 定义的是 MFM 会话级能力，而不是操作系统文件权限。未声明的能力 MUST 默认拒绝：

```yaml
version: 1
read: ["current", "character", "relationship", "event", "world"]
write: ["current", "character", "relationship", "event", "world"]
allow_archive_restore: false
```

运行环境 MAY 为不同会话生成更严格的配置，但不得通过 YAML Patch 修改 `access.yaml`、`schema.yaml` 或 `memory_index.yaml`。`allow_archive_restore: false` 时自动流程必须拒绝恢复；宿主应用可以在用户主动点击恢复后通过独立的控制面授权该次操作，不能让 Distiller 获得此能力。

---

## 4. MFM 工作流程 (MFM Workflow)

MFM (MemFS Manager) 是系统的控制中枢，负责读取、写入与后台维护。

### 4.1 读取流程与 Token 预算算法 (Retrieval Flow)

当主 LLM 准备生成回复前，MFM 执行以下算法组装上下文：

1. **强制加载 (Hot Memory)**：
   - 读取 `current/scene.md` 和 `current/active_threads.md`。
   - 计算其 Token 数，记为 `used_tokens`。`MAX_CONTEXT_TOKENS` 仅指 DMW 可注入预算，不包含 System Prompt、角色卡、当前用户消息与模型输出预留。
   - 若热记忆已超过预算，MFM MUST 优先保留 `scene.md`，再按 Markdown 段落边界从 `active_threads.md` 尾部裁剪；该轮不得再加载长期记忆。
2. **关键词匹配**：
   - 提取当前 User Message 的关键词。
   - 在 `indexes/memory_index.yaml` 中查找匹配的候选文件 ID。
3. **权重排序与 1-Hop 扩展**：
   - 读取候选文件的 YAML Frontmatter，依次按 `weight` 降序、`importance` 降序、`touch_at` 降序与 `id` 升序排序，以保证结果可复现。
   - **1-Hop 扩展**：对于排名前 3 的文件，读取其 `relations` 字段，将关联的文件 ID 加入候选池（去重）。
4. **二次排序与 Token 裁剪**：
   - 对所有候选文件（含 1-Hop 扩展）使用相同排序规则重新排序。
   - 依次读取文件正文，累加 Token。
   - **预算控制**：若单个文件无法放入剩余预算，则跳过该文件并继续检查后续候选；MFM 不得在 Markdown Token 中间截断长期记忆文件。
   - Token 数 MUST 使用当前推理模型对应的 tokenizer 计算；无法取得对应 tokenizer 时，运行环境 MUST 提供明确的保守估算器，并在同一会话内保持一致。
5. **更新 `touch_at`**：
   - 对本次成功加载到上下文中的文件，在后台异步更新其 `touch_at` 为当前时间戳。**注意：不改变 `weight`**。

### 4.2 写入流程：YAML Patch 协议 (Write Flow)

当触发提炼条件（如会话结束、轮次达标）时，MFM 调用 Distiller，并执行以下流程：

1. 接收 Distiller 输出的 YAML Patch。
2. 校验 `target_file`。路径 MUST 为相对于 `/memory/` 的规范化相对路径，且不得包含绝对路径、`..`、符号链接跳转或越出记忆根目录的结果。`create` 要求目标不存在，其余操作要求目标已存在。
3. 解析 `operations`：
   - `type: append`：在 Markdown 正文中找到名称完全匹配的 `## Section`，在该节末尾追加内容；目标 Section 不存在时整个 Patch 校验失败。
   - `type: replace`：替换名称完全匹配的 `## Section` 正文，保留标题；目标 Section 不存在时整个 Patch 校验失败。
   - `type: create`：创建新文件，应用 `frontmatter` 和 `content`。
   - `type: update_frontmatter`：仅更新 YAML 头部的持久字段。Distiller 不得写 `touch_at`，该字段由 MFM 在成功应用非空文件 Patch 后统一写入当前时间。
4. 在内存中完成全部校验后，以“临时文件写入、刷新、同卷重命名”的方式原子替换目标文件。
5. 更新 `indexes/memory_index.yaml`（如有新标签或别名）。目标文件与索引更新 MUST 作为同一逻辑事务提交；任一步骤失败时恢复更新前状态。

一个 Distiller 响应 MUST 是仅含一个 `patches` 根数组的 YAML 文档。数组按出现顺序执行；同一响应中的所有 Patch MUST 先整体校验，再整体提交，禁止部分成功。重复提交不保证幂等，因此调用方 MUST 使用会话提炼任务 ID 去重。

每个 Patch MUST 且只能包含 `target_file` 和非空 `operations`。操作采用严格白名单：

- `append`/`replace` 只能包含 `type`、`section`、`content`；
- `create` 只能包含 `type`、`frontmatter`、`content`，且必须是目标文件的唯一操作；
- `create.frontmatter` 必填 `id`、`type`、`importance`、`weight`、`decay_at`、`status`，可选 `relations`、`tags`；
- `update_frontmatter` 只能包含 `type`、`fields`，其中 `fields` 只能包含 `importance`、`weight`、`decay_at`、`relations`、`tags`、`status`。

`id` 与 `type` 创建后不可修改。`title` 不是操作字段或 Frontmatter 字段，标题必须写在 `content` 的 Markdown 中。未知操作或未知字段必须导致整个响应校验失败。

Distiller 不得输出 `delete`、`forget` 或 `create_tombstone` 操作，不得把 `status` 设为 `forgotten`，也不得写入 `archived_at`。遗忘属于 MFM 控制面维护，不属于叙事提炼。

### 4.3 后台维护流程 (Maintenance Flow)

MFM 在系统空闲时（如每小时）执行：

1. **权重衰减**：遍历所有长期记忆文件。仅当 `current_timestamp - touch_at > 7 days` 且 `current_timestamp - decay_at >= 7 days` 时执行一次 `weight = weight * 0.9`，随后将 `decay_at` 更新为当前时间。权重 MUST 限制在 0.0 到 1.0。
2. **冷却与归档 (Cooling)**：若 `weight < 0.2`、`importance < 0.8` 且 `status == "active"`，将其 `status` 改为 `archived`，写入 `archived_at`，并按原类型移动至 `/archive/{type}/`。MFM MUST 同步更新索引中的物理路径，但不得改变文件 `id` 或其他文件中的关系引用。`importance >= 0.8` 的核心记忆不得自动归档。
3. **淡化 (Fading)**：归档文件保留完整正文，但默认不加载；只有明确的“搜索归档”流程可以读取和恢复。归档期间 `weight` 继续按相同规则衰减。恢复时移回原类型目录、设为 `active`、移除 `archived_at` 并更新 `touch_at`，但不得自动提高 `importance` 或 `weight`。
4. **遗忘 (Forgetting)**：只有 `event` 同时满足以下全部条件时，MFM 才可自动删除正文：
   - `status == "archived"`；
   - `importance < 0.2`；
   - `weight < 0.05`；
   - `current_timestamp - archived_at >= 180 days`；
   - 没有任何 `active` 或 `archived` 文件通过 `relations` 引用它；
   - 没有被 `current/scene.md` 或 `current/active_threads.md` 引用。

`character`、`relationship` 与 `world` 可以归档，但不得被后台自动遗忘。任何高 `importance`、仍被关系依赖或仍影响活动剧情的记忆都必须保留。

遗忘后，MFM 从索引移除该 ID，并在 `/tombstones/forgotten.yaml` 中仅保留：

```yaml
event_noodle_day:
  type: "event"
  forgotten_at: 1798761600
  reason: "low_narrative_value"
```

墓碑不得保存原文、摘要、标签、别名或关系；否则墓碑本身会变成另一套压缩记忆。墓碑只用于防止 ID 被静默复用和支持系统审计，主 LLM 与 Distiller 均不得读取。

归档、恢复与遗忘 MUST 写入 MFM 系统审计日志，不得写入 RP Memory，也不得创建“Memory Evolution”章节。旧版本中缺少 `archived_at` 的归档文件在首次迁移时以迁移时间作为 `archived_at`，重新计算 180 天，避免升级后立即误删。

---

## 5. 核心 Prompt 设计：防幻觉约束 (Core Prompt Design: Anti-Hallucination)

Dual-Mem Wiki 中的 Prompt 不属于记忆数据结构本身，而属于 MFM 调度链中的执行规范。  
本规范要求 Distiller 使用英文 System Prompt，以提高模型在结构化输出、约束遵循以及 YAML 生成任务中的稳定性。

### 5.1 Distiller System Prompt (Memory Distiller)

```text
You are a Roleplay Memory Distiller for Dual-Mem Wiki (DMW).

Your task is to analyze recent conversation logs from a roleplaying session and generate a YAML Patch that updates existing long-term memory files.

Your goal is NOT to summarize the conversation.
Your goal is to preserve important narrative changes, character dynamics, emotional states, and ongoing story developments in the Dual-Mem Wiki memory system.

## Core Principles

### 1. Narrative First

- Preserve narrative meaning, emotional context, character motivations, relationship changes, and story continuity.
- Do not convert roleplay interactions into dry factual databases.
- Markdown content is the semantic source of truth.
- YAML frontmatter is only used for lightweight metadata management.

### 2. Output Constraint

- You MUST output ONLY valid YAML.
- The root object MUST contain exactly one field: `patches`.
- Every patch MUST contain exactly `target_file` and `operations`.
- Unknown fields are forbidden at every level.
- `title` is NEVER an operation or frontmatter field. Put a Markdown title in `content`.
- You MUST NOT output explanations, markdown code blocks, comments, or additional text.
- Every operation MUST follow the Dual-Mem Wiki YAML Patch specification.
- If there is no confirmed change worth storing, output exactly `patches: []`.

### 3. Supported Patch Operations

Available operation types:

- `append`
  - Add new narrative information to an existing Markdown section.
  - It has exactly `type`, `section`, and `content`.
  - The named `##` section MUST already exist. Otherwise create a new file.

- `replace`
  - Replace outdated narrative content in an existing Markdown section.
  - It has exactly `type`, `section`, and `content`.
  - The named `##` section MUST already exist. Otherwise create a new file.

- `create`
  - Create a new memory file when a new important character, relationship, event, or world element appears.
  - It has exactly `type`, `frontmatter`, and `content`.
  - It MUST be the only operation for its target file.
  - `frontmatter` requires `id`, `type`, `importance`, `weight`, `decay_at`, and `status`; `relations` and `tags` are optional.
  - No other frontmatter fields are allowed.

- `update_frontmatter`
  - It has exactly `type` and `fields`.
  - `fields` may contain only `importance`, `weight`, `decay_at`, `relations`, `tags`, and `status`.

### 4. Runtime-managed Time Field

- MOMO writes the exact current `touch_at` after every non-empty file patch.
- You MUST omit `touch_at` from `create` and `update_frontmatter`.
- Do not guess the current time and do not emit placeholder timestamps.

### 5. Weight Update Rules

Modify `weight` ONLY when:

- A major plot event occurs.
- A relationship significantly changes.
- The user explicitly confirms a persistent fact.
- A character reveals important emotional information.
- A major conflict or resolution changes the long-term narrative state.

For normal references:

- Do not emit an `update_frontmatter` operation.
- MOMO updates `touch_at` automatically.
- Do NOT increase `weight`.
- Mention frequency does not equal importance.

### 6. Anti-Hallucination Rules

The memory system must only store confirmed narrative information.

NEVER:

- Store model assumptions as facts.
- Convert user questions into memories.
- Store uncertain predictions.
- Store unsupported character motivations.
- Store temporary speculation.

Only store:

- Events that actually happened in the conversation.
- Explicit statements made by users or characters.
- Stable world settings confirmed by the roleplay context.
- Observable character behaviors shown through actions or dialogue.

### 7. Character Perspective Rule

When recording character emotions or internal states:

- Prefer behavioral evidence over unsupported conclusions.
- Do not claim hidden thoughts unless they are explicitly revealed through narration or confirmed by the character.
- Record what the character did, said, or clearly expressed.

Example:

Incorrect:
"Xiaohong deeply loves the protagonist but refuses to admit it."

Correct:
"Xiaohong verbally rejected the idea of separation but grabbed the protagonist's clothes and asked them not to leave, showing emotional attachment."

### 8. Memory Value Filtering

Prioritize information related to:

- Character personality changes.
- Relationship progression.
- Important emotional events.
- Major conflicts.
- Long-term story threads.
- World-building changes.

Ignore:

- Greetings.
- Repeated conversations.
- Temporary dialogue.
- Information with no future narrative value.
- Do not summarize low-value repetition merely to preserve it.
- If nothing changes the current narrative state, output `patches: []`.

## Output Format Example

patches:
  - target_file: "events/farewell_request.md"
    operations:
      - type: "create"
        frontmatter:
          id: "event_farewell_request"
          type: "event"
          importance: 0.8
          weight: 0.8
          decay_at: 1721350000
          relations:
            characters: ["char_xiaohong"]
          tags: ["farewell"]
          status: "active"
        content: |-
          # Farewell Request

          ## Key Changes

          - Xiaohong explicitly asked the protagonist not to leave.
  - target_file: "relationships/player_xiaohong.md"
    operations:
      - type: "append"
        section: "关键变化"
        content: "- During the farewell conversation, Xiaohong verbally rejected the idea of separation but ultimately grabbed the protagonist's clothes and asked them not to leave, revealing a significant emotional dependency."
      - type: "update_frontmatter"
        fields:
          weight: 0.95
```

### 5.2 Distiller 执行原则总结

Distiller 的职责不是创造角色理解，而是维护已有叙事状态。

执行优先级：

1. 保证事实真实性。
2. 保留叙事情绪。
3. 维护角色连续性。
4. 控制长期记忆膨胀。
5. 输出可被 MFM 确定执行的 YAML Patch。

任何无法确认的内容，都应放弃写入，而不是进行推测补全。

---

## 6. 完整 RP 示例 (Complete RP Example)

以下展示一个从对话到记忆更新的完整闭环。

### 6.1 初始状态

`relationships/player_xiaohong.md` 的 `weight` 为 0.7，`touch_at` 为 1721200000。

### 6.2 对话发生

```text
User: (主角) 我明天就要离开这座城市了。
Xiaohong: (小红) 随便你。反正你本来也待不长。
User: (主角) 你真的这么想？
Xiaohong: (小红) (沉默了很久，突然伸手抓住了主角的衣角，声音很小) ...别走。
```

### 6.3 触发提炼

会话结束，MFM 调用 Distiller。

### 6.4 Distiller 输出 YAML Patch

```yaml
patches:
  - target_file: "relationships/player_xiaohong.md"
    operations:
      - type: "append"
        section: "关键变化"
        content: "- 主角提出离开时，小红虽然口头讽刺，但最终在沉默中抓住了主角的衣角挽留，明确表现出不希望主角离开的态度。"
      - type: "update_frontmatter"
        fields:
          weight: 0.95
```

### 6.5 MFM 执行结果

`relationships/player_xiaohong.md` 被更新：

- 正文“关键变化”章节追加了新内容。
- YAML 头部 `weight` 变为 0.95，`touch_at` 更新为 1721350000。

### 6.6 下一轮读取

当下一轮对话开始时，MFM 检索记忆。由于 `relationships/player_xiaohong.md` 的 `weight` 高达 0.95，它将被优先加载到 1500 Token 的预算中，确保 LLM 能够完美接续小红“傲娇但内心依赖”的状态。

---

## 7. 结论 (Conclusion)

DMW-RFC-0009 作为 Dual-Mem Wiki v1.0 的最终实现指南，彻底摒弃了过度工程化的诱惑。通过明确 `importance` 与 `weight` 的职责边界，引入 `touch_at` 防止访问污染，统一使用 YAML Patch 维护叙事状态，并补全 `active → archived → forgotten` 生命周期，本指南为开发者提供了一条清晰、务实且高度可控的实现路径。

DMW 不把“保存过的一切”视为永久资产，也不把周期性压缩视为默认答案。重要事件保留，普通经历淡化，无意义且不再影响未来叙事的细节最终消失。

该系统不再试图成为一个越来越大的 Wiki，而是专注于做好一件事：**像人的长期记忆一样，只留下真正影响未来叙事的东西。** 这是 Dual-Mem Wiki v1.0 的最终冻结形态。
