# MOMO Space、会话与控制动作指南

[English](spaces_and_controls.en.md)

本文面向接入 Core 的宿主开发者，解释 1.0 中最容易混淆的四个对象。

## 四个对象不是一回事

| 对象 | 作用 | 持久化边界 |
| --- | --- | --- |
| Core 实例根目录 | 一次部署的数据库、配置和所有 Space 的容器 | 不是用户 Space |
| 个人 Space | 某个人的 DMW、NSG 与 MO State 来源 | `personal_space_id` |
| 会话 Space | 一段私聊或群聊会话及消息的所有权 | `conversation_space_id` |
| 角色卡 | 可被会话引用和切换的资源 | 全局 `character_id`；可另记 `owner_space_id` |

不存在“角色目录 UUID”。`owner_space_id` 只回答谁管理或导出这张卡；运行时按
`character_id` 查卡。一个群聊会话可以使用个人 Space 的记忆，也可以额外读取群聊
Space 的记忆；角色卡本身不会因此复制到这些 Space。

## 为什么既有个人 Space 又有会话 Space

用户通常长期使用同一个个人 Space。宿主也可按功能配置稳定 Space；切换助手、编辑提示词
或新建对话不要求用户切换空间。功能 Space 表示数据管理与访问边界，并不能证明其中每条
记忆的来源助手、经历参与者或适用情境。功能划分属于宿主策略，不是 Core 固定的目录。

[记忆身份、来源与共享设计](memory_identity_and_provenance.md)采用“默认隔离、显式共享”：
助手身份稳定，提示词可修改；来源、主体与适用范围由记录和证据表达。可以共享用户偏好，
不能把 A 的经历或承诺自动转移给 B。优先共享读取权威记录，避免按功能默认复制多份。
工作树已通过 [Profile 1](memory_provenance_runtime.md) 接入记录策略与资格检查；宿主仍负责 Space 授权。

两者解决不同的问题。个人 Space 让同一用户跨会话保持自己的长期记忆；会话 Space
保证群 A 的消息不会被群 B 当成历史。群聊请求可以明确选择个人和共享记忆源：

```json
[
  {"space_id":"<person>","label":"personal","weight":70,"memory":true,"semantic_graph":true},
  {"space_id":"<conversation-space>","label":"conversation","weight":30,"memory":true,"semantic_graph":true}
]
```

权重只分配检索预算，不授予权限。宿主必须先决定调用者能读哪些 Space。多个 Space
可以同时读，但后台维护每轮只写 `memory_write_space_id` 指定的一个 Space，避免把个人
事实同时污染到群聊，或把群聊共识写进每个成员的私人记忆。

## 新会话、删会话与清记忆

这些动作必须分开：

- 新会话：宿主丢弃 session → conversation 映射；旧会话数据仍存在。
- 删除会话：`delete_conversation` 删除指定会话及消息，然后宿主移除映射。
- 清记忆：`clear_memory` 清一个目标 Space 的 DMW、NSG 或两者；不删除聊天记录。
- 换角色：`switch_character` 改变已有会话引用的 `character_id`；普通回复请求不能偷偷换。

删除会话后，已提炼的 DMW/NSG 仍然保留。当前删除还会留下本地墓碑/删除快照，且不会按
会话清理独立的待提炼队列；队列里的旧问答之后仍可能生成记忆。“删聊天”“停止后续提炼”
和“撤销派生记忆”不能混成同一个保证。详见[当前维护行为](memory_maintenance_current_behavior.zh-CN.md)。

控制请求发送到 `POST /v1/momo/control`，不会经过模型：

```json
{
  "schema": "momo.control/1.0",
  "request_id": "<unique-request-id>",
  "actor_space_id": "<personal-space>",
  "action": {
    "type": "clear_memory",
    "target_space_id": "<personal-or-group-space>",
    "memory": true,
    "semantic_graph": false
  }
}
```

