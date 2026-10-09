# MOMO 的领域职责与配置边界

**状态：** 当前实现说明与架构决策，不新增 wire 或可移植格式契约  
**核对日期：** 2026-10-02

当前 MOMO 是模块化单体，已经有部分明确的领域规则，但还不能称为完整的 DDD 实现。
本次把职责和术语对齐，并收回没有真实接入需求支撑的通用 FFI 计划。
现阶段提供固定 workspace revision 的 Rust API，以及已记录的 HTTP/SSE 接口。
以后确有其他语言的进程内调用需求时，再由接入方或双方围绕具体场景设计绑定。

本文的“宿主”指接入并配置 Core 的应用，可以是桌面客户端、后端服务或本地适配器，
不指某个特定产品。宿主决定期望配置、授权范围与模型接入；Core 负责校验这些输入，
执行领域规则、响应编排、持久化和故障恢复。用户界面与配置选择不会取代 Core 的一致性职责。

## 什么是领域职责

DDD 应当从业务身份、规则、状态转换和一致性要求出发。拆 crate、换 SQL 存储或使用
worker 本身都不足以证明已经采用 DDD；SQL 也不妨碍实现领域规则。

| 概念 | 当前含义和职责 |
| --- | --- |
| 角色卡 | 有稳定身份的业务定义；原生字段有效性由 `CharacterCard::validate()` 判断 |
| Space | 数据归属与访问请求中的显式标识；可用范围仍由受信任宿主授权 |
| `SpaceWorker` | 串行访问、文件物化和故障恢复的技术所有者；不能直接等同于领域聚合 |
| 记忆身份与证据 | 描述个人 Space、会话、角色、连续性及来源；不由提示词名称或当前默认角色代替 |
| 应用操作 | 检查资源所属范围，协调模型调用、重放、取消和维护工作 |
| 存储事务 | 把必须一起成功的助手消息、证据、维护任务、状态完成和响应重放记录一起提交 |
| 模型/HTTP 适配器 | 处理外部协议及序列化，调用同一组应用操作 |

这次修正了两处具体依赖方向：原生角色创建和更新调用领域对象上的校验规则，应用层只
转换错误；响应应用直接构造类型化 `ResponseEvidenceCapture`，存储不再读取响应审计
JSON 的字段来决定证据身份、类别和配置指纹。存储仍在原来的事务内校验会话身份，并
绑定真正写入的用户/助手消息 ID。审计 JSON 继续用于说明和重放。

角色对象仍是公开可修改的记录；兼容导入保留各自格式的校验路径。这次没有把所有构造、
状态转换或低层写入都改成受聚合保护的方法。应用层和记忆模块仍持有部分业务规则，
完整领域边界需要通过实际用例继续确认。[架构说明](architecture_1_0.md)记录具体所有者。

## “配置”需要按所有者和寿命区分

| 内容 | 所有者与作用范围 | 保存与恢复 |
| --- | --- | --- |
| 数据目录、监听地址、网关与凭据 | 宿主/服务启动 | 宿主管理；不进入角色资产或 MOC |
| 历史窗口、维护阈值、请求覆盖策略 | 一个 `MomoRuntime` 及引用它的服务 | 原子保存到 `runtime-settings.json`，重启恢复；支持稀疏 Space 覆盖 |
| 六个 Prompt Spaces 槽位 | 同一个运行时的具名提示词注册表 | 正常运行时保存到 `<data_dir>/prompt-spaces.json`，重启重新加载 |
| 请求的 instructions、采样与预算 | 当前请求，由运行时策略批准或限制 | 不替换实例设置；接受结果进入相应审计/重放 |
| 角色卡、角色所有的 DDM profile | 明确的业务身份 | 按各自可移植格式契约保存，可进入对应 MOC 模块 |
| 向量、能力观测、恢复记录 | 缓存、观测或运行时恢复机制 | 由各自机制管理，不能统一当作用户配置 |

实例设置由一个运行时持有，支持显式 Space 覆盖；响应按个人 Space 选择，后台维护按写 Space 选择。
共享同一运行时的服务会看到同一组设置和提示词；另一个独立运行时不会共享其内存所有者。
Prompt Space 也不是用户 Space，提示词正文的 SHA-256 revision 不能当作助手身份。

