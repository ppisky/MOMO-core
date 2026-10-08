# 2026-10-02 架构问题修复与验证

本次修复针对原生检索、Space 冷启动、文件恢复、历史读取与多轮证据连续性。
没有调用真实模型；离线耗时和网关契约测试不能证明角色扮演质量提升。

## 实际改动

- provenance/state 纯读走 Worker 只读命令，不再导出全量快照或进入提交路径；版本检查按 Space/module 批量提交，沿用原有权限、更正、撤销和内容哈希规则。
- 注册表预留冷 Space 身份和容量，初始化在请求方执行；同 Space 等待一个初始化结果，已有 Space 地址不等待磁盘恢复。
- journal 仍是权威状态。覆盖有差异的物化文件前，完整 JSON 快照写入 `spaces/<id>/memory-recovery-backups` 并同步；备份失败则阻止覆盖和提交。外部编辑不自动取得记忆权限。README 已纠正相互矛盾的说明。
- 启用历史窗口时，SQL 先从 role/timestamp 元数据计算完整轮次的淘汰边界，只加载保留消息正文及其证据。120 轮含中间 system 消息和待回复 user 的场景，在 12/2 设置下淘汰 330 条，保留 31 条。
- 图片引用/data URL 与用户消息原子写入；完整工具调用与结果随助手锚点原子提交。重启、删除恢复和 MOC 导入导出均保留这些数据，跨 Space 查询仍隔离。
- 工具历史通过文本预算整组保留或丢弃，再转换为原生 assistant/tool 消息。历史图片受当前模型能力与八张图片总配额约束，当前输入优先。丢弃和图片 token 未计入文本估算均有审计。

## 同负载复测

基线为 `5b45b28`，修复版为本次工作区。Windows，Debug，单 Space，离线合成 DMW，NSG/embedding 关闭；每条事件显式绑定同一 conversation/character。查询唯一 tag，每次断言命中预期记录。一轮预热，三次采样取中位数。

| 记录数 | 无身份：基线 → 修复 | 带身份：基线 → 修复 | 带身份加速 |
| --- | --- | --- | --- |
| 50 | 49 → 40 ms | 346 → 49 ms | 7.1 倍 |
| 100 | 78 → 63 ms | 1029 → 80 ms | 12.9 倍 |
| 200 | 138 → 105 ms | 3251 → 137 ms | 23.7 倍 |
| 1000 | 未测 → 478 ms | 未测 → 668 ms | 无基线 |

可复现入口：

```powershell
cargo run --locked -p momo_core --example retrieval_pipeline_benchmark -- 50 100 200 1000
```

原始输出位于 `target/architecture-review-20261002/results.txt` 与 `fixed-results.txt`。
这包含原生 Worker/provenance 链路，不包含模型或 HTTP 耗时，也不是生产 p95/SLA。
低层 memory/vector benchmark 仍不能替代这条链路。

## 场景与故障验证

原生 HTTP 网关测试连续执行图片输入、工具结果续写、重启后追问三次响应：第三次请求仍包含原图片引用、原生 assistant tool call 和配对 tool output，结果没有冒充用户发言。相同 request ID 重放无网络请求且不重复消息。网关是确定性本地替身，不做图片识别或模型质量评分。

其他回归覆盖冷初始化暂停时热地址可用、只读不进入故障注入提交点、在线/离线文件编辑备份、备份失败阻止覆盖、SQL 长历史窗口、预算不足丢弃完整工具组、图片配额/能力限制、删除恢复与 MOC Space 重绑定。已有进程退出、journal 损坏、权限/证据撤销、维护重放测试继续保留。

验证结果：全工作区 all-features 共 361 个 Rust 测试通过，Clippy `-D warnings`、fmt、文档构建和冻结契约 SHA256 校验通过。MORP 的 102 个 Python 测试及 21 个离线运行时契约通过，AI 调用为 0。工具预算回归还覆盖首尾截断删除开头标记的情况：按原始消息位置匹配后，整条证据锚点被丢弃，JSON 尾部不会泄漏到上游。

## 真实模型实验入口与未验证项

增加 `benchmarks/morp/configs/momo-live.example.json`，明确使用 live 历史。填入实际网关、模型和部署版本后，用现有 100/500 轮 dependency presets 创建同模型对照：

```powershell
python -m benchmarks.morp build --suite momo --horizons 100 500 --variants 1 --out target/long-live-dataset
python -m benchmarks.morp scenario-plan target/long-live-dataset --config benchmarks/morp/configs/momo-live.example.json --matrix core --repeats 1 --case-id morp/0.1.2/context_scene_override/en/v0/h100 --case-id morp/0.1.2/context_scene_override/en/v0/h500 --case-id morp/0.1.2/extracted_corrected_commitment/en/v0/h100 --case-id morp/0.1.2/extracted_corrected_commitment/en/v0/h500 --out target/long-live-plans
```

以上是离线计划：三个对照组预计 3612 次候选模型调用，另有维护调用，不执行网络。context/extracted presets 包含会话切换；500 个事件不等于同一 conversation 的 500 轮。真实执行、费用和回答质量必须另行记录，不能把计划称为已完成实验，也不能混入 MORP 1.0 角色扮演主榜。

仍有明确限制：变更命令保留全量 after-image；SQL 淘汰边界仍扫描历史角色元数据；远程图片 URL 可能过期；图片 token 尚无本地精确计数；工具暂停/恢复运行状态不在现有响应契约内。本次修复不声称解决这些边界，也不声称推翻此前质量相同、token 增加的 A/B 结果。
