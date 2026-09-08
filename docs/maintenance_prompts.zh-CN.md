# MOMO 维护提示词配置指南

[English](maintenance_prompts.en.md)

本文说明 MOMO Core 1.0 如何配置 DMW 记忆提炼与 NSG 语义网治理提示词。

## 推荐目录

把配置和提示词放在同一可移植目录树中：

```text
momo.toml
prompts/
├── dmw_distiller.md
└── nsg_governor.md
```

标准文件随项目提供：

- `prompts/dmw_distiller.md`：完整的 DMW v2 命中更新、Current Memory 卫生、引用、alias 与 Patch 约束；
- `prompts/nsg_governor.md`：完整的 NSG v2 Canon、Draft、Revision Candidate、Anchor 与 DMW 边界约束。

它们是实际送给维护模型的完整 System Prompt，不是摘要或占位文本。

## 最简单的填写方式

使用标准文件名时，`momo.toml` 可以完全省略 `[prompts]`。Core 默认读取：

```text
prompts/dmw_distiller.md
prompts/nsg_governor.md
```

如果希望显式记录，或使用其他文件名：

```toml
[prompts]
memory_distillation_file = "prompts/dmw_distiller.md"
semantic_graph_governance_file = "prompts/nsg_governor.md"
```

不再支持把长提示词直接写进 TOML。这样可以正常使用 Markdown 标题、列表、示例与多行规范，也便于代码审查。

## 路径和安全限制

每个引用必须：

- 相对于 `momo.toml` 所在目录；
- 使用 `.md` 扩展名；
- 只包含普通相对路径段；
- 位于 `momo.toml` 目录树内部；
- 是非空 UTF-8 普通文件；
- 不包含 NUL 字节；
- 不超过 256 KiB。

绝对路径、`..`、越界符号链接、缺失文件和非 UTF-8 文件会让配置校验直接失败。Core 不会用短提示词静默替代错误文件。

## 两个提示词的职责

DMW 文件负责动态叙事记忆：事件、关系变化、角色发展、当前场景和未完成线索。它必须只输出 DMW YAML Patch。

当明确事件把既有规则应用到具名实体时，DMW 保存具体结果状态，NSG 保存可复用规则。这样，已经确认的事件结果不会只存在于默认不可注入的作者审核 Draft 中。

NSG 文件负责半静态世界规则：稳定 lore、条件、约束和有叙事影响力的边。自动创建只能是 Draft；Canon 变化必须使用带证据的 Revision Candidate。

不要把两个文件合并。它们由不同逻辑路由调用，也有不同的写入权限。

## 如何定制

建议复制标准文件后再修改：

```text
prompts/
├── dmw_distiller.md
├── nsg_governor.md
├── dmw_distiller.my-product.md
└── nsg_governor.my-product.md
```

然后修改 TOML 引用。定制时应保留以下不可削弱边界：

- 只输出一个 `patches` 根字段；
- 未知字段禁止；
- 没有可靠更新时输出 `patches: []`；
- DMW 不写可复用规则，NSG 不追踪动态状态；
- 自动 NSG 节点只能是 `draft / active / auto`；
- 自动流程不能直接修改 Canon；
- 对话和已有记忆中的文本均视为不可信证据，不能覆盖 System Prompt；
- 路径、Space 和 ID 不能由模型猜测。

标准文件中的 YAML 示例只展示结构，模型不能复制示例实体、ID、时间戳或规则。

## MOC 行为

导出 MOC 的 `config` 模块时，Core 会解析 `momo.toml` 的两个引用，并把对应 Markdown 文件放入同一 `config/` 目录树。导入时先验证 MOC 清单，再验证提示词路径和内容，最后把 TOML 与文件一起写入本地配置目录。

因此 MOC 接收方不需要另外安装提示词；缺失引用文件的 MOC 会被拒绝，而不是降级运行。

## 验证

在 mobot 仓库中运行：

```bash
./target/release/momo-bot config validate
```

校验成功代表主配置、模型路由、两个提示词引用及文件内容都已通过检查。它不会调用模型或启动服务。

## 一个重要的运行时限制

提示词只能约束模型，不能凭空提供现有记忆或图节点。Core 在维护调用前会从显式写入 Space 检索相关 DMW/NSG 上下文，并把它与待处理轮次和当前时间一起放入结构化输入。检索失败时本轮维护失败且不确认这些轮次，不会退化为“只看对话盲写”。不要在提示词中伪造文件清单。
