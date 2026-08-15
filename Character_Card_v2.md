# MOMO-STD-0001: Character Card Specification v2.0.0

```
Standard: MOMO-STD-0001                              August 04, 2026
Category: Specification
Status: Implementation Baseline (v2.0.0)
```

---

## 摘要 (Abstract)

本文档独立定义 MOMO Character Card（角色卡）v2.0.0。角色卡是纯粹、去中心化且适合自由传播的角色身份资产，由 TOML 元数据、角色 Markdown，以及可选的用户上下文与开场消息组成。

本规范完整规定 v2 的文件结构、字段、身份规则、内容资源、加载安全与外部格式转换。实现方不需要读取其他版本的角色卡规范即可实现 v2。

---

## 1. 引言 (Introduction)

### 1.1 定义 (Definition)

Character Card（角色卡）是一种用于描述 AI 角色交互身份的静态配置格式。它由角色配置文件与角色描述文件组成，并可选包含用户上下文描述与开场消息，旨在为运行环境提供构建角色所需的核心静态上下文。

角色卡是**内容资产**，不是**运行控制资产**。它首先回答的问题是"角色是谁"；在资产提供用户上下文或开场消息时，也可以回答"角色面对的用户是谁"与"角色如何开口"，但不回答"模型应该如何生成文本"。

### 1.2 核心原则 (Core Principles)

- **简单 (Simplicity)**：角色卡必须易于创建、导出与分享。v2.0.0 版本仅包含维持角色身份所需的最小文件集合。
- **可读 (Readability)**：核心描述内容采用 Markdown 格式，允许人类创作者直接阅读与编辑。
- **可移植 (Portability)**：角色卡作为独立资产，不依赖于任何特定的推理模型、宿主客户端、外部记忆系统或中心化平台。
- **去中心化 (Decentralization)**：角色卡的身份与署名不依赖任何中心化服务的认证或存在。角色卡可以被自由复制、分发、存档与 Fork。
- **纯洁 (Purity)**：角色卡仅承载角色身份与交互上下文。运行时控制、平台分类、世界知识注入等逻辑不属于角色卡。

### 1.3 系统边界 (System Boundaries)

角色卡 v2.0.0 MUST NOT 负责以下系统级职责：

- 模型选择与推理参数配置（如 Temperature、Top-P）。
- 会话状态管理与上下文窗口控制。
- 长期记忆（如 DMW）的集成与注入。
- Prompt 编排框架与拼接顺序。
- System Prompt 的构建、注入或管理。
- 世界书（Worldbook / Lorebook / Character Book）的定义与触发。
- 标签、分类、推荐等平台分发元数据的管理。
- 多媒体资源（图片、声音）的管理。
- 越狱（Jailbreak）、防说教（Anti-Preaching）等运行时对齐控制。

上述功能属于运行环境（Runtime Environment）、`.moc` 容器扩展模块或未来独立标准的职责范畴。

---

## 2. 物理文件结构 (Physical File Structure)

一个标准的角色卡资产 MUST 包含角色元数据与角色主体描述，并 MAY 包含用户上下文描述与开场消息：

```text
character_card/
├── character.toml    # 角色卡元数据配置（必需）
├── character.md      # 角色主体描述（必需）
├── user.md           # 角色视角下的用户上下文描述（可选）
└── opening.md        # 开场消息（可选）
```

具体物理存储形式（如保持目录结构、使用角色卡专用归档，或作为角色卡模块装入 `.moc`）MAY 由实现方决定，但逻辑结构 MUST 保持上述两文件必需模型加两个可选文件。

`.moc`（MOMO-RFC-0003）仅作为外层通用容器，不改变角色卡内部规范。

---

## 3. 元数据规范 (Metadata Specification)

角色卡的元数据配置 MUST 使用 TOML 格式，存储于 `character.toml` 中。

### 3.1 必需字段 (Required Fields)

