# MOMO 对外配置与控制清单

核对日期：2026-10-02。本文按当前服务器路由与类型化设置编写，区分产品控制、
高级本机管理和内部派生文件。Core 不加载宿主的 momo.toml；用户不需要手写 DDM YAML。

## 配置如何生效

启动环境变量负责数据目录、监听、网关和凭据。运行时策略持久化到
runtime-settings.json；默认实例策略可被稀疏 Space 覆盖，再由请求覆盖政策治理。
前台响应使用 personal_space_id 的策略，后台维护使用写 Space 的策略。
GET/PUT/PATCH runtime-settings 保留完整类型化 JSON；ETag/If-Match 检查全实例 revision。
Space DELETE 恢复继承。metadata 额外提供每字段实例/Space 来源；description 完整列出
字段类型、默认值、枚举、数值上下限与交叉约束，宿主应使用它构建设置界面。
未知字段、无效范围、旧 revision 和持久化失败均不会发布新策略。

Prompt Spaces 是六个独立实例槽位，正文持久化到 prompt-spaces.json：assistant、
vision_fallback、roleplay_director、memory_distillation、semantic_graph_governance、
ddm_profile_generation。它们没有与策略一起提交的组合事务。

## MO State 和 DDM

MO State 的 profile、scene_management、injection_mode、memory_lifecycle 和 ddm.enabled
属于运行时策略。DDM 开关默认 false；开启后无作者覆盖时，MO State 从捕获的角色名称
和描写提出、校验并缓存基础规则。用户描写与会话不用于改写基础性格；有效倾向仍每轮
根据场景、记忆和状态计算。缓存按管理 Space/角色隔离，角色内容或生成提示词变化时
重建，不修改角色卡，不作为 MOC 作者资产导出。生成使用既有 memory_distillation
模型路由，可能产生额外调用；次数、usage 和上游请求 ID 进入审计，usage 计入响应总量。没有真实模型质量
提升的新增结论。作者扩展 profile 仍可作为可选覆盖。

max_reconcile_steps 限制每次 DDM 或维护提案的验证/修复尝试；max_agent_steps 限制
初始响应及同一用户轮次的工具续跑步骤；operation_timeout_ms 限制状态操作和每个维护
批次。已接受的持久提交保留取消所有权，超时不回滚已提交内容。宿主仍执行业务工具，
当前没有 requires_action 的持久暂停/恢复协议。

state contract GET/PUT/DELETE 按 Space 管理 YAML 覆盖，经校验、CAS 和 Worker journal
提交。用户可保留内置值，无需日常编辑 config/state_contract.yaml。

## 启动变量

| 变量 | 默认与用途 |
| --- | --- |
| MOMO_DATA_DIR | .momo-data；实例数据目录 |
| MOMO_SERVER_BIND | 127.0.0.1:8765；监听地址 |
| MOMO_SERVER_ALLOW_REMOTE | 默认关闭；显式允许非 loopback 监听 |
| MOMO_MODEL_GATEWAY_ORIGIN | http://127.0.0.1:8788/v1；模型网关 |
| MOMO_MODEL_GATEWAY_API_KEY | 可选，由宿主保管 |
| MOMO_RESPONSE_MAX_CONCURRENCY | 默认 8，范围 1–256 |
| MOMO_RESPONSE_TIMEOUT_SECONDS | 默认 120，范围 1–3600；传输层响应超时 |
| RUST_LOG | 日志过滤；未指定时采用服务器默认值 |

## 当前 HTTP 控制入口

根 /health 也是发现探针。只有三项产品 wire 路由及其版本化契约获得 1.0 稳定面承诺；
其余入口属于可信本机管理，不能直接作为普通用户权限界面。模型调用、导入导出、
删除、审核与恢复是操作，不应全部展示成设置开关。Space 授权仍由宿主负责。

