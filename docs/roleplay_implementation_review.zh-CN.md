# MOMO Core 角色扮演实现审查

[English](roleplay_implementation_review.en.md)

**审查日期：** 2026-09-05
**审查范围：** 当前工作区（含未提交的 MO State v2、控制协议与维护批次改动）
**性质：** 非规范性工程审查；规范优先级仍以 [`spec_index.md`](spec_index.md) 为准

## 结论

MOMO Core 已经是一套扎实的“角色扮演运行基础设施”，但还不能仅凭现有测试证明它是
一套成熟的“高质量角色扮演产品”。它最强的部分是资产兼容、安全边界、确定性记忆检索、
幂等持久化和 Canon 保护；较弱的部分是完整工具回路、跨 Space 状态一致性、开场资产的
运行时落地，以及对实际角色表现的自动评测。

综合工程评分为 **7.9 / 10**。该分数评价当前实现的完整性、正确性、可维护性和文档对齐，
不等同于模型本身的文风或扮演能力评分。

评分口径：9–10 表示边界清晰且有端到端验证；7–8 表示主路径可靠但存在明确缺口；5–6
表示已有可用基线、尚未完成文档中的完整闭环；低于 5 表示主要仍是接口或设计稿。

## 分模块评分

| 模块 | 评分 | 评价 |
| --- | ---: | --- |
| 角色卡核心与外部兼容 | 8.7 | MOMO v2、CCv1/v2/v3、PNG/CHARX 的识别、限制、来源保留和逆向导出边界清楚；路径、BOM、Frontmatter、APNG、压缩包和未知字段处理有测试。管理 API 仍缺少一次性创建完整 `opening_markdown`/作者 URL 的便捷请求形状。 |
| 会话、消息与结构化控制 | 8.4 | Space 归属、角色切换、删除/清理、幂等冲突和原子响应完成较强。消息存储仍是纯文本三角色模型，无法原生保存图片块和完整工具调用历史。 |
| Prompt / 上下文组装 | 7.7 | 角色、用户、DMW、MO State、NSG 与历史有明确顺序和硬预算，保留最新用户轮次。本次修复后，获准的运行时指令不会作为旧历史被裁掉，多 Space 记忆也保留来源。剩余问题是所有系统区仍整体裁剪，缺少逐区预算和逐区丢失审计。 |
| DMW 长期记忆 | 8.8 | 检索、直接命中、单跳扩展、预算隔离、`touch_at`/注入时间分离、衰减、归档、遗忘 tombstone、事务回滚和 Patch 权限均有较强确定性实现。缺少真实长对话上的误写率、漏写率与召回质量评测。 |
| NSG 叙事语义图 | 8.6 | Draft 隔离、Canon 修订候选、Anchor/向量融合、Auto-Zone、单跳扩展、边白名单和候选去重均已实现并测试。语义质量仍依赖 Governor 模型，当前没有冲突语料集上的 precision/recall。 |
| MO State v2 | 6.7 | 已有按 Space 锁、来源指纹、revision、Operation Journal、Snapshot、Scene 解析和恢复性维护，是可信的 baseline；但规范中的多来源独立版本、跨 DMW/NSG 的统一 Saga、真正有界 Reconciler、软超时执行和持久 `run_id` 暂停/恢复尚未完成。 |
| 多模态角色扮演 | 7.6 | 能根据能力发现选择原图直传或受治理描述回退，限制数量/大小并保存回退 usage，重试不会重复视觉推理。直传图片只对当前轮可见，后续历史只有图片数量标记，长期视觉连续性仍需宿主或记忆层补足。 |
| 工具调用 / Agent 回路 | 5.9 | Gateway 的工具 delta、调用 ID 与输入输出映射可用；本次增加了同会话与严格配对校验，并避免把工具 JSON 伪装成用户历史。但 1.0 仍没有 `requires_action`、暂停点、同一 run 恢复、步数/超时执行或完整 typed tool history。 |
| 配置治理与能力发现 | 8.1 | allow/ignore/reject、参数白名单、能力上限、视觉回退和提示词文件边界清楚。本次删除了官方示例中 Core 实际不执行的旧宿主字段，并明确“可移植保留不等于执行”。 |
| 可移植性、安全与恢复 | 9.0 | MOC v3、加密、LSB、路径穿越防护、单实例锁、原子文件写、持久幂等和来源保留是项目最成熟的部分。 |
| 测试与可观测性 | 7.8 | 当前 workspace 测试覆盖大量失败路径、边界和 mock 端到端流程，且状态/请求审计较完整；仍缺真实 provider smoke、故障注入矩阵与角色扮演行为 eval。 |

## 文档与代码对齐

