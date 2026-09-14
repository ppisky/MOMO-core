# MOMO 编译期产品提示词

[English](maintenance_prompts.en.md)

MOMO Core 以可审查的 Markdown 源文件维护三份 System Prompt：

- `crates/momo-core/src/product_prompts/dmw_distiller.md`：DMW 后台维护；
- `crates/momo-core/src/product_prompts/nsg_governor.md`：NSG 治理；
- `crates/momo-core/src/product_prompts/roleplay_director.md`：前台角色表演。

`crates/momo-core/src/product_prompts.rs` 使用 `include_str!` 把它们编译进
`momo_core`。它们是编译期源码输入，不是部署时附带的文件，也不是运行时配置。

## 修改与运行边界

修改产品提示词必须修改 Core 源码、接受审查、重新编译并部署新的二进制。
Core 运行时不会发现或读取 `prompts/` 目录，也不依赖 server 的工作目录。

因此产品提示词：

- 不是 `momo.toml` 中的字段或路径；
- 不能由原生响应请求替换；
- 不随 MOC 的 config 模块导入或导出；
- 不是 Space，也没有 Space 级所有权；
- Core 运行期间不能重新读取或热更新。

这里的“开源”是指完整提示词源码由 Core 仓库追踪并可审查，不代表它必须成为
用户可编辑的运行时文件，也不需要单独建立 Prompt 服务或 Prompt Space。

## 基准测试边界

MORP 中的 `basic_context`、`dmw_nsg` 和 `all_enabled` 是测试计划标签，不是
产品提示词变体或 Cargo feature。所有实验臂使用同一版本的编译期提示词；组件
开关与反事实输入属于测试层，不能让提示词差异或构建差异成为隐藏实验变量。

## 职责

Roleplay Director 负责前台生成，把 Character Card、对话、DMW、NSG 与 MO State
证据转化为场景内表演；它不写记忆，也不能替用户行动。

DMW Distiller 只能为稳定事件、关系、角色发展、当前场景和未完成线索生成 DMW
YAML Patch。NSG Governor 只能提出稳定世界规则和图关系；自动变更保持 Draft，
不能直接修改 Canon。

所有产品控制指令和结构键使用英文。叙事内容可以使用任何语言，并且必须保留来源
文字与含义。Core 不使用针对某一种语言的关键词表，也不通过隐藏的 CJK/Latin
检查决定事实是否有效。

## Core 自身验证

在 MOMO Core 仓库根目录运行：

```bash
cargo test -p momo_core product_prompts::tests::compiled_product_prompts_are_complete
```

该测试验证三份编译期提示词源码，不调用 mobot，也不调用模型。