| 字段名       | 类型     | 描述                                                   | 示例                                                |
| --------- | ------ | ---------------------------------------------------- | ------------------------------------------------- |
| `id`      | String | 角色全局唯一标识符。MUST NOT 随名称变化而改变。新建角色 MUST 使用 UUIDv7 URN。 | `"urn:uuid:0190a5f8-7c2d-7b3a-8d9e-01f23456789a"` |
| `name`    | String | 角色的显示名称。                                             | `"雪球"`                                            |
| `version` | String | 角色卡版本号，遵循语义化版本控制 (SemVer)。                           | `"2.0.0"`                                         |

### 3.2 作者字段 (Author Fields)

角色卡 MUST 包含 `[author]` 表。作者信息为纯署名，不依赖任何中心化平台的认证或 UID 绑定。

| 字段名    | 类型     | 约束                | 示例                      |
| ------ | ------ | ----------------- | ----------------------- |
| `name` | String | 作者署名。MUST 为非空字符串。 | `"MOMO Creator"`        |
| `url`  | String | 作者主页或联系方式。可选。     | `"https://example.com"` |

角色卡 MUST NOT 包含以下作者字段：

- `uid`：中心化平台用户标识。
- `display_name`：与 `uid` 绑定的显示名称快照。

作者身份的可信性由分发渠道自行保证。角色卡标准本身不定义作者认证机制。

### 3.3 资源引用字段 (Resource Reference Fields)

| 字段名              | 类型     | 描述                            | 默认值                     |
| ---------------- | ------ | ----------------------------- | ----------------------- |
| `character_file` | String | 角色主体描述文件路径。                   | `"character.md"`        |
| `user_file`      | String | 用户上下文描述文件路径。可选。缺省时表示无专用用户上下文。 | `"user.md"`（仅当文件存在时）    |
| `opening_file`   | String | 开场消息文件路径。可选。缺省时表示无开场消息。       | `"opening.md"`（仅当文件存在时） |

### 3.4 不属于角色卡的字段

以下字段 MUST NOT 出现在 `character.toml` 中：

- `description`：角色简短介绍。属于展示层，由分发平台或外部 Catalog 管理。
- `language`：内容语言。属于展示层或运行环境检测范畴。
- `tags`：标签。属于外部 Catalog 或平台分发元数据，不属于角色身份。
- `system_prompt`：系统提示词。属于运行环境配置。
- `post_history_instructions`：历史后处理指令。属于运行环境配置。
- `first_mes`：首条消息。在 v2.0.0 中由 `opening.md` 承载，不作为 TOML 字段。
- `alternate_greetings`：备选开场白。属于扩展资产，由 `.moc` 容器或运行环境管理。
- `character_book`：世界书 / 角色书。属于外部知识系统，由独立标准定义。
- `creator_notes`：创作者备注。属于展示层或外部元数据。
- `character_version`：外部格式版本号。不等同于 MOMO SemVer。

### 3.5 标识符规则 (Identity Rules)

- 新建角色卡时，生成方 MUST 使用 RFC 9562 UUIDv7 的小写 URN 文本形式作为 `id`。UUIDv7 提供近似创建时间排序与更好的数据库索引局部性，但其时间字段不得替代可信的审计时间戳。
- 导入器 MAY 接受外部资产已有的稳定标识符；创建新的 v2 身份时 MUST 生成 UUIDv7，且不得根据文件名、作者名称或角色名称推导 UUID。
- 导入方发现相同 `id` 与相同或更高 `version` 时，MUST 将其视为同一角色卡的更新或副本，不得静默创建第二个身份；内容冲突时 MUST 请求用户选择保留版本。
- `version` 表示角色卡内容版本，不表示本规范版本。运行环境 MUST 按 SemVer 解析该字段。
- 未知字段 MUST 被解析器忽略并在再次导出时尽可能保留，以便向前兼容；字段类型错误或缺少必需字段时 MUST 拒绝加载并返回可定位的错误。

### 3.6 `character.toml` 完整示例

```toml
id = "urn:uuid:0190a5f8-7c2d-7b3a-8d9e-01f23456789a"
name = "雪球"
version = "2.0.0"

character_file = "character.md"
user_file = "user.md"
opening_file = "opening.md"

[author]
name = "MOMO Creator"
url = "https://example.com"
```

