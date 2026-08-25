# 角色卡格式与兼容边界

**状态：** Implemented Profile
**更新日期：** 2026-08-25

## 1. MOMO 独立角色卡

MOMO Character Card v2 是 MOMO-STD-0001 定义的独立格式：

```text
character_card/
├── character.toml
├── character.md
├── user.md           # optional
└── opening.md        # optional
```

其规范版本是 MOMO 自己的版本号，不表示外部 `chara_card_v2`。当前 Core 将该格式作为
MOC v2 的 `characters` 模块导入和导出，并在运行时归一化为 `CharacterCard` 领域对象。

## 2. 外部规范与实现来源

规范与实现参考严格分层：

| 层级 | 名称 | 上游来源 | 本仓库处理 |
| --- | --- | --- | --- |
| 规范 | Character Card v1/v2 | [`spec_v1.md`](https://github.com/malfoyslastname/character-card-spec-v2/blob/8083fb388615ccbce768e97cbbd49d2b3214632c/spec_v1.md)、[`spec_v2.md`](https://github.com/malfoyslastname/character-card-spec-v2/blob/8083fb388615ccbce768e97cbbd49d2b3214632c/spec_v2.md) | 上游未声明许可证，只链接，不再分发全文快照 |
| 规范 | Character Card v3 | [`SPEC_V3.md`](https://github.com/kwaroran/character-card-spec-v3/blob/f3a86af019fbd99f788f7a1155f399655b34ab35/SPEC_V3.md) | `docs/spec_v3.md`，固定于 `f3a86af019fbd99f788f7a1155f399655b34ab35`，MIT |
| 事实参考实现 | RisuAI | [`src/ts/characterCards.ts`](https://github.com/kwaroran/Risuai/blob/c0ed1026de4b06a1c4600b79c789fea0616c297c/src/ts/characterCards.ts) | 固定核对 `c0ed1026de4b06a1c4600b79c789fea0616c297c`；GPL-3.0 源码只用于确认互操作行为，不包含或翻译其代码 |
| 下游兼容参考 | Character Foundry CHARX | <https://github.com/character-foundry/character-foundry/blob/322fe8d940d1b91c978b43330b80ab2e115002e4/docs/charx.md> | 只链接；固定提交没有根 LICENSE，不分发全文快照 |

外部 CCv2 使用 `spec: "chara_card_v2"` 的 JSON 对象，并可嵌入 PNG；CCv3 使用
`spec: "chara_card_v3"`，规范同时描述 JSON、PNG/APNG 与 CHARX。它们与 MOMO 的
TOML + Markdown 结构不是同一种线格式。

### 来源优先级与声明

1. CCv2/CCv3 文档中的规范关键词决定基础线格式；
2. RisuAI 源码只用于识别事实上的 Risu 扩展：CHARX-JPEG、`x_meta` 与
   `module.risum`，这些行为不覆盖 CCv3 的要求；
3. Character Foundry 是更晚出现的下游兼容库，只用于交叉检查，不作为协议授权或
   规范来源；
4. Core 对扩展内容只做容器安全验证和原样保留，不解释或执行 `module.risum`；
5. MOMO 固定 `card.json` 10 MiB、其他单条目 50 MiB、总展开 200 MiB、最多 10000
   条。这些是 MOMO 自己的防御性实现上限，不是 CCv3 线格式要求。

本实现是在阅读公开规范与上述公开实现资料后编写，因此不声明为 clean-room 实现。
仓库未包含逐字复制或机械翻译的 RisuAI 源码，但这项事实性声明不等于对衍生作品问题作出
法律结论。复用或分发者仍应自行完成许可证评估。

## 3. 当前实现状态

| 能力 | 当前状态 |
| --- | --- |
| MOMO Character Card v2 → MOC v2 | 已实现 |
| MOC v2 → MOMO Character Card v2 | 已实现 |
| 外部 CCv1/v2 JSON/PNG → MOMO | 已实现 |
| 外部 CCv3 JSON/PNG/APNG/CHARX → MOMO | 已实现；CHARX 原始容器及安全条目独立保存 |
| MOMO → 外部 CCv2/CCv3 JSON | 已实现；无来源快照时为有损转换 |
| MOMO → 外部 CHARX | 已实现；无来源资产时输出仅含 `card.json` 的标准 ZIP |
| 导入 CHARX → MOC → CHARX | 已实现；资产、`x_meta`、`module.risum` 与未知安全条目往返 |
| MOMO → 外部 PNG/APNG | 未实现；需要调用方提供媒体载体 |
| 外部未知字段与运行时字段保留 | 已实现；作为来源元数据保存并随 MOC 往返 |

HTTP 接口使用 `POST /v1/characters/import-external` 导入本地文件，使用
`POST /v1/characters/{id}/export-external` 导出；格式为 `ccv2_json`、`ccv3_json` 或
`ccv3_charx`。CHARX 导入器验证重复/危险路径、符号链接、加密条目、条目数、单文件与
总展开大小，并完整读取条目以触发 CRC 校验。标准 ZIP 与 JPEG+ZIP 均可读取；输出固定
重建为标准 ZIP，不承诺压缩字节、时间戳或条目顺序与来源逐字节一致。

```json
POST /v1/characters/import-external
{"input_path":"D:/cards/example.charx"}
```

```json
POST /v1/characters/<id>/export-external
{"output_path":"D:/cards/example.charx","format":"ccv3_charx"}
```

路径由本机 `momo-server` 读取或写入；HTTP 客户端上传二进制文件不属于该接口。

## 4. English summary

MOMO Character Card v2 is an independent TOML + Markdown format defined by
MOMO-STD-0001. It is not the external `chara_card_v2` wire format. External
compatibility design uses the normative CCv2/CCv3 documents. RisuAI is cited
only as the GPL-3.0 de facto implementation of Risu-specific extensions, while
Character Foundry is a downstream cross-check. Unlicensed upstream documents
are linked rather than redistributed. This is not represented as a clean-room
implementation; the provenance disclosure is factual and does not make a legal
conclusion about derivative-work status.
Core imports CCv1/v2 JSON and PNG plus CCv3 JSON, PNG/APNG, and CHARX. It
exports CCv2/CCv3 JSON and CHARX. Imported CHARX assets, Risu extension files,
and unknown safe entries survive MOC round trips. PNG/APNG export remains
outside the current profile.
