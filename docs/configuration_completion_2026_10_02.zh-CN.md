# 配置与控制补齐记录

日期：2026-10-02。范围为本轮确认的 Core 配置与控制缺口。

## 已完成

- DDM 归入 MO State：开启 `mo_state.ddm.enabled` 后，没有作者覆盖时从角色名称和角色
  描写提出规则，校验后缓存。输入排除用户描写和会话；缓存按管理 Space/角色隔离，
  重启复用，角色或提示词变化时重建。派生缓存不改写角色卡、不作为 MOC 作者资产导出。
- 运行策略持久化到 `runtime-settings.json`，支持实例默认值、稀疏 Space 覆盖、
  JSON Merge Patch、恢复继承、递增 revision 与 `ETag/If-Match`。失败写入不发布新策略。
- 新增字段描述和来源查询：字段作用、类型、默认值、枚举、范围、有效值及实例/Space 来源。
- 状态契约增加按 Space 的读取、校验替换与重置接口，版本检查与所属 Worker 日志提交。
  自动 DDM 缓存增加读取与重置接口。
- 提案/修复次数、工具续跑步骤和状态/维护超时均实际执行限制；关闭场景管理后拒绝自动
  修改场景与活动线程。已接受的持久提交仍由运行时完成，超时不撤销其提交。
- 前台响应固定开始时的策略，维护使用写 Space 策略；提示词保持独立资源。
  对外清单覆盖当前全部 63 个 `/v1` 路径及其方法，并同步旧配置说明。

## 验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --all-features --locked` | 370 项通过，0 失败；较本轮开始新增 8 项 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo doc --workspace --all-features --no-deps --locked` | 通过 |
| `scripts/test-morp.ps1`，Python 3.13 | 102 项 Python 测试、21 项离线契约检查通过；真实 AI 调用 0 |
| `contracts/1.0/SHA256SUMS` | 8 个冻结协议样例的哈希均匹配 |

新增回归覆盖持久化失败、重启恢复、稀疏继承和交叉约束、旧版本拒绝、状态契约日志提交，
以及 DDM 隔离、用户描写排除、缓存失效、无效提案降级、超时、工具续跑上限与重放。
场景管理测试同时验证开启时接受合法提案、关闭时拒绝。

本轮验证使用模拟网关与离线契约。生产启用 DDM 需要已配置的 `memory_distillation`
模型路由；自动生成的真实模型质量、成本与长程表现需要另行实测。本记录不是稳定版发布
门槛完成声明，也不包含 GUI 或持久 `requires_action` 暂停/恢复协议。

控制入口见[完整清单](configuration_controls.zh-CN.md)，请求格式见
[运行策略 API](runtime_config_0_1.md)，DDM 所有权见[实现状态](ddm_implementation_status.zh-CN.md)。