---

## 4. 内容资源规范 (Content Resource Specification)

所有被引用的内容资源 MUST 采用 Markdown 格式。Markdown 文件仅承载自然语言叙事，MUST NOT 包含任何 TOML/YAML 元数据头（Frontmatter），以保持内容层的纯粹性。

所有文本文件 MUST 使用无 BOM 的 UTF-8；读取方 MAY 接受 UTF-8 BOM，但导出时 MUST 移除。换行符 MAY 为 LF 或 CRLF，解析结果不得因换行风格不同而变化。

### 4.1 角色主体描述 (`character.md`)

该文件是角色卡的核心，用于定义角色的交互身份。

内容要求：

- 角色身份与背景。
- 性格特点与行为模式。
- 交流风格与语言习惯。
- 核心行为规则。
- 对话示例（可选）。

标准**不规定**具体的 Markdown 章节标题，具体组织方式由创作者自由决定。

`character.md` MUST NOT 包含以下内容：

- 标签或分类信息。
- 系统提示词或模型控制指令。
- 世界书触发规则或关键词映射。
- 首条消息或开场白。
- 运行时 Prompt 编排逻辑。

示例：

```markdown
# 雪球

你正在扮演雪球。

雪球是一只治愈型猫娘。她温柔、依恋主人，同时保持自己的独立人格。

## 性格

- 温柔体贴，善于察觉他人的情绪变化。
- 偶尔会展现出猫科动物的慵懒与调皮。

## 语言风格

- 句尾偶尔会带上"喵"。
- 语气轻柔，极少使用强烈的否定词。

## 行为规则

- 当用户表现出疲惫时，主动提供情感安慰。
- 保持角色的猫娘设定，不打破第四面墙。
```

### 4.2 用户上下文描述 (`user.md`)（可选）

该文件用于描述角色视角下的交互对象。它是可选文件；没有 `user.md` 时，运行环境 SHOULD 使用中性的默认用户占位，或直接以当前会话用户输入作为交互对象，不得因此拒绝加载角色卡。

重要约束：

- `user.md` 不是用户系统的账号信息（如姓名、密码、个人隐私）。
- 它定义的是"在角色的世界观和当前设定中，与它对话的用户是谁"。
- 当它存在时，它是角色卡资产的一部分，随角色卡一同分发。

示例：

```markdown
# 用户

用户是雪球的主人。

用户喜欢安静的陪伴，经常在工作后感到疲惫。

雪球会主动关心用户的状态，并尝试用轻柔的方式缓解用户的压力。
```

### 4.3 开场消息 (`opening.md`)（可选）

该文件用于承载角色的首条消息（Greeting / First Message）。

性质：

- `opening.md` 是**会话开场资产**，不是角色静态身份的一部分。
- 它定义的是"角色在对话开始时说的第一句话"。
- 它 MAY 包含模板变量（如 `{{user}}`、`{{char}}`），由运行环境在会话初始化时替换。
- 它是可选文件。若不存在，运行环境 MAY 自行生成开场消息或等待用户输入。

约束：

- `opening.md` MUST NOT 包含 Frontmatter。
- `opening.md` MUST NOT 包含角色设定、性格描述或行为规则。
- `opening.md` 的内容 SHOULD 保持原样分发，转换工具 MUST NOT 对其内容进行润色、改写或翻译。

示例：

```markdown
你回来了喵。今天辛苦了吗？
```

或包含模板变量：

```markdown
{{user}}推开门，{{char}}立刻抬起头，耳朵微微颤动。

"你回来了喵。今天辛苦了吗？"
```

---

## 5. 加载与解析规则 (Loading and Parsing Rules)

客户端或运行环境在加载角色卡时，MUST 遵循以下流程：

1. **解析元数据**：读取并解析 `character.toml`，校验 `id`、`name`、`version` 等必需字段。
2. **加载内容资源**：根据 `character_file` 字段读取角色主体 Markdown。若 `user_file` 或 `opening_file` 字段存在且指向有效文件，则一并读取。
3. **移交运行环境**：将解析后的结构化元数据与纯文本内容交由运行环境处理。

