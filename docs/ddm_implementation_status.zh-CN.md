# DDM 实现状态

[English](ddm_implementation_status.md)

**状态日期：** 2026-09-19

**文档角色：** 非规范性实现报告

**规范：** [`../Dynamic_Disposition_Model_v1.md`](../Dynamic_Disposition_Model_v1.md)

MOMO Core 1.0.0 已完整实现可选的实验性 `momo.ddm/1` 投影契约。这里的
“完整”是指仓库实现了契约和确定性符合性用例，并不表示把实验规范提升为稳定兼容面，
也不等于已经证明真实供应商模型上的行为收益。

## 已实现边界

| 范围 | 当前实现 |
| --- | --- |
| 所有权与传输 | 每个角色一个经验证的 YAML profile，与角色同属；MOC 固定路径 `extensions/momo-ddm/profile.yaml`；管理 API 为 `GET`、`PUT`、`DELETE /v1/characters/{id}/ddm-profile` |
| 运行时启用 | 只保留全局开关 `mo_state.ddm.enabled`；不存在角色到宿主文件的映射，也不新增 DDM Space |
| 计算 | 确定性的 logit 加法和乘法 profile、有限且有界的累计效果、精确的中性边界、缺失输入中性 |
| 治理输入 | 封闭且带类型的 DMW、NSG、MO State、scene 和 request 信号族 |
| 选择 | 具有不可移除预算优先级的作者硬约束、确定性互斥组、top-k，以及 latent/salient/dominant 表达区间 |
| 迟滞 | 前一轮区间按 `(managed_space_id, conversation_id, character_id)` 持久化；仅在 profile revision 与确定性的类型化 profile 指纹同时匹配时复用；删除 profile 会清除历史 |
| 原子性 | 检索正文、DMW/NSG/scene 指纹和观测在同一个有序来源锁窗口内取得；下一轮 DDM 区间及其 profile 身份与 MO State snapshot 在同一 SQLite 事务发布，并校验审计与更新一致性 |
| 模型边界 | 只有作者写的表达提示与硬约束进入 `[STATE_CONTEXT]`；激活值、命中规则、证据 ID、抑制项、区间和指纹仅进入审计 |
| 重放 | request-ID 重放复用已发布 snapshot，不重复计算或叠加调制 |

Character Card v2 核心元数据没有改变。导入导出把 DDM profile 当作可选的角色所有扩展，
旧消费者可以忽略它而不会误读基础角色卡。

## 验证证据

Rust 测试覆盖 profile 解析与拒绝、持久化 revision 上界、精确中性端点、有限乘法累计、
正负调制单调性、确定性重放、来源指纹、scene/request 类型、互斥冲突、top-k、持久化
与跨区间迟滞、profile 身份重置、硬约束预算优先级、跨 scope 隔离，以及 snapshot 与下一轮
区间的原子发布。服务端契约测试还验证了 profile 管理、表达提示实际进入对话
模型请求、下一轮区间持久化、重启重放，以及 `momo.responses/1.0` 输出语义不变。

rc.3 的八题混合上下文 A/B 实跑真正启用了检索和 MO State；direct 与 MOMO 两臂均为
84.375，双方各胜三题、两题平局。这证明链路可执行，但样本过小且结果持平，不能据此
宣称行为收益。本文不声称已经完成 DDM 专属的凭据化供应商反事实实验。

## 实验性限制

- `momo.ddm/1` 仍是实验规范，在稳定化之前可能变化。
- 管理端点是新增管理面，不属于冻结的 `momo.responses/1.0` 响应 wire。
- 行为质量仍取决于作者 profile 与对话模型；确定性符合性证明机制，不代表通用心理学
  有效性或回复质量。
- 稳定版 1.0 的门槛（包括更广的凭据化图片与长程证据）继续由
  [`roadmap_1_0_0.md`](roadmap_1_0_0.md) 跟踪。
