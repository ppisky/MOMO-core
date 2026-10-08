# MOMO Prompt Spaces

[English](maintenance_prompts.en.md)

Prompt Spaces 是运行时实例所有的具名提示词资源。MOMO Core 仍然随二进制提供经过审查的
Markdown 默认值，但运行值不再只能由编译结果决定。宿主通过类型化 `PromptSpaces` Rust API
或原生 HTTP API 替换当前值；`MomoRuntime` 将覆盖值持久化到
`<data_dir>/prompt-spaces.json`，不从 `momo.toml` 读取提示词正文。

## 固定槽位

| id | 默认职责 |
| --- | --- |
| `assistant` | 通用 System Prompt，默认是 `You are a helpful assistant.` |
| `vision_fallback` | 对话模型不能直接接收图片时，指导视觉模型生成文本描述 |
| `roleplay_director` | 前台角色表演策略 |
| `memory_distillation` | DMW 后台提炼策略 |
| `semantic_graph_governance` | NSG 后台治理策略 |
| `ddm_profile_generation` | MO State 从角色描述建立 DDM 派生规则的策略 |

这些 id 是封闭契约，不允许随意创建新名字，避免拼写错误悄悄产生无人使用的状态。
Prompt Space 不是 Space，没有用户级所有权，也不随 MOC 导入或导出。

## 默认助手的身份与提示词

这里指默认通用系统提示词的配置入口，不要求在对话中途编辑角色卡；角色卡编辑是另一个
资源操作。提炼/治理提示词也不能修改由代码强制执行的遗忘时间、归档阈值或 Canon 权限。

`assistant` 是实例级提示词槽位，不是默认助手的身份标识。替换或恢复提示词只修改行为配置，
不代表创建新助手、切换角色或重置记忆；返回的 `revision` 也不能用作助手 ID。
宿主把默认选择从 A 改成 B 时，则需要保留 A、B 各自的稳定身份。
此 API 不提供每用户或每助手的独立提示词槽位，覆盖值作用于同一运行时中使用该回退路径的请求。
共享运行时的服务共享注册表；独立构造的运行时不共享其内存所有者。

已经实现的[provenance Profile 1](memory_provenance_runtime.md)定义捕获的历史身份、证据、
记录适用性与宿主所有的默认助手映射。[记忆身份、来源与共享设计](memory_identity_and_provenance.md)
保留更广的目标规则；与当前实现不同之处应以实现 profile 为准。原生响应创建需要有效的
显式角色身份或宿主映射的默认角色身份，不能把提示词槽位本身当作无角色身份的会话入口。

## HTTP API

2026-10-09：本机管理接口 `GET /v1/memory/default-assistant?personal_space_id=<UUID>`
返回当前映射的 `character_id`（未配置为 null）。`POST /v1/memory/default-assistant`
接收 `personal_space_id`、`name`、`author_name`，仅在没有映射时原子创建空人物设定的
助手并绑定；并发和重开复用同一身份，已有映射原样返回。此操作不改变提示词槽位，
也不覆盖用户选定的默认角色。原 `PUT` 仍用于显式改选已有角色。

```http
GET /v1/prompt-spaces
GET /v1/prompt-spaces/assistant
PUT /v1/prompt-spaces/assistant
Content-Type: application/json

{"content":"You answer accurately and concisely."}
```

`PUT` 整体替换一个槽位并返回当前资源。正文不能为空白，最大 262,144 字节。响应中的
`source` 为 `builtin` 或 `override`，`revision` 是当前正文的 SHA-256。

```http
DELETE /v1/prompt-spaces/assistant
```

`DELETE` 只删除覆盖值并恢复编译期默认值，不会删除这个具名槽位。替换无需重启即可被后续
提示词读取看见，并能跨重启保留。Core 先完成持久化再提交内存状态，因此磁盘写入失败时，
不会出现 API 报错但进程却偷偷采用了新值的情况。
独立创建的 `PromptSpaces::in_memory()` 不落盘；上面的跨重启行为对应正常加载的运行时。
内容哈希是指纹，不是递增版本或 CAS 令牌。响应和维护阶段分别读取槽位，没有把所有提示词
与运行时设置一起固定为一个全程快照。

只有响应既没有通过治理的 `instructions`、也没有载入非空角色卡时，Core 才使用
`assistant`。角色卡存在时，通用助手提示词会被省略；若策略允许显式请求级
`instructions`，它仍然保留并优先于默认值。

`roleplay_director` 是角色卡响应的静态执行策略，位于最终 System 上下文的
`# Roleplay Direction` 段。它不是 MO State：MO State 产生的是当前场景、认知和状态证据，
而 director 规定模型如何依据角色卡与这些证据完成下一次表演。`roleplay.enabled` 与
`vision.enabled` 属于 `/v1/runtime-settings`；对应提示词正文属于 Prompt Spaces。

## 与宿主配置的边界

宿主可以用任意格式承载面向用户的配置，校验期望的提示词内容后，在启动或配置协调阶段
通过类型化 Rust API 或 HTTP API 逐项更新槽位：

```text
用户配置 -> 宿主校验 -> PromptSpaces::replace 或 HTTP PUT -> MOMO
```

转换逻辑和本地文件读取属于宿主。MOMO 通过资源 API 接收提示词正文，不规定宿主产品、
配置文件名或提示词文件布局。

从宿主配置文件省略某个提示词，并不会删除已保存的覆盖值。把文件当作权威配置的宿主需要
显式调用 `PromptSpaces::reset` 或 HTTP `DELETE`，让相应槽位恢复默认值。
运行时设置也持久化，并支持 PUT 完整替换、PATCH 与 Space 覆盖；提示词仍按槽位管理。详见
[运行时设置](runtime_config_0_1.md)与[领域及配置边界](domain_and_configuration_boundaries.zh-CN.md)。

## 源码与验证

默认值仍位于 `crates/momo-core/src/product_prompts/`，并由 `include_str!` 编译进 Core。
缺少默认源码会导致编译失败；与此同时，任何部署都可以通过相同 HTTP 契约替换运行值。

在仓库根目录执行：

```bash
cargo test -p momo_core prompt_spaces
cargo test -p momo-server prompt_spaces
```