运行环境职责：

角色卡规范 MUST NOT 规定以下内容，这些均由运行环境自行决定：

- System Prompt 的构建逻辑与拼接顺序。
- Context Window 的截断与 Token 分配策略。
- 任何外部记忆系统（如 DMW）的注入方式。
- 世界书（Worldbook）的触发与注入。
- 越狱、防说教等运行时对齐控制。
- 标签的检索、过滤或推荐。

### 5.1 路径与打包安全 (Path & Package Safety)

- `character_file` 缺省时取 `character.md`；`user_file` 与 `opening_file` 缺省时，仅在 `user.md` 或 `opening.md` 存在时分别取对应文件。显式字段必须是以角色卡根目录为基准的相对路径。
- 路径不得为空、不得为绝对路径、不得包含 `..`，解析后 MUST 位于角色卡根目录内。加载器 MUST 拒绝符号链接以及 ZIP 路径穿越。
- `character_file` MUST 指向常规 `.md` 文件。若存在，`user_file` 与 `opening_file` MUST 指向不同于 `character_file`、且彼此不同的常规 `.md` 文件。文件名比较在大小写敏感与不敏感平台上都不得产生歧义。
- 若实现支持压缩包，解包前 MUST 同时限制文件数量、单文件大小与总解压大小，以防止压缩炸弹。具体上限由运行环境公布，不属于角色卡内容的一部分。
- 导入器 MUST 将包视为不可信输入，并在持久化前完成 TOML、路径、编码与大小校验。

---

## 6. 与 MOC 容器的关系 (Relationship with MOC Container)

角色卡与 `.moc`（MOMO-RFC-0003）的关系遵循以下原则：

- `.moc` 是通用可移植容器，MAY 承载角色卡模块。
- 角色卡被 `.moc` 承载时，其内部规范不因容器而改变。
- `.moc` MAY 在角色卡模块之外，额外承载以下扩展资产：
  - 运行配置（TOML）。
  - 世界书 / Lorebook（结构化 JSON）。
  - 运行时提示词预设（System Prompt、越狱指令等）。
  - 标签与分类元数据（Catalog）。
  - 酒馆格式原始快照（用于逆向转换）。
  - 备选开场白（`alternate_greetings`）。
- 需要向下兼容或可逆导出的私有信息（如酒馆原始 JSON、酒馆 `extensions`、额外展示字段、供应方自定义字段）SHOULD 放在 `.moc` 的独立扩展模块中，而不是写入角色卡核心文件。推荐使用 `extensions/<namespace>/...` 一类独立目录；导入器可以保留并报告未知扩展模块，但不能把它们当作角色身份的一部分。
- 运行环境 MUST 能够在仅加载角色卡模块的情况下正常工作，即使 `.moc` 中的扩展资产缺失。
- 角色卡的加载与解析 MUST NOT 依赖 `.moc` 中的扩展资产。

---

## 7. 设计原则与未来边界 (Design Principles & Future Boundaries)

### 7.1 v2.0.0 的克制

MOMO-STD-0001 v2.0.0 的核心价值在于其**克制**与**去中心化**。

它仅定义了一个可被保存、导出、分享和跨客户端加载的最小角色身份资产。它不依赖任何中心化平台，不承载任何运行时控制逻辑，不包含任何平台分发元数据。

这种极简设计确保了角色卡能够像文本文件一样被自由复制、分发、存档与 Fork。

### 7.2 不属于角色卡的资产

以下资产明确不属于角色卡，MUST NOT 被写入 `character.toml`、`character.md`、`user.md` 或 `opening.md`：