| 文档面 | 对齐度 | 审查结果 |
| --- | ---: | --- |
| Character Card v2 | 高 | 物理格式、字段边界、安全校验和兼容导入基本一致。`opening.md` 的格式职责明确，但此前没有说明 Core 响应运行时不会自动插入开场；现已写入运行时文档。 |
| DMW v2 | 高 | 主要常量和检索/生命周期规则与实现一致。规范比实现测试更全面之处主要是质量性 SHOULD 和异步策略，而不是相反行为。 |
| NSG v2 | 高 | Canon/Draft、检索、Zone、边与向量缓存边界和代码一致。模型治理质量没有数据集级证据。 |
| MO State v2 | 中 | 文档主体描述完整目标架构，代码是其中的 baseline。实现状态章节此前容易让人误读为工具续跑、多来源版本和跨系统 Saga 已完成；现已明确列为未实现边界。 |
| Native response runtime | 中高 | 幂等、持久化、流、视觉和维护顺序与实现一致。本次补全了消息角色、指令优先位置、角色卡热更新、开场、图片历史和工具续接语义。 |
| Space model | 高 | 归属与访问职责清楚。此前称 Space 权重同时影响“排名和预算”，实际代码只按权重切分 Space 预算、在各 Space 内独立排名；现已纠正。 |
| Portable runtime config | 中高 | Core 实际只执行治理、维护、MO State、视觉和提示词字段。旧 `momo.example.toml` 混入了由宿主拥有且会被 Core 忽略的路由/默认角色/权重/并发字段；现已清理并说明未知字段仅被保留。 |

## 本次直接修正

1. 将通过治理的顶层 `instructions` 放入 `# Runtime Instructions` 系统区，避免它在历史裁剪时
   被静默删除但审计仍显示已应用。
2. DMW/NSG 内容进入最终 RP 上下文时保留 `memory_space` 的 label 与 ID，避免个人与群聊
   事实无来源地混在一起。
3. 自治 MO State 的编译、指纹观察与 Journal 统一使用同一个 managed Space。
4. 结构化 `message` 输入只允许 `user` 角色；系统指令必须经过顶层 `instructions` 治理，
   客户端不能伪造 Core 持有的 assistant/system 历史。
5. 工具续接要求已有 conversation，且每个 `function_call_output` 必须与前置
   `function_call` 按 `call_id` 一对一配对；工具-only 输入不再落成假的用户消息。
6. 角色 CRUD 写入增加与角色卡格式一致的名称、SemVer、作者、URL、文本大小与
   Frontmatter 校验；删除了 HTTP 创建请求中被静默忽略的 `description` 字段。
7. 修订运行时、Space、MO State 和示例配置文档，明确实际执行边界。

## 尚未解决的优先事项

后续进展（2026-09-05）：已新增 [MORP-Bench](../benchmarks/morp/README.md)，
包含原创记忆情境、ACGN 去标签对照、离线 Rust 契约回放、模型/裁判的显式启用脚本、
哈希与断点记录、配对评分和许可过滤。上下文已改为逐区预算并提供区段裁剪审计。
原审查表中的“系统区整体裁剪”与“缺少行为评测流程”由此得到修复；模型评测仍未运行，
人工校准、真实 provider smoke 和下述跨系统状态工程缺口仍然存在。
保留原评分作为审查时点记录，不依据新增脚本就上调实际角色表现分数。

### P1：会直接影响长对话可信度

- 为多 Space MO State Snapshot 记录每个读取来源的独立 DMW/NSG/Scene revision，而不只
  记录 managed Space。
- 设计下一代 typed conversation event 存储，持久保存 tool call/output 与可授权的图片
  描述；在此之前不要声称支持可恢复的完整 Agent run。
- 建立角色扮演行为评测集：角色约束遵循率、跨 50/100/500 轮一致性、DMW 误写/漏写、
  NSG Canon 冲突率、场景切换准确率、群聊私有记忆泄漏率。
- 为 DMW 与 NSG 同时受一个事件影响的情况实现共享 Saga/Outbox operation identity，并做
  两个落盘阶段之间的故障注入测试。

### P2：产品体验与可诊断性

- 增加明确的“创建会话并应用 opening”宿主操作，规定模板替换、消息角色和幂等语义。
- 提供可选的 character revision pin；当前角色卡更新会立即影响所有绑定会话的后续轮次。
- 对真实 provider 做发布门槛中的 credentialed smoke，并加入取消、超时、断流、重复 delta
  和维护模型非法 Patch 的故障矩阵。

## 验证记录

本次审查先运行完整 workspace 测试作为基线，修改完成后又执行了以下发布级检查：

- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过；
- `cargo test --workspace --all-features`：通过，共 203 个测试；
- `cargo doc --workspace --all-features --no-deps`：通过；
- `scripts/test-morp.sh`：通过，58 个框架测试、387 个离线测试点和 21/21 个运行时契约检查，AI 调用为 0；
- `cargo audit`：通过，扫描 `Cargo.lock` 中的 476 个依赖，未发现已知漏洞；
- `git diff --check`：通过，仅报告仓库既有文件的 CRLF/LF 转换提示。
