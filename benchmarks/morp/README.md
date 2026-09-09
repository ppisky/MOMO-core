# MORP-Bench 1.0

[English](README.en.md)

MORP 是 **MOMO Role-Playing Benchmark**。它只回答一个问题：模型或产品能否持续、具体、自然地扮演同一个人物。

MORP 1.0 不再把记忆召回率、事实抽取、证据 ID、向量检索或维护吞吐量混入角色扮演主分。旧的 memory、ACGN、MOMO dependency 和 stress 数据生成器保留为兼容性诊断工具，但不属于 MORP 1.0 主榜。

## 测什么

主集包含 64 个 case、16 个场景族、8 个等权维度。每个维度包含两个独立场景族；每个场景族有中英两种语言以及只改变最后关键情节的两个反事实分支。

| 维度 | 判断重点 |
| --- | --- |
| `character_fidelity` | 独特声音、自我、动机与缺点，而不是口癖或标签 |
| `emotional_continuity` | 情绪具有惯性，会因新事件渐变、混合或转向 |
| `relationship_dynamics` | 信任、冲突、亲近与修复由经历推动 |
| `agency` | 不替用户决定行动、感受、台词或同意 |
| `world_embodiment` | 身体、空间、物件、时间和后果真实进入表演 |
| `initiative` | 角色有欲望和行动，能推进场景但不劫持故事 |
| `narrative_coherence` | 用台词、动作与潜台词承接当前戏剧节点 |
| `epistemic_viewpoint` | 只使用角色可知信息，不把全知上下文当角色知识 |

题目使用四名机制完整、互相可区分的原创成年角色。角色卡不含类型标签；裁判不奖励模板化友善、长度、华丽文字、复述设定或无条件顺从。

## 为什么不再测记忆

记忆可以为角色扮演提供证据，但“记住地点”不等于“演活一个人”。MORP 1.0 将历史视为已经提供给模型的场景条件，并直接测试下一轮表演。记忆系统本身应由独立的 retrieval/state benchmark 评价。

原生 MOMO 协议会把作者写好的 `user` / `assistant` 对话逐条原样写进一个新 conversation，然后只调用一次候选模型生成最后一轮。它不会：

- 用统一占位回复伪造角色历史；
- 在主分中测试 DMW/NSG 开关；
- 要求候选输出 facts 或 evidence IDs；
- 把候选生成次数乘以历史长度。

OpenAI-compatible 基线同样按真实消息角色回放剧本，而不是把整段历史塞进一个 JSON 问题。

## 离线构建与计划

默认构建的就是 MORP 1.0：

```powershell
python -m benchmarks.morp build --out target/morp-roleplay
python -m benchmarks.morp validate target/morp-roleplay
python -m benchmarks.morp plan target/morp-roleplay `
  --config benchmarks/morp/configs/qwen3.8-flash.momo.json `
  --split eval --repeats 3 --out target/morp-roleplay.plan.json
```

构建和计划完全离线。64 个 case、3 次重复对应 192 次候选调用。只有显式传入 `--allow-ai` 才允许访问模型：

```powershell
python -m benchmarks.morp run target/morp-roleplay `
  --plan target/morp-roleplay.plan.json `
  --out target/morp-roleplay-run --allow-ai
```

也可以使用安全包装：

```powershell
./scripts/run-morp-model.ps1 -Config path/to/candidate.json
./scripts/run-morp-model.ps1 -Config path/to/candidate.json -AllowAI
```

不带 `-AllowAI` 时只生成数据和计划，AI 调用数为零。

## 候选输出

评测接受自然角色扮演正文，不要求 JSON：

```text
角色在这一刻的自然台词、动作或叙述
```

为兼容旧适配器，也接受只有 `answer` 一个字段的 JSON。候选不再输出事实表、证据 ID、分数或分析；纯文本与兼容 JSON 的评分完全相同。

## 单一可审计评审与评分

先为成功的候选生成盲审计划，再生成一份绑定身份与哈希的评审模板：


```powershell
python -m benchmarks.morp judge-plan target/morp-roleplay --predictions target/morp-roleplay-run/predictions.jsonl --out target/judge.plan.json
python -m benchmarks.morp review-template --plan target/judge.plan.json --reviewer codex:rc2 --out target/reviews.jsonl
# Codex 按 judge.plan.json 的冻结 rubric 填写 reviews.jsonl 中的 score、quote、reason，并把 status 改为 ok。
python -m benchmarks.morp score target/morp-roleplay --plan target/morp-roleplay.plan.json --predictions target/morp-roleplay-run/predictions.jsonl --votes target/reviews.jsonl --out target/morp-roleplay.report.json
```

评审按 0–4 评分，并必须引用候选回答中的原文：

- 0：破坏核心角色或场景要求；
- 1：基本失败，主要是通用或矛盾回答；
- 2：意图可辨，但存在实质性的角色、行动权、视角或连续性问题；
- 3：可信表演，仅有轻微瑕疵；
- 4：具体、属于当前场景且完整一致的角色呈现。

一份 `source: "reviewer"`、带稳定 `reviewer` 身份、引用和理由的评审票即可形成正式 case 分数。Codex 可以作为该可审计评审者，不应伪装成人类。旧的两个独立模型裁判仍可作为自动化兼容路径；若采用它们且相差超过 1 分，仍等待 reviewer 裁定。缺失、请求失败或格式错误的候选记 0 并保留执行状态；评审错误不会惩罚候选。

聚合顺序为：评审 → case → 场景族 → 维度 → 八维等权总分。中英文、反事实分支和重复运行不会凭数量提高某个场景族的权重。置信区间以场景族为 bootstrap 单位。

`score_summary.roleplay_score` 和兼容字段 `selected_score` 使用 0–100 标尺。只要成功回答仍未完成可审计评审，或计划中的预测尚未执行，总分就是 `null`；已实际发生的请求失败或格式错误按预注册规则记零。不能把 `null` 宣传成零分或满分。

## 旧诊断工具

旧套件只能显式选择：

```powershell
python -m benchmarks.morp build --suite memory --out target/legacy-memory
python -m benchmarks.morp build --suite acgn --out target/legacy-acgn
python -m benchmarks.morp build --suite momo --out target/legacy-momo
python -m benchmarks.morp build --suite stress --out target/legacy-stress
python -m benchmarks.morp build --suite legacy-all --out target/legacy-all
```

这些报告不得命名为 MORP 1.0 角色扮演成绩，也不得与主榜混合比较。

## 限制

当前题目是公开、原创、AI 辅助编写的开发基准，尚未经过大规模人类偏好校准。它能用于回归和产品对比，但不是防污染隐藏榜单，也不能代表所有角色、文体、语言或成人内容场景。正式发布排名前仍需要盲测人工复核和裁判校准。
