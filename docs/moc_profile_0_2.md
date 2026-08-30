# MOMO Container Implementation Profile 0.2

**状态：** Implementation Baseline  
**编码：** `.moc`（tar 归档经 Zstandard 压缩）
**格式版本：** 2  
**更新日期：** 2026-08-25

本文记录当前源码实现的 MOC v2 行为。规范字段与扩展规则见
`MOMO_Container_v2.md`。

## 已实现

- 导出使用 `format_version = 2`。
- Manifest 包含模块定义、依赖、导入顺序与逐文件 SHA-256。
- 模块 ID 为 `config`、`characters`、`conversations`、`memory`、
  `semantic_graph`、`tavern_compat`、`encrypted-container`。
- `config` 模块包含 `momo.toml` 及其引用的 DMW/NSG Markdown 提示词；导入导出会
  校验相对路径、UTF-8、文件类型、大小与清单摘要，不使用内联回退。
- DMW/NSG 按 `lore/`、`rules/`、`archive/lore/`、`archive/rules/` 前缀分区。
- MOMO 独立 Character Card v2 导入导出及可选 `opening.md`。
- 外部角色卡原始字段按角色 ID 保存在 `tavern_compat` 模块；CHARX 来源另以
  `source.charx` 保存，并由 `source.json` 中的 SHA-256 与结构摘要绑定。
- 非 v2 容器、非 MOMO v2 角色卡元数据和非规范模块 ID 会被拒绝。
- 未知模块安全解包并在报告中列出，不写入已知业务数据。
- 更高格式版本明确拒绝。
- tar 路径、重复条目、条目类型、摘要、数量和总大小限制保持启用。
- 私有 MOC 使用 `private/payload.enc` 单文件封装和 512 MiB 上限。
- `export_moc_json` 通过一个结构化请求文档接收输出路径、`scope_id`、
  模块选择、兼容 profile、设置与显式 protection 类型。

## 当前实现范围

- MOC v2 只生成和接收完整的已选模块快照；不声明 incremental/deletion 能力。
- 未知模块负载会被验证和报告，且只在宿主显式提供认领目录时复制到该目录；Core 不解释、
  执行或写入已知业务数据。宿主也可在导出时显式提供扩展模块目录、依赖和导入顺序。
  未认领的临时负载在导入结束后丢弃，不会自动进入后续导出包。
- 私有容器解密后仍必须是有效的 v2 MOC。密码只在本次导入内存中使用；调用方负责
  确保日志和临时目录不记录密码。

## 验证

测试覆盖 v2 创建与解包、格式版本拒绝、非规范模块 ID、未知模块报告、重复路径、路径
穿越、摘要、资源上限及 MOMO Character Card v2 资源验证。外部 CCv2/CCv3 的直接
JSON/PNG/CHARX 导入及 CCv2/CCv3 JSON/CHARX 导出由角色卡兼容层实现，详见
[`character_card_compatibility.md`](character_card_compatibility.md)。
