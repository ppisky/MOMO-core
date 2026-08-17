# 角色卡格式与兼容边界

**状态：** Implemented Profile
**更新日期：** 2026-08-17

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

外部 CCv2 使用 `spec: "chara_card_v2"` 的 JSON 对象，并可嵌入 PNG；CCv3 使用
`spec: "chara_card_v3"`，规范同时描述 JSON、PNG/APNG 与 CHARX。它们与 MOMO 的
TOML + Markdown 结构不是同一种线格式。

## 3. 当前实现状态

| 能力 | 当前状态 |
| --- | --- |
| MOMO Character Card v2 → MOC v2 | 已实现 |
| MOC v2 → MOMO Character Card v2 | 已实现 |
| 外部 CCv1/v2 JSON/PNG → MOMO | 已实现 |
| 外部 CCv3 JSON/PNG/APNG/CHARX → MOMO | 已实现；CHARX 媒体资产不进入核心角色卡 |
| MOMO → 外部 CCv2/CCv3 JSON | 已实现；无来源快照时为有损转换 |
| MOMO → 外部 PNG/APNG/CHARX | 未实现；需要调用方提供媒体载体 |
| 外部未知字段与运行时字段保留 | 已实现；作为来源元数据保存并随 MOC 往返 |

HTTP 接口使用 `POST /v1/characters/import-external` 导入本地文件，使用
`POST /v1/characters/{id}/export-external` 导出 JSON；导出格式为 `ccv2_json` 或
`ccv3_json`。导入器限制文件大小、校验 PNG chunk CRC、限制 CHARX 条目数，并拒绝
格式标识不一致的输入。

```json
POST /v1/characters/import-external
{"input_path":"D:/cards/example.charx"}
```

```json
POST /v1/characters/<id>/export-external
{"output_path":"D:/cards/example.json","format":"ccv3_json"}
```

路径由本机 `momo-server` 读取或写入；HTTP 客户端上传二进制文件不属于该接口。

## 4. English summary

MOMO Character Card v2 is an independent TOML + Markdown format defined by
MOMO-STD-0001. It is not the external `chara_card_v2` wire format. External
compatibility design is based only on the two repositories pinned above.
Core imports CCv1/v2 JSON and PNG plus CCv3 JSON, PNG/APNG, and CHARX. It
exports CCv2/CCv3 JSON and preserves external-only source fields through MOC
round trips. Media-container export remains outside the current profile.