`actor_space_id` 是审计上下文，不会自动形成授权。HTTP 暴露时，鉴权和 Space 访问表仍由
宿主或可信代理负责。

`delete_conversation` 和 `switch_character` 还必须同时给出
`conversation_space_id` 与 `conversation_id`，Core 会校验归属。清理群聊记忆时，宿主把
群聊的 Space UUID 放入 `target_space_id`；清理个人记忆时则放个人 Space UUID。两者是同
一个控制协议，不需要再制造“个人版 Core”或“群聊版 Core”。

已完成的控制操作按 `actor_space_id` 与 `request_id` 持久化重放；相同操作不会再次执行，
同一标识若换成不同动作内容则返回 HTTP 409。直接 CRUD 删除路由属于可信本机管理 profile，
不能替代面向最终用户的控制协议；完整分层见 [HTTP 边界](http_api_1_0.md)。

宿主将用户动作映射为下面的职责。Core 不规定客户端命令或界面标签。

| 意图 | MOMO 接入方式 |
| --- | --- |
| 开始新会话 | 宿主选择或创建会话，并更新自己的 session 映射 |
| 删除会话数据 | `delete_conversation`，携带明确的会话 Space 和会话 ID |
| 清个人或共享记忆 | `clear_memory`，携带目标 Space UUID 和模块选择 |
| 切换角色 | `switch_character`，携带明确的会话和角色 ID |

“清记忆”通过 `memory` 和 `semantic_graph` 布尔字段选择模块。自然语言消息即使写着“忘掉
所有东西”，也不会获得删除能力；必须走上述结构化控制面。

## 权重和写入目标

宿主选择每次 `momo.responses/1.0` 请求里的 `memory_sources` 与
`memory_write_space_id`。每个来源带有明确的 Space UUID、权重和模块开关。
`70/30` 不是相似度阈值或权限比例，而是两个来源竞争有限上下文预算时的相对份额；
每个权重可取 1..100。省略某个会话来源会停止本次请求读取它，但不会删除数据。
写入目标是明确的 Space UUID，不是客户端配置中的枚举；Core 会根据请求来源校验它。

Core 不规定宿主怎样保存或展示这些选择。维护策略使用独立的类型化运行时设置资源，
详见[`runtime_config_0_1.md`](runtime_config_0_1.md)。

## MOC 导入导出

MOC v3 不再接收一个 UUID 后把它套到所有模块。宿主分别选择角色所有权 Space、会话
Space、DMW Space 与 NSG Space。导入默认保留来源 UUID；只有显式 `space_map` 才转换，
并且两个来源不能合并到一个目标。

因此不同 Core 仓库/部署不会天然共享同一份记忆：数据归属于 MOC 中明确列出的 Space。
导入时保留 UUID，表示继续承认同一个 Space；使用一对一 `space_map`，表示宿主明确把它
转换为另一个 Space。Core 实例根目录只是承载多个 Space 的部署目录，不是“角色目录”或
某个人的“MOMO 空间目录”。

## 配置和完整提示词

通用助手、视觉回退、DMW、NSG 与前台角色扮演提示词都具有
`crates/momo-core/src/product_prompts/` 下的 Markdown 默认值，并通过
`include_str!` 编译进 `momo_core`。MOMO 不读取 `momo.toml`；这些提示词也不属于
MOC 或任何 Space。实例级覆盖值通过类型化 `PromptSpaces` Rust API 或 HTTP API 管理，并保存在运行时数据目录中。
详见[Prompt Spaces 说明](maintenance_prompts.zh-CN.md)。

## 2026-09-30 provenance profile update

The working tree implements [memory provenance Profile 1](memory_provenance_runtime.md). This profile supersedes earlier planned-extension notes for identity, record eligibility, scoped scenes, source controls and the 12/2 context window. MOC preserves origin evidence while dropping imported local grants; source Space IDs are not rewritten by `space_map`.
