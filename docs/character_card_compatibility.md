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

## 2. 外部规范来源

外部角色卡兼容设计只以以下仓库为规范来源：

| 名称 | 规范仓库 | 本仓库参考快照 |
| --- | --- | --- |
| Character Card v1/v2 | <https://github.com/malfoyslastname/character-card-spec-v2> | `docs/spec_v1.md`、`docs/spec_v2.md`，固定于 `8083fb388615ccbce768e97cbbd49d2b3214632c` |
| Character Card v3 | <https://github.com/kwaroran/character-card-spec-v3> | `docs/spec_v3.md`，固定于 `f3a86af019fbd99f788f7a1155f399655b34ab35` |
| Character Foundry CHARX | <https://github.com/character-foundry/character-foundry/blob/master/docs/charx.md> | `docs/character-foundry_charx.md`，固定于 `322fe8d940d1b91c978b43330b80ab2e115002e4` |

外部 CCv2 使用 `spec: "chara_card_v2"` 的 JSON 对象，并可嵌入 PNG；CCv3 使用
`spec: "chara_card_v3"`，规范同时描述 JSON、PNG/APNG 与 CHARX。它们与 MOMO 的
TOML + Markdown 结构不是同一种线格式。

### 来源优先级与声明

1. CCv3 规范中的 MUST/MUST NOT 是 CHARX 基础格式的规范要求；
2. Character Foundry 文档用于补充可互操作的读取/写入行为，包括 JPEG+ZIP、`x_meta`
   与 `module.risum`，不覆盖 CCv3 的强制要求；
3. `x_meta` 与 `module.risum` 是 Risu 兼容扩展，Core 只验证容器安全并原样保留，不解释
   或执行其中内容；
4. 上游 Character Foundry README 声明 MIT，但固定提交没有独立根 LICENSE 文件。
   本仓库保留其版权边界与来源声明，参考快照不重新许可为 Apache-2.0，详见
   `NOTICE` 与 `docs/character-foundry-charx.LICENSE`。

固定的 Character Foundry 文档内部对默认解包上限有一处不一致：选项定义写明
`card.json` 10 MiB、单资产 50 MiB、总量 200 MiB，而末尾安全摘要写成单文件与总量
均 50 MiB、最多 1000 条。相同提交的读取器源码实际使用 10/50/200 MiB 和 10000 条。
MOMO 0.4.0 明确固定为 `card.json` 10 MiB、其他单条目 50 MiB、总展开 200 MiB、最多
10000 条；这些是本实现的安全上限，不解释为 CCv3 线格式要求。

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
compatibility design uses the pinned normative specifications and the declared
Character Foundry CHARX implementation reference above.
Core imports CCv1/v2 JSON and PNG plus CCv3 JSON, PNG/APNG, and CHARX. It
exports CCv2/CCv3 JSON and CHARX. Imported CHARX assets, Risu extension files,
and unknown safe entries survive MOC round trips. PNG/APNG export remains
outside the current profile.