Core 不查找 `momo.toml` 或 `MOMO_CONFIG_PATH`。`momo-config` 是可移植资产所用的 TOML
文档工具。宿主可以采用自己的文件格式，但必须显式把值转换为 Rust API 参数或 HTTP 请求。

模型连接配置也属于宿主接入边界：HTTP 服务启动时读取 `MOMO_MODEL_GATEWAY_ORIGIN` 和
可选的 `MOMO_MODEL_GATEWAY_API_KEY`，Rust 嵌入方则通过 `MomoApiService::new(...)` 注入。
这两个连接参数保存在服务实例内存中，不是 `/v1/runtime-settings` 字段，也不写入 SQLite
或 MOC。实际服务商账号、凭据和模型路由映射由宿主或网关管理，Core 不规定其存储格式。

2026-10-09 接入增量：`momo-server` 提供可选的本机连接目录与管理接口，文件由 HTTP 宿主负责；通过 `MomoApiService::with_model_connections` 注入解析器。`MomoRuntime` 不加载账号文件，Rust 嵌入方可继续使用原网关注入，也可自行提供解析器与存储。详见 [宿主模型连接](model_connections.md)。

## 更新到底意味着什么

`update_runtime_settings(settings)` 和 `PUT /v1/runtime-settings` 都是校验后完整替换。
JSON 省略的字段恢复默认值。例如当前 `history_window.evict_turns` 为 4，随后只发送
`{"vision":{"enabled":true}}`，会让 `evict_turns` 回到默认 2，同时重置其他省略值。
局部更新可使用 PATCH；实例字段置 null 回到类型默认值，Space 字段置 null 清除覆盖并恢复继承。
GET 返回 ETag，PUT/PATCH/DELETE 可用 If-Match 检查全实例递增 revision；旧版本更新返回 409。
所有更新串行校验，先成功持久化再发布内存；重启读取已保存的实例策略和 Space 覆盖。
`/v1/runtime-settings/metadata` 返回有效值、revision 与每字段的实例/Space 来源；
`/v1/runtime-settings/description` 返回所有字段的类型、默认值、枚举及范围。

Prompt Spaces 的 `replace/PUT` 完整替换一个槽位，先成功写文件再发布到内存；`reset/DELETE`
恢复编译期默认值。它们会跨重启保留。宿主配置文件删除某项，并不会自动清除以前保存的
提示词覆盖值；若该文件是宿主的权威配置，宿主还要明确决定哪些槽位应调用 reset。
独立创建的 `PromptSpaces::in_memory()` 不落盘；上述持久化描述对应正常初始化的运行时。

设置与提示词之间没有组合事务。一个响应固定其开始时读取的策略快照，后台维护批次独立捕获策略；提示词槽位仍分别读取。需要干净切换时，
宿主应协调请求进入和维护工作，再完成配置协调。已接受的维护批次、生命周期事件保存
各自捕获的输入或设置供恢复使用，实例策略另由 `runtime-settings.json` 持久化。

## 当前合理性与后续需求

保留 Rust API 与 HTTP 两个入口、让宿主管自己的配置文件、维持必要的 SQL 原子事务，
适合目前本地单写者的接入场景。这些选择依赖明确的使用约束，而不是“已经全面 DDD”
的承诺。配置寿命不同是现有实现的事实，宿主必须按上面的规则协调，不能猜测。

已补齐运行策略持久化、Space 覆盖、PATCH、revision/CAS 与字段来源查询。
仍没有通用 FFI、宿主文件自动重载或设置与提示词组合事务。
DDM 开启后由 MO State 建立经校验的派生 profile 缓存；无需把规则放进 `user.md`，
也不改写角色卡。作者 profile 仍可作为可移植覆盖值，运行时派生缓存不进入 MOC。
具体控制的范围、默认值和生效方式见[对外控制清单](configuration_controls.zh-CN.md)。

详细资源语义见[运行时设置](runtime_config_0_1.md)、[Prompt Spaces](maintenance_prompts.zh-CN.md)；
已有记忆来源与适用性实现见[provenance Profile 1](memory_provenance_runtime.md)。
契约优先级见[规范索引](spec_index.md)。
