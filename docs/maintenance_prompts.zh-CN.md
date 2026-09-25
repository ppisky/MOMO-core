# MOMO Prompt Spaces

[English](maintenance_prompts.en.md)

Prompt Spaces 是进程级的具名提示词资源。MOMO Core 仍然随二进制提供经过审查的
Markdown 默认值，但运行值不再只能由编译结果决定。宿主通过原生 HTTP API 替换当前值，
Core 将覆盖值作为内部 JSON 状态持久化在数据根目录下，不从 `momo.toml` 读取提示词正文。

## 固定槽位

| id | 默认职责 |
| --- | --- |
| `assistant` | 通用 System Prompt，默认是 `You are a helpful assistant.` |
| `vision_fallback` | 对话模型不能直接接收图片时，指导视觉模型生成文本描述 |
| `roleplay_director` | 前台角色表演策略 |
| `memory_distillation` | DMW 后台提炼策略 |
| `semantic_graph_governance` | NSG 后台治理策略 |

这些 id 是封闭契约，不允许随意创建新名字，避免拼写错误悄悄产生无人使用的状态。
Prompt Space 不是 Space，没有用户级所有权，也不随 MOC 导入或导出。

## HTTP API

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

只有响应既没有通过治理的 `instructions`、也没有载入非空角色卡时，Core 才使用
`assistant`。角色卡存在时，通用助手提示词会被省略；若策略允许显式请求级
`instructions`，它仍然保留并优先于默认值。

`roleplay_director` 是角色卡响应的静态执行策略，位于最终 System 上下文的
`# Roleplay Direction` 段。它不是 MO State：MO State 产生的是当前场景、认知和状态证据，
而 director 规定模型如何依据角色卡与这些证据完成下一次表演。`roleplay.enabled` 与
`vision.enabled` 属于 `/v1/runtime-settings`；对应提示词正文属于 Prompt Spaces。

## 与宿主配置的边界

宿主可以用任意格式承载面向用户的配置。例如 mobot 可以读取自己的 TOML 提示词设置，
完成宿主侧校验后，在启动或配置协调阶段逐项发送 `PUT`：

```text
用户配置 -> 宿主校验 -> HTTP PUT /v1/prompt-spaces/:id -> MOMO
```

转换逻辑属于 mobot。MOMO 不解析 mobot 配置、不接收其中的提示词文件路径，也不恢复过去
把提示词正文塞回 TOML 的方式。这样所有宿主都能使用同一套 API，mobot 不会成为协议依赖。

## 源码与验证

默认值仍位于 `crates/momo-core/src/product_prompts/`，并由 `include_str!` 编译进 Core。
缺少默认源码会导致编译失败；与此同时，任何部署都可以通过相同 HTTP 契约替换运行值。

在仓库根目录执行：

```bash
cargo test -p momo_core prompt_spaces
cargo test -p momo-server prompt_spaces
```
