# 角色卡格式与兼容边界

**状态：** Implementation Boundary
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
| 外部 CCv2 JSON/PNG → MOMO | 未实现；仅有转换设计 |
| 外部 CCv3 JSON/PNG/CHARX → MOMO | 未实现；仅有转换设计 |
| MOMO → 外部 CCv2/CCv3 | 未实现；设计上属于有损转换 |

因此，文档不得把“支持 MOMO Character Card v2”简写成可能被理解为“支持外部
CCv2”的表述。只有实现并测试对应解析器、字段保留、安全限制和逆向导出后，才能声明
对外部格式兼容。

## 4. English summary

MOMO Character Card v2 is an independent TOML + Markdown format defined by
MOMO-STD-0001. It is not the external `chara_card_v2` wire format. External
compatibility design is based only on the two repositories pinned above.
Current Core imports and exports the MOMO format through MOC v2; direct CCv2
and CCv3 import/export is not implemented.
