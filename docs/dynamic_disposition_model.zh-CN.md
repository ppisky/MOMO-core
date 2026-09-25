# Dynamic Disposition Model（DDM）非规范性说明

[English specification](../Dynamic_Disposition_Model_v1.md)

DDM 的定位是 **MO State 内部的运行时投影组件**，不是新的记忆层：

```text
角色卡中的稳定倾向
  + 当前情境（场景、请求、关系证据、世界约束）
  + 当前内部状态（生理、认知、关系姿态、情绪）
  -> 本轮有效倾向
  -> 少量表达指引
  -> 模型生成的行为
```

具体所有权如下：

- 角色卡负责相对稳定的倾向及其自然语言定义；
- DMW 负责经历、共享事件和关系证据；
- NSG 负责世界事实、Canon 与规则；
- MO State 负责当前场景、内部状态和一致快照；
- DDM 在该快照上计算一次性的有效倾向；
- 对话模型决定最终措辞和动作。

因此它的位置是：**MO State 完成一致快照之后，State Projector 生成
`[STATE_CONTEXT]` 之前**。rc.3 把结果作为 `## Effective dispositions` 子节注入
现有状态上下文，并把前一轮区间按 Space、会话和角色持久化，用迟滞避免阈值附近抖动。

原始乘法公式可以保留：

```text
E_i(t) = clamp(B_i * M_context_i(t) * M_state_i(t), 0, 1)
```

但两个调制量的中性值必须是 `1`；缺失信号也应视为中性，而不是 `0`。
否则任一输入缺失都会把倾向直接清零，而且 `[0, 1]` 范围内的乘法只能
削弱倾向，不能增强倾向。

正式实现更建议使用有界的 logit 加法：

```text
E_i(t) = sigmoid(logit(B_i) + delta_context_i(t) + delta_state_i(t))
```

正负 delta 分别增强或抑制倾向，最终结果仍在 `[0, 1]`。字段名建议用
`base_activation` 和 `effective_activation`，避免与 DMW 检索权重混淆。

规则、Canon、安全边界、同意边界和用户自主权不应参与“谁权重大”的竞争，
而应作为硬门控。再强的保护倾向也不能替用户决定行动。

有效倾向只存于可重建的 MO State snapshot/audit，不能自动写回角色卡。模型
这一次表现得愤怒，也不能因此永久提高角色的“易怒”基础值。持久性格变化
必须经过独立且明确授权的角色编辑流程。

完整的数据格式、封闭信号族、表达阈值、迟滞、审计字段和稳定化要求见英文规范；
实现证据与剩余限制单独记录在
[DDM 实现状态](ddm_implementation_status.zh-CN.md)。

全局启用开关位于 `PUT /v1/runtime-settings` 的 `mo_state.ddm.enabled`。若 mobot
继续向用户提供 `momo.toml`，由 mobot 将该值翻译成上述 JSON 请求；MOMO 本身不读取
配置文件。角色 profile 由管理 API 维护，并随角色在 MOC 的
`extensions/momo-ddm/profile.yaml` 固定路径传输；它不进入 Character Card v2 核心
元数据，也不新增 DDM Space。实现符合性不改变规范的实验状态。
