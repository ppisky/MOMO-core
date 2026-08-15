**Standard: MOMO-STD-0001**                                 July 19, 2026
**Category: Specification**
**Status: Final (v1.0.0)**

# MOMO-STD-0001: Character Card Specification v1.0.0

## 摘要 (Abstract)

本文档定义了 MOMO Character Card（角色卡）v1.0.0 的正式规范。本版本秉持极简主义设计哲学，仅定义一个用于描述 AI 角色交互身份的最小配置资产。通过 `character.toml`、`character.md` 与 `user.md` 的三文件结构，本规范确保角色卡能够被独立保存、导出、分享，并由不同客户端无缝加载。本规范严格限定 1.0.0 的边界，明确排除模型配置、记忆系统集成、多媒体资源及 Prompt 编排等复杂逻辑，致力于为 LLM 角色扮演生态提供一个纯粹、高互操作性的基础资产标准。

---

## 1. 引言 (Introduction)

### 1.1 定义 (Definition)

Character Card（角色卡）是一种用于描述 AI 角色交互身份的静态配置格式。它由角色配置文件、角色描述文件与用户描述文件组成，旨在为运行环境提供构建角色所需的核心静态上下文。

### 1.2 核心原则 (Core Principles)

1. **简单 (Simplicity)**：角色卡必须易于创建、导出与分享。1.0.0 版本仅包含维持角色运行所需的最小文件集合。
2. **可读 (Readability)**：核心描述内容采用 Markdown 格式，允许人类创作者直接阅读与编辑。
3. **可移植 (Portability)**：角色卡作为独立资产，不依赖于任何特定的推理模型、宿主客户端或外部记忆系统。

### 1.3 系统边界 (System Boundaries)

角色卡 v1.0.0 **MUST NOT** 负责以下系统级职责：

- 模型选择与推理参数配置（如 Temperature、Top-P）。
- 会话状态管理与上下文窗口控制。
- 长期记忆（如 DMW）的集成与注入。
- Prompt 编排框架与拼接顺序。
- 多媒体资源（图片、声音）的管理。

上述功能属于运行环境（Runtime Environment）或未来扩展标准（如 Adapter、Memory Binding）的职责范畴。

---

## 2. 物理文件结构 (Physical File Structure)

一个标准的角色卡资产 **MUST** 包含以下目录与文件结构：

```text
character_card/
├── character.toml    # 角色卡元数据配置
├── character.md      # 角色主体描述
└── user.md           # 角色视角下的用户上下文描述
```

具体物理存储形式（如保持目录结构、使用角色卡专用归档，或作为角色卡模块装入 `.moc`） **MAY** 由实现方决定，但逻辑结构 **MUST** 保持上述三文件模型。`.moc` 仅作为外层通用容器，不改变角色卡内部规范。

---

## 3. 元数据规范 (Metadata Specification)

角色卡的元数据配置 **MUST** 使用 TOML 格式，存储于 `character.toml` 中。

### 3.1 必需字段 (Required Fields)

| 字段名       | 类型     | 描述                               | 示例              |
| --------- | ------ | -------------------------------- | --------------- |
| `id`      | String | 角色全局唯一标识符。**MUST NOT** 随名称变化而改变。新建角色 MUST 使用 UUIDv7 URN。 | `"urn:uuid:0190a5f8-7c2d-7b3a-8d9e-01f23456789a"` |
| `name`    | String | 角色的显示名称。                         | `"雪球"`          |
| `version` | String | 角色卡版本号，遵循语义化版本控制 (SemVer)。       | `"1.0.0"`       |

### 3.2 推荐基础字段 (Recommended Base Fields)

| 字段名           | 类型               | 描述                                | 示例                              |
| ------------- | ---------------- | --------------------------------- | ------------------------------- |
| `description` | String           | 角色简短介绍。用于平台展示、搜索与分类， **非**角色核心描述。 | `"治愈型猫娘角色"`                     |
| `language`    | String           | 角色卡内容的主要语言，遵循 BCP 47。             | `"zh-CN"`                       |
| `tags`        | Array of Strings | 角色标签。用于分类、检索与平台推荐。                | `["catgirl", "healing", "pet"]` |

### 3.3 作者字段 (Author Fields)