| 资产                    | 归属                | 说明                   |
| --------------------- | ----------------- | -------------------- |
| 标签 (Tags)             | 外部 Catalog / 平台   | 标签是发现与分类元数据，不构成角色身份。 |
| 简介 (Description)      | 外部 Catalog / 平台   | 简介是展示层信息，不是角色核心描述。   |
| 语言 (Language)         | 外部 Catalog / 运行环境 | 语言可由客户端检测或由平台标注。     |
| 系统提示词 (System Prompt) | 运行环境 / `.moc` 扩展  | 系统提示词是运行控制，不是角色身份。   |
| 越狱 / 防说教指令            | 运行环境 / `.moc` 扩展  | 对齐控制是运行环境的职责。        |
| 世界书 (Worldbook)       | 独立标准 / `.moc` 扩展  | 世界书是动态知识注入系统，不是静态身份。 |
| 备选开场白                 | `.moc` 扩展         | 多开场白属于会话体验扩展。        |
| 多媒体资源                 | 未来标准              | 图片、声音等由未来标准定义。       |

### 7.3 未来标准的预留

以下功能 MUST NOT 在本版本中实现，而应作为独立的标准在未来发布：

- **世界书 / 知识绑定 (Worldbook / Lore Binding)**：通过独立标准定义角色卡如何与世界书等知识系统绑定。
- **多角色场景 (Multi-Character Scenes)**：通过独立的 Scene 标准定义多角色调度。
- **多媒体资源 (Multimedia Resources)**：未来版本或扩展标准中考虑图片、声音等资产的引用。
- **外部 Catalog 与标签 (External Catalog & Tagging)**：通过独立标准定义标签、分类、推荐等平台分发元数据的管理。

---

## 8. 结论 (Conclusion)

MOMO-STD-0001 (Character Card Specification v2.0.0) 确立了一个简单、可读、可移植、去中心化的角色定义标准。通过必需的 `character.toml`、`character.md`，以及可选的 `user.md` 与 `opening.md`，本标准将"角色静态定义"与"系统运行时逻辑"彻底解耦，并进一步将"角色身份"与"平台分发元数据"彻底解耦。

角色卡 v2.0.0 不试图成为一个包罗万象的 AI Agent 配置系统。它的唯一使命是提供一个标准化的、纯粹的、可自由传播的容器，确保"角色是谁"，以及在需要时"角色面对的用户是谁"、"角色如何开口"这些核心资产能够像文本文件一样，被自由地创建、导出、分享与加载。

---

## 附录 A：酒馆格式转换指南 (Tavern Format Conversion Guide)

本附录为参考性（Informative），定义 MOMO 角色卡与常见酒馆（Tavern / SillyTavern）角色卡格式之间的转换规则。

### A.1 格式识别

| 输入特征                                                        | 判定                       | MOMO 行为 |
| ----------------------------------------------------------- | ------------------------ | -------- |
| 顶层包含 `name`、`description` 等字段，无 `spec` 字段                   | Tavern V1（Legacy）        | 按 V1 字段映射，缺失字段按空字符串处理 |
| 顶层包含 `spec: "chara_card_v2"`，核心字段位于 `data` 内                | Tavern V2                | 按 V2 `data` 字段映射 |
| 顶层包含 `spec: "chara_card_v3"`，核心字段位于 `data` 内                | Character Card V3 / CCv3 | 按 V3 的 V2 超集字段映射，V3 新字段进入扩展资产 |
| PNG/APNG 内存在 `ccv3` tEXt chunk                               | CCv3 embedded image      | 优先读取 `ccv3` |
| PNG/APNG 内仅存在 `chara` / `Chara` 元数据                         | V1/V2 embedded image     | 读取并按 V1/V2 判断 |
| `.charx` ZIP 根目录存在 `card.json`                                | CCv3 CHARX               | 读取 `card.json`，资产按扩展资产保留 |
| 包含未知字段、未来 `spec_version`、非空 `extensions` 或应用私有 JSON / 资产文件 | 扩展 / 未来版本                | 核心字段照常导入，未知内容原样保留至 `.moc` 扩展模块 |

MOMO 转换器 MUST 把输入文件视为不可信数据：PNG/APNG/CHARX/JSON 解包前必须限制文件数量、单文件大小、总大小、路径穿越、符号链接、压缩炸弹与编码错误。若同一输入同时存在 V2 `chara` 与 V3 `ccv3`，SHOULD 优先使用 V3 `ccv3`，并将被忽略的旧块作为原始快照保留。

