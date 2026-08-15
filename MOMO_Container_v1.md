**Internal Request for Comments: MOMO-RFC-0003**            July 20, 2026
**Category: Design Direction**
**Status: Draft**

# MOMO-RFC-0003: TOML 配置与 MOC 通用容器方向 (Configuration & Portable Container Direction)

## 摘要 (Abstract)

本文档只确定 MOMO 配置与数据导出的总体方向，不在当前阶段冻结具体字段、文件数量或内部目录。MOMO 的运行配置使用 TOML；`.moc`（MOMO Container）是基于 tar.zstd 的通用可移植容器，可承载用户选择导出或同步的各类 MOMO 数据。相同容器原则同时用于本地导入导出与官方云存储，避免为不同传输渠道维护彼此不兼容的数据格式。

---

## 1. 配置方向 (Configuration Direction)

MOMO 中用于控制运行行为的配置 MUST 使用 TOML 表达。当前阶段不规定配置必须合并为单个文件，也不规定必须拆分为系统、模型或其他固定文件；实际边界应在对应模块进入实现阶段后确定。

未来配置 MAY 包括但不限于：

- 上下文保留、裁剪与压缩策略。
- Provider、模型、协议与能力配置。
- 流式输出开关及流事件处理策略。
- Context Window、输出预留和各类 Token 预算。
- 是否启用记忆系统及其运行方式。
- 记忆提炼、检索与防幻觉所需的 Prompt。
- UI、同步、隐私与其他运行环境选项。

上述项目目前仅代表方向。字段名称、默认值、作用域、配置覆盖优先级、热重载方式与迁移策略 MUST 在实现前通过后续规范确定，本文不得被当作既定 Schema。

角色卡的静态定义继续遵循 `Character_Card_v1.md`，DMW 文件继续遵循 `Dual-Mem_Wiki_v1.md`。它们可以被 `.moc` 承载，但不因此变成运行配置，也不改用 TOML 重写原有内容格式。

---

## 2. MOC 通用容器原则 (MOC General Container Principle)

`.moc` 文件的物理形式确定为 Zstandard 压缩的 tar 归档，即 tar.zstd。它不是“配置文件”的同义词，而是 MOMO 数据的通用容器。

根据用户选择与功能权限，一个 `.moc` MAY 包含：

- TOML 运行配置与模型配置。
- Character Card 及其资源。
- DMW 当前记忆、长期记忆与索引。
- 会话、消息及其必要元数据。
- Prompt、场景或未来标准允许导出的其他模块。

用户 MUST 能够选择需要导出、导入或同步的数据模块。实现不得因为 `.moc` 能够容纳全部数据，就默认收集所有模块。

---

## 3. 本地与云端使用一致性 (Local & Cloud Consistency)

`.moc` 同时服务于以下场景：

1. 本地备份与恢复。
2. 设备间手动迁移。
3. WebDAV 存储。
4. MOMO 官方云同步或云端快照。

本地和云端 MUST 共享同一模块标识、版本概念与数据解释规则。云端 MAY 保存完整 `.moc`，也 MAY 为增量同步存储由相同规则生成的模块级 `.moc`；两种形式恢复后必须得到等价的逻辑数据。

`.moc` 的 tar.zstd 封装只提供归档和压缩，不提供加密。隐私模式下，容器整体或其中的敏感模块 MUST 在上传前应用 `MOMO_v1.md` 定义的客户端加密机制；非隐私模式也必须使用 TLS 传输。

---

## 4. 后续规范必须回答的问题 (Questions for Future Specification)

正式实现 `.moc` 前，后续规范至少需要明确：

- 容器清单、格式版本与应用版本如何表达。
- 各模块在归档中的稳定标识和目录布局。
- 全量快照、增量包与删除记录如何表示。
- 模块之间的引用、依赖和导入顺序。
- 冲突检测、合并、覆盖与回滚规则。
- 单文件、总大小、条目数量及路径安全限制。
- 凭据默认是否排除，以及受保护凭据如何加密。
- 未知模块和更高格式版本的兼容策略。
- 云端对象、WebDAV 文件与本地状态之间的版本映射。

在这些问题完成设计评审前，MOMO SHOULD NOT 将任何临时目录结构声明为稳定的 `.moc` v1 标准。

当前仓库已经提供用于实现验证的 `docs/runtime_config_0_1.md` 与 `docs/moc_profile_0_1.md`。两者状态均为 Experimental Implementation Profile，用来准确记录现有产物和迁移行为，不代表本文所保留的问题已经全部冻结，也不得被宣传为最终 `.moc` v1 标准。

---

## 5. 结论 (Conclusion)

当前阶段只固定两个原则：**运行配置使用 TOML，跨模块数据使用 `.moc` 通用容器**。`.moc` 可以承载角色卡、记忆、模型、会话与其他获准数据，并同时适用于本地和云端；具体 Schema 与内部布局留待各模块实现边界稳定后再定义。