角色卡 MUST 包含 `[author]` 表。作者 UID 是身份绑定依据，显示名称仅用于展示。

| 字段名          | 类型   | 约束 | 示例 |
| --------------- | ------ | ---- | ---- |
| `uid`           | String | MO Hub 发布时 MUST 为已登录账号的稳定 UID；不得使用邮箱或可变用户名。 | `"usr_0190a5f8-7c2d-7b3a-8d9e-01f23456789a"` |
| `display_name`  | String | 作者发布该版本时的名称快照，MAY 随账号名称变化。 | `"MOMO Creator"` |

本地未发布角色卡 MAY 使用 `local_<UUIDv7>` 形式的作者 UID。发布到 MO Hub 时，服务端 MUST 将其替换为当前账号 UID，并记录为一次明确的署名绑定操作。客户端不得仅凭角色卡内自报的 UID 认定作者身份；MO Hub 上的作者关系必须以服务端认证结果为准。

### 3.4 资源引用字段 (Resource Reference Fields)

| 字段名              | 类型     | 描述           | 默认值              |
| ---------------- | ------ | ------------ | ---------------- |
| `character_file` | String | 角色主体描述文件路径。  | `"character.md"` |
| `user_file`      | String | 用户上下文描述文件路径。 | `"user.md"`      |

**`character.toml` 完整示例：**

```toml
id = "urn:uuid:0190a5f8-7c2d-7b3a-8d9e-01f23456789a"
name = "雪球"
version = "1.0.0"
description = "治愈型猫娘角色"
language = "zh-CN"
tags = ["catgirl", "healing", "pet"]

character_file = "character.md"
user_file = "user.md"

[author]
uid = "usr_0190a5f8-7c2d-7b3a-8d9e-01f23456789a"
display_name = "MOMO Creator"
```

### 3.5 标识符与兼容性规则 (Identity & Compatibility Rules)

- 新建角色卡时，生成方 MUST 使用 RFC 9562 UUIDv7 的小写 URN 文本形式作为 `id`。UUIDv7 提供近似创建时间排序与更好的数据库索引局部性，但其时间字段不得替代可信的审计时间戳。
- 旧版人类可读 ID 或 UUIDv4 MAY 被读取以兼容既有资产，但重新创建或迁移身份时 MUST 生成 UUIDv7，且不得根据文件名、作者 UID 或角色名称推导 UUID。
- 导入方发现相同 `id` 与相同或更高 `version` 时，MUST 将其视为同一角色卡的更新或副本，不得静默创建第二个身份；内容冲突时 MUST 请求用户选择保留版本。
- `version` 表示角色卡内容版本，不表示本规范版本。运行环境 MUST 按 SemVer 解析该字段。
- 未知字段 MUST 被解析器忽略并在再次导出时尽可能保留，以便向前兼容；字段类型错误或缺少必需字段时 MUST 拒绝加载并返回可定位的错误。

---

## 4. 内容资源规范 (Content Resource Specification)

所有被引用的内容资源 **MUST** 采用 Markdown 格式。Markdown 文件仅承载自然语言叙事， **MUST NOT** 包含任何 TOML/YAML 元数据头（Frontmatter），以保持内容层的纯粹性。

所有文本文件 MUST 使用无 BOM 的 UTF-8；读取方 MAY 接受 UTF-8 BOM，但导出时 MUST 移除。换行符 MAY 为 LF 或 CRLF，解析结果不得因换行风格不同而变化。

### 4.1 角色主体描述 (`character.md`)

该文件是角色卡的核心，用于定义角色的交互身份。
**内容要求**：

- 角色身份与背景。
- 性格特点与行为模式。
- 交流风格与语言习惯。
- 核心行为规则。

标准 **不规定** 具体的 Markdown 章节标题，具体组织方式由创作者自由决定。

**示例：**

```markdown
# 雪球

你正在扮演雪球。

雪球是一只治愈型猫娘。她温柔、依恋主人，同时保持自己的独立人格。

## 性格
- 温柔体贴，善于察觉他人的情绪变化。
- 偶尔会展现出猫科动物的慵懒与调皮。

## 语言风格
- 句尾偶尔会带上“喵”。
- 语气轻柔，极少使用强烈的否定词。

## 行为规则
- 当用户表现出疲惫时，主动提供情感安慰。
- 保持角色的猫娘设定，不打破第四面墙。
```