| 方法 | 路径 | 边界 |
| --- | --- | --- |
| `GET` | `/v1/health` | 本机管理/发现 |
| `GET,POST` | `/v1/characters` | 本机管理/发现 |
| `POST` | `/v1/characters/import-external` | 本机管理/发现 |
| `POST` | `/v1/characters/{id}/export-external` | 本机管理/发现 |
| `POST` | `/v1/characters/{id}/export-stored-source` | 本机管理/发现 |
| `GET,PUT,DELETE` | `/v1/characters/{id}` | 本机管理/发现 |
| `GET,PUT,DELETE` | `/v1/characters/{id}/ddm-profile` | 本机管理/发现 |
| `GET,POST` | `/v1/conversations` | 本机管理/发现 |
| `PUT,DELETE` | `/v1/conversations/{id}` | 本机管理/发现 |
| `GET` | `/v1/conversations/{id}/messages` | 本机管理/发现 |
| `POST` | `/v1/messages` | 本机管理/发现 |
| `PUT,DELETE` | `/v1/messages/{id}` | 本机管理/发现 |
| `POST` | `/v1/momo/responses` | 冻结产品 wire |
| `POST` | `/v1/momo/responses/{request_id}/cancel` | 冻结产品 wire |
| `GET` | `/v1/prompt-spaces` | 本机管理/发现 |
| `GET,PUT,DELETE` | `/v1/prompt-spaces/{id}` | 本机管理/发现 |
| `GET,PUT,PATCH` | `/v1/runtime-settings` | 本机管理/发现 |
| `GET,PUT,PATCH,DELETE` | `/v1/runtime-settings/{id}` | 本机管理/发现 |
| `POST` | `/v1/momo/control` | 冻结产品 wire |
| `GET` | `/v1/metrics` | 本机管理/发现 |
| `POST` | `/v1/embeddings/generate` | 本机管理/发现 |
| `POST` | `/v1/capabilities/resolve` | 本机管理/发现 |
| `POST` | `/v1/context/prepare` | 本机管理/发现 |
| `POST` | `/v1/mo-state/compile` | 本机管理/发现 |
| `GET` | `/v1/runtime-settings/metadata` | 本机管理/发现 |
| `GET` | `/v1/runtime-settings/description` | 本机管理/发现 |
| `GET,PUT,DELETE` | `/v1/mo-state/contract` | 本机管理/发现 |
| `GET,DELETE` | `/v1/mo-state/ddm-profile` | 本机管理/发现 |
| `GET` | `/v1/mo-state/runtime` | 本机管理/发现 |
| `POST` | `/v1/memory/retrieve-scoped` | 本机管理/发现 |
| `POST` | `/v1/memory/maintenance` | 本机管理/发现 |
| `GET` | `/v1/memory/recovery` | 本机管理/发现 |
| `POST` | `/v1/memory/recovery/{space_id}/retry` | 本机管理/发现 |
| `POST` | `/v1/momo/maintenance/turns` | 本机管理/发现 |
| `POST` | `/v1/momo/maintenance/drain` | 本机管理/发现 |
| `GET` | `/v1/memory/documents` | 本机管理/发现 |
| `GET` | `/v1/memory/provenance` | 本机管理/发现 |
| `PUT` | `/v1/memory/record-policy` | 本机管理/发现 |
| `PUT` | `/v1/memory/identity-binding` | 本机管理/发现 |
| `POST` | `/v1/memory/evidence/control` | 本机管理/发现 |
| `PUT` | `/v1/memory/default-assistant` | 本机管理/发现 |
| `GET,PUT,DELETE` | `/v1/memory/documents/{id}` | 本机管理/发现 |
| `POST` | `/v1/memory/documents/{id}/archive` | 本机管理/发现 |
| `POST` | `/v1/memory/documents/{id}/restore` | 本机管理/发现 |
| `POST` | `/v1/memory/patches/apply` | 本机管理/发现 |
| `GET,POST` | `/v1/memory/patch-reviews` | 本机管理/发现 |
| `POST` | `/v1/memory/patch-reviews/{id}/approve` | 本机管理/发现 |
| `POST` | `/v1/memory/patch-reviews/{id}/reject` | 本机管理/发现 |
| `PUT` | `/v1/semantic-graph/nodes` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/nodes/list` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/nodes/archive` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/nodes/delete` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/patches/apply` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/pending` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/pending/approve` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/pending/reject` | 本机管理/发现 |
| `GET` | `/v1/semantic-graph/vector-status` | 本机管理/发现 |
| `POST` | `/v1/semantic-graph/vectors/rebuild` | 本机管理/发现 |
| `POST` | `/v1/moc/export` | 本机管理/发现 |
| `POST` | `/v1/moc/import` | 本机管理/发现 |
| `GET` | `/v1/moc/encrypted` | 本机管理/发现 |
| `POST` | `/v1/lsb/embed` | 本机管理/发现 |
| `POST` | `/v1/lsb/extract` | 本机管理/发现 |

详细请求格式：[运行策略](runtime_config_0_1.md)、[Prompt Spaces](maintenance_prompts.zh-CN.md)、
[HTTP 边界](http_api_1_0.md)、[Space 与产品控制](spaces_and_controls.zh-CN.md)。
MOC 作者 profile、角色 TOML/Markdown、DMW/NSG 文档属于可移植业务资产；数据库、
向量缓存、派生 DDM 与评测 JSON 不是终端用户必须编写的运行配置。

本轮实现和验证结果见[配置与控制补齐记录](configuration_completion_2026_10_02.zh-CN.md)。