### A.2 字段映射表

| 酒馆字段                        | MOMO 处理                                          | 目标位置                               |
| --------------------------- | ------------------------------------------------ | ---------------------------------- |
| `name`                      | 必需映射                                             | `character.toml` → `name`          |
| `creator`                   | 映射为作者署名                                          | `character.toml` → `[author] name` |
| `description`               | 转为角色身份描述                                         | `character.md`                     |
| `personality`               | 转为性格特点                                           | `character.md`                     |
| `scenario`                  | 拆分：角色世界观进 `character.md`；用户关系 / 场景前提可进 `user.md` | `character.md` / 可选 `user.md`      |
| `mes_example`               | 转为对话示例或语言风格示例                                    | `character.md`                     |
| `first_mes`                 | 保留为开场消息                                          | `opening.md`                       |
| `alternate_greetings`       | 不进核心，保存至 `.moc` 扩展                               | `.moc` 扩展模块                        |
| `character_book`            | 不进核心，保存为结构化世界书                                   | `.moc` 扩展模块                        |
| `system_prompt`             | 不进核心，保存为运行时预设                                    | `.moc` 扩展模块                        |
| `post_history_instructions` | 不进核心，保存为运行时预设                                    | `.moc` 扩展模块                        |
| `tags`                      | 不进核心，写入外部 Catalog                                | 外部 Catalog                         |
| `creator_notes`             | 不进核心，可作为外部说明                                     | 外部 Catalog / `.moc` 扩展             |
| `character_version`         | 可参考，不强制等同 MOMO `version`                         | 转换日志 / warning                     |
| `extensions`                | 原样保留                                             | `.moc` 扩展模块                        |
| `nickname`                  | 不覆盖 `name`；作为酒馆运行时别名保留                            | `.moc` 扩展模块                        |
| `creator_notes_multilingual` | 不进角色身份；作为多语言展示说明保留                              | 外部 Catalog / `.moc` 扩展             |
| `source`                    | 不进角色身份；作为来源元数据保留                                  | 外部 Catalog / `.moc` 扩展             |
| `assets`                    | 不进角色卡核心；若实现支持头像/背景等媒体，作为外部资产引用或文件保留             | `.moc` 扩展模块 / 未来多媒体标准             |
| `group_only_greetings`      | 不进核心开场白；作为群聊专用开场扩展保留                            | `.moc` 扩展模块                        |
| `creation_date` / `modification_date` | 不作为可信审计时间；仅作为来源元数据保留                      | 外部 Catalog / `.moc` 扩展             |
| 未知字段                        | 不猜测，保留或记录 warning                                | `.moc` 扩展 / 转换日志                   |

### A.3 转换规则

- 转换工具 MUST 保留原始内容的语言，MUST NOT 在转换过程中进行翻译。
- 转换工具 MUST NOT 编造原始输入中不存在的设定、性格、背景、关系或剧情。
- `first_mes` MUST 原样保留至 `opening.md`，MUST NOT 润色、改写或翻译。
- `character.md` 与可选 `user.md` 中的 `{{user}}` SHOULD 替换为"用户"，`{{char}}` SHOULD 替换为角色名。`opening.md` 中 MAY 保留原始模板变量。
- V1 的 `<BOT>` / `<USER>` 与 V2/V3 的 `{{char}}` / `{{user}}` 都是运行时模板变量。转换到核心 Markdown 时 MAY 做中性替换；为支持逆向导出，原始模板形式 MUST 在扩展快照中保留。
- `character_book` MUST NOT 被展开、改写或合并进 `character.md`。世界书是结构化触发系统，展开为自然语言会丢失关键词触发、优先级、常驻状态等语义。
- `tags` MUST NOT 被写入 `character.toml` 或任何角色卡文件。
- `system_prompt` 与 `post_history_instructions` MUST NOT 被写入角色卡文件。
- 若酒馆卡缺少 `user.md` 对应的用户信息，转换工具 MAY 省略 `user.md`；若为了兼容旧运行环境而生成最小中性描述（如"用户是与 {name} 进行对话的人"），MUST NOT 编造用户姓名、性别、年龄或身份。
- 转换工具 SHOULD 保留酒馆原始 JSON、PNG/APNG 元数据块、CHARX `card.json`、CHARX 资产清单与未消费资产于 `.moc` 扩展模块中，以支持逆向导出。