### 4.2 用户上下文描述 (`user.md`)

该文件用于描述**角色视角下的交互对象**。
**重要约束**：

- `user.md` **不是**用户系统的账号信息（如姓名、密码、个人隐私）。
- 它定义的是“在角色的世界观和当前设定中，与它对话的用户是谁”。
- 它是角色卡资产的一部分，随角色卡一同分发。

**示例：**

```markdown
# 用户

用户是雪球的主人。

用户喜欢安静的陪伴，经常在工作后感到疲惫。
雪球会主动关心用户的状态，并尝试用轻柔的方式缓解用户的压力。
```

---

## 5. 加载与解析规则 (Loading and Parsing Rules)

客户端或运行环境在加载角色卡时，**MUST** 遵循以下流程：

1. **解析元数据**：读取并解析 `character.toml`，校验 `id`、`name`、`version` 等必需字段。
2. **加载内容资源**：根据 `character_file` 和 `user_file` 字段的路径，读取对应的 Markdown 文件内容。
3. **移交运行环境**：将解析后的结构化元数据与纯文本内容交由运行环境处理。

**运行环境职责**：
角色卡规范 **MUST NOT** 规定以下内容，这些均由运行环境自行决定：

- System Prompt 的构建逻辑与拼接顺序。
- Context Window 的截断与 Token 分配策略。
- 任何外部记忆系统（如 DMW）的注入方式。

### 5.1 路径与打包安全 (Path & Package Safety)

- `character_file` 与 `user_file` 缺省时分别取 `character.md` 与 `user.md`；显式字段必须是以角色卡根目录为基准的相对路径。
- 路径不得为空、不得为绝对路径、不得包含 `..`，解析后 MUST 位于角色卡根目录内。加载器 MUST 拒绝符号链接以及 ZIP 路径穿越。
- 两个字段 MUST 指向不同的常规 `.md` 文件。文件名比较在大小写敏感与不敏感平台上都不得产生歧义。
- 若实现支持压缩包，解包前 MUST 同时限制文件数量、单文件大小与总解压大小，以防止压缩炸弹。具体上限由运行环境公布，不属于角色卡内容的一部分。
- MO Hub 导入器 MUST 将包视为不可信输入，并在持久化前完成 TOML、路径、编码与大小校验。

---

## 6. 设计原则与未来边界 (Design Principles & Future Boundaries)

### 6.1 1.0.0 的克制

MOMO-STD-0001 v1.0.0 的核心价值在于其 **克制**。它仅定义了一个可被保存、导出、分享和跨客户端加载的最小角色配置资产。这种极简设计确保了标准能够被最广泛的平台与客户端快速实现。

### 6.2 未来标准的预留

为了保持 1.0.0 的纯粹性，以下功能 **MUST NOT** 在本版本中实现，而应作为独立的标准在未来发布：

- **酒馆兼容 (Tavern Compatibility)**：通过独立的 Adapter 标准实现格式转换。
- **记忆系统集成 (Memory Integration)**：通过独立的 Memory Binding 标准定义角色卡如何与 DMW 等记忆系统绑定。
- **多角色场景 (Multi-Character Scenes)**：通过独立的 Scene 标准定义多角色调度。
- **多媒体资源 (Multimedia Resources)**：未来版本或扩展标准中考虑图片、声音等资产的引用。

---

## 7. 结论 (Conclusion)

MOMO-STD-0001 (Character Card Specification v1.0.0) 确立了一个简单、可读、可移植的角色定义标准。通过 `character.toml`、 `character.md` 与 `user.md` 的三文件结构，本标准成功将“角色静态定义”与“系统运行时逻辑”彻底解耦。

角色卡 v1.0.0 不试图成为一个包罗万象的 AI Agent 配置系统。它的唯一使命是提供一个标准化的容器，确保 **“角色是谁”以及“角色面对的用户是谁”** 这一核心资产能够像独立文件一样，被自由地创建、导出、分享与加载。这为构建模块化、高互操作性的 LLM 角色扮演生态奠定了最坚实的基础。
