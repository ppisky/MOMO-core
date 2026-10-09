# 网页资源编辑的 Core 契约

基线为 32c2088d96b93d2d985950c5898fbe556f373d24，修改前暂存区及工作区干净。使用独立 worktree，不修改原 D:/MOMO-core 的既有脏工作区，也不修改 ron。验证显式使用 Rust 1.99.0，未更改仓库声明的工具链。

## 通用能力

- `POST /v1/characters` 接受 CharacterCard 的版本、作者链接和开场白等完整字段，继续复用角色卡校验。
- 新建对话与非空开场白在同一个 SQLite 事务内发布。开场白是正常助手消息，历史、上下文与 MOC 共用；重用同一对话 ID 不重复插入，也不跨上下文改写身份。
- `PUT /v1/memory/assets/configure` 接收 `space_id`、`expected_revision`、`content`、`policy`。content 为 memory（完整 Metadata + body）或 semantic_graph（完整 NsgNode）。新建 revision 为 null；编辑为 provenance 中 revisions 数量，旧未追踪资产为 0。RecordPolicy.revision 仍独立递增。
- 内容和授权在现有 Space worker 暂存快照中执行，全部成功后一次发布 journal。正文成功而策略失败时回滚暂存快照；冲突拒绝部分更新。源证据、原始修订和 NSG 来源角色不可伪造；同图关系目标必须存在。正文编辑不通过模型 patch 间接改写。
- 记忆详情增加完整 `metadata`，避免编辑时遗漏 decay、aliases、relations 等已有字段。`current` 场景继续由状态运行时管理。
- `GET /v1/memory/identity-binding?space_id=…` 返回对话的实际 MemoryIdentity，客户端不能把 conversation_id 猜成共享 continuity_id。
- NSG 列表增加可选 `include_archived`（默认 false），供编辑器归档后恢复；旧调用行为保持。
- MO State 契约读取进入 Space admission/recovery 流程，修复真实对话完成后可能撞上待确认记忆提交的错误。

## 宿主边界与验证

MO State 设置、状态契约、运行状态，以及 MOC 的逐模块 Space 选择/角色选择/兼容格式/口令保护均复用既有契约。没有增加网页专用 Core 配置。网页只负责表单和白名单适配。

通过：`cargo +1.99.0 test -p momo_core --test asset_editing --test default_assistant`（3 项）；契约读取恢复回归（1 项）；四个受影响 crate 的 all-targets clippy `-D warnings`；fmt。真实 Web HTTP + 独立 Core 联调另验证开场白、用户设定、记忆和 NSG 进入模型请求，归档/恢复、删除对话后保留绑定、MO State CAS/继承、契约 CAS、选定与加密 MOC 的真实包内容。详见 Web 的 configuration.verification.ts 与 validation.md。

存储 Space 与个人 Space 保持独立：原子编辑只要求 policy.local_space_id 与目标一致，不强迫 personal_space_id 等于存储 Space。身份查询可显式传 personal_space_id；未传沿用请求空间，适配当前单人宿主。此次测试包含不同个人/存储空间的记录。

Windows 并行链接曾遇到页面文件不足（os error 1455）；随后使用 `cargo +1.99.0 test -j 1` 重跑三项全部通过，未改系统设置、Rust 版本或运行时依赖。
