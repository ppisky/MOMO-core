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

两者解决不同的问题。个人 Space 让同一用户跨频道保持自己的长期记忆；会话 Space
保证群 A 的消息不会被群 B 当成历史。对于 Discord 群聊，mobot 默认提交两个记忆源：

```json
[
  {"space_id":"<person>","label":"personal","weight":70},
  {"space_id":"<channel>","label":"conversation","weight":30}
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

mobot 把这些控制映射为不同命令：

| 意图 | CLI / 交互 | Discord |
| --- | --- | --- |
| 新建会话映射 | `new` / `/new` | `!momo new` |
| 删除当前会话数据 | `control delete-conversation` / `/delete` | `!momo delete` |
| 清个人记忆 | `control clear-memory --target personal ...` / `/clear-personal` | `!momo clear personal` |
| 清群聊记忆 | `control clear-memory --target conversation ...` / `/clear-conversation` | `!momo clear conversation` |
| 切换角色 | `control switch-character <UUID>` / `/character <UUID>` | `!momo character <UUID>` |

“清记忆”还可选择 `memory`、`semantic-graph` 或 `all`。自然语言消息即使写着“忘掉
所有东西”，也不会获得删除能力；必须走上述结构化控制面。

## 权重和写入目标

这些选择属于宿主的会话策略；宿主把它们转换成每次
`momo.responses/1.0` 请求里的 `memory_sources` 与
`memory_write_space_id`。例如宿主可维护如下自己的配置：

```toml
personal_memory_weight = 70
conversation_memory_weight = 30
conversation_memory_enabled = true
memory_write_target = "personal"
```

`70/30` 不是相似度阈值，也不是权限比例，而是两个来源竞争有限上下文预算时的相对份额。
宿主可以改成其他 1..100 数值。`conversation_memory_enabled = false` 会停止把会话 Space
作为群聊记忆来源，但不会删除数据。`memory_write_target` 只能选 `personal` 或
`conversation`；选后者时若当前运行环境没有会话记忆来源，配置/请求会被拒绝，而不是退回
个人 Space。

这些字段不是 MOMO Core `momo.toml` 的可执行字段。若把它们原样放进可移植配置，容器
往返会保留它们，但 Core 不会据此构造请求；官方可执行字段见
[`runtime_config_0_1.md`](runtime_config_0_1.md)。

## MOC 导入导出

MOC v3 不再接收一个 UUID 后把它套到所有模块。宿主分别选择角色所有权 Space、会话
Space、DMW Space 与 NSG Space。导入默认保留来源 UUID；只有显式 `space_map` 才转换，
并且两个来源不能合并到一个目标。

因此不同 Core 仓库/部署不会天然共享同一份记忆：数据归属于 MOC 中明确列出的 Space。
导入时保留 UUID，表示继续承认同一个 Space；使用一对一 `space_map`，表示宿主明确把它
转换为另一个 Space。Core 实例根目录只是承载多个 Space 的部署目录，不是“角色目录”或
某个人的“MOMO 空间目录”。

## 配置和完整提示词

DMW 与 NSG 的维护 System Prompt 使用 Markdown 引用。使用标准文件名时可以省略
`[prompts]`；Core 读取 `prompts/dmw_distiller.md` 与 `prompts/nsg_governor.md`。仓库中的
两个文件是完整规范提示词，不是 TOML 中的简化摘要。自定义方法和安全限制见
[维护提示词配置指南](maintenance_prompts.zh-CN.md)。
