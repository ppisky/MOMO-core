# MOMO Container Implementation Profile 0.2

**状态：** Implementation Baseline  
**编码：** tar.zstd  
**格式版本：** 2  
**更新日期：** 2026-08-16

本文记录当前源码实现的 MOC v2 行为。规范字段与扩展规则见
`MOMO_Container_v2.md`。

## 已实现

- 导出使用 `format_version = 2`。
- Manifest 包含包类型、模块定义、依赖、导入顺序、逐文件 SHA-256、可选 sequence
  范围和删除记录结构。
- 模块 ID 为 `config`、`characters`、`conversations`、`memory`、
  `semantic_graph`、`encrypted-container`。
- DMW/NSG 按 `lore/`、`rules/`、`archive/lore/`、`archive/rules/` 前缀分区。
- Character Card v2 导入导出及可选 `opening.md`。
- 非 v2 容器、非 v2 角色卡元数据和非规范模块 ID 会被拒绝。
- 未知模块安全解包并在报告中列出，不写入已知业务数据。
- 更高格式版本明确拒绝。
- tar 路径、重复条目、条目类型、摘要、数量和总大小限制保持启用。
- 私有 MOC 使用 `private/payload.enc` 单文件封装和 512 MiB 上限。
- 0.3.1 的 `export_moc_json` 通过一个结构化请求文档接收输出路径、`scope_id`、
  模块选择、设置与可选密码，不再暴露九个位置参数。

## 当前实现范围

- 导出器生成完整快照包。Manifest 数据模型可以表达 incremental/deletion。
- 未知模块负载会被验证和报告；当前导入流程不会自动把该负载写入后续新建导出包。
- 私有容器解密后仍必须是有效的 v2 MOC。密码只在本次导入内存中使用；调用方负责
  确保日志和临时目录不记录密码。

## 验证

测试覆盖 v2 创建与解包、格式版本拒绝、非规范模块 ID、未知模块报告、重复路径、路径
穿越、摘要、资源上限及 Character Card v2 资源验证。