### A.4 模型辅助转换提示词

酒馆卡到 MOMO 角色卡的转换 MAY 使用大模型辅助，尤其是 `scenario` 拆分、`description` / `personality` / `mes_example` 归并、V3 `assets` 说明归类等人工规则容易不稳定的部分。

模型输出 MUST 被视为转换建议，而不是可信事实来源。转换器 MUST 保留原始输入，MUST NOT 允许模型编造新设定、翻译原文、删除未知字段或决定安全策略。建议使用结构化 JSON 输出，并在写入文件前由程序校验。

推荐英文系统提示词：

```text
You convert Tavern / SillyTavern character cards into MOMO Character Card v2 draft files.

Rules:
- Preserve the source language exactly. Do not translate.
- Do not invent facts, relationships, memories, traits, scenes, or examples.
- Separate stable character identity from runtime controls.
- Put character identity, personality, speaking style, and examples into character_markdown.
- Put only explicit user-facing relationship or scene premise into user_markdown. If the source does not provide such information, return null.
- Put first_mes into opening_markdown verbatim.
- Do not include system_prompt, post_history_instructions, lorebooks, tags, creator notes, alternate greetings, assets, or application extensions in the core Markdown.
- Return every non-core or unsupported field under extension_notes without changing its value.
- Keep template variables such as {{char}}, {{user}}, <BOT>, and <USER> visible unless the caller explicitly asks for neutral replacement.

Return JSON with:
{
  "name": string,
  "author_name": string | null,
  "character_markdown": string,
  "user_markdown": string | null,
  "opening_markdown": string | null,
  "extension_notes": {
    "runtime": object,
    "catalog": object,
    "lorebook": object | null,
    "assets": array,
    "unknown": object,
    "warnings": string[]
  }
}
```

推荐英文用户提示词模板：

```text
Convert this character card to MOMO Character Card v2.

Source format: {{source_format}}
Source JSON:
{{source_json}}

If any field is ambiguous, keep it in extension_notes.warnings instead of guessing.
```

### A.5 `.moc` 扩展模块建议

酒馆兼容信息 SHOULD 使用独立扩展模块承载，推荐模块 ID 为 `tavern_compat`，根目录为 `extensions/tavern/`：

```text
extensions/tavern/
├── original.json              # 原始 V1/V2/V3 JSON 或从图片/CHARX 中提取的 card.json
├── manifest.toml              # 转换器名称、来源格式、警告、保留策略
├── runtime.json               # system_prompt、post_history_instructions、alternate_greetings 等
├── catalog.json               # tags、creator_notes、source、日期等展示/来源元数据
├── lorebook.json              # character_book / Lorebook 原始结构
└── assets/                    # CHARX 或图片扩展资产，路径保持相对且安全归一化
```

导入器 MAY 不理解 `tavern_compat` 的内部语义，但 MUST 在安全校验通过后保留并报告该扩展模块。核心角色卡加载 MUST NOT 依赖该模块；缺失该模块时，角色仍应能作为普通 MOMO Character Card v2 加载。

### A.6 逆向导出 (MOMO → Tavern)

- 逆向导出为有损转换（Lossy Conversion）。
- `character.md` 内容需拆分回 `description`、`personality` 等字段。
- `opening.md` 映射回 `first_mes`。
- `tags` 从外部 Catalog 读取。
- `system_prompt`、`post_history_instructions`、`alternate_greetings`、`character_book`、`extensions`、V3 `assets` 等字段优先从 `tavern_compat` 扩展模块读取；若不存在，留空、使用安全默认值，或由导出工具明确标记为缺失。
- 导出工具 SHOULD 在导出结果中标记有损字段。

---

