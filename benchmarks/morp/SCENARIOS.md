# MORP 0.2：MOMO 组件归因场景

状态：执行与报告代码已提供，真实模型成绩尚未测量。模型、固定部署 revision 和价格由运行者提供。

## 实验定义

各实验臂复用同一 case/repeat、模型、温度、context_window 和输出预算，并通过 MOMO 原生接口
录入完全相同的历史。每个 case/repeat/臂使用独立的新 Space 和角色；私有 Space、
expected 和 rubric 不进入候选输入。

0.1.2 增加两种明确的预设依赖，二者都不在最终探针中重新粘贴完整历史：

- `context`：探针继续最近的当前 conversation，测试当前场景、局部规则、用户刚表达的
  边界，以及结构化记忆是否错误覆盖更近的上下文；
- `extracted`：探针打开全新 conversation，只提供问题和事实输出 schema。基础组没有
  跨会话来源，全部开启组必须依靠 DMW/NSG/MO State 提取结果。

旧 MORP case 继续重放相同可见历史，作为长上下文兼容回归；它们不能用来证明跨会话提取。

| 场景 | DMW 读写 | NSG 读写 | MO State |
| --- | --- | --- | --- |
| `basic_context` | 关闭 | 关闭 | 关闭 |
| `dmw_only` | 开启 | 关闭 | 关闭 |
| `nsg_only` | 关闭 | 开启 | 关闭 |
| `dmw_state` | 开启 | 关闭 | 开启 |
| `nsg_state` | 关闭 | 开启 | 开启 |
| `dmw_nsg` | 开启 | 开启 | 关闭 |
| `all_enabled` | 开启 | 开启 | 开启 |

`history_mode: "recorded"` 使用 `momo-recorded-sessions/1`；live 预设使用
`momo-dependency-scenarios/1`；旧的历史重放对照仍使用
`momo-context-scenarios/1`。不要跨协议混榜。`extracted` 的预期不是两组绝对同分，
而是量化全部开启组在同一原始事件流上的跨会话增益。错误计入覆盖率和质量，不能为某组
悄悄增大预算。

`momo` 预设包含 3 个原创成年角色。每个角色按经历、认知、需求、规则、关系和行为倾向
描述，不用标签或口癖代替角色机制。6 个场景族覆盖场景新旧值、局部规则、行动权与隐私、
选择性更正承诺、跨会话两跳关系推理、关系边界；默认中英双语、两个变体和三个长度。

“全部”指上述三个开关。视觉、工具、LSB、格式导入等不属于本实验的干预变量。
宿主仍须配置有效的 conversation、memory_distillation、semantic_graph_governance 路由、
治理提示词及所需 embedding profile。开启请求标志不意味着宿主已经配置了这些能力。
revision 应固定 Core、网关、模型和配置版本；运行期间不应更换路由映射。

任何启用 DMW 或 NSG 的实验臂都在探针前调用本地管理端点 `POST /v1/momo/maintenance/drain`，
将历史的 DMW/NSG 部分批次也处理完；失败或超时则该次预测为错误，不继续探针。
live 模式在探针后也排空新产生的维护工作；recorded 模式不等待与当前得分无关的探针后维护。
此管理端点会调用模型，
不是只读状态查询；受服务响应超时及最多每类 64 批的限制，不属于稳定 1.0 wire。
每批最多 32 条。使用专用测试实例，并停止其他写入。
它保证待维护队列处理完成，不保证模型生成了高质量记忆，亦不会自动批准待审核知识。

## 现在即可离线生成计划

在仓库根目录运行以下命令（每次使用新的输出目录）：

```powershell
python -m benchmarks.morp build --suite momo --horizons 24 --variants 1 --out target/morp-preset-data
python -m benchmarks.morp scenario-plan target/morp-preset-data --config benchmarks/morp/configs/momo.example.json --split all --repeats 1 --matrix core --out target/morp-preset-plans
```

支持原 `plan` 的过滤器，并新增可重复的 `--dependency context` / `--dependency extracted`。
`--arm` 仍指 ACGN 的标签实验组，不指基础/全开场景。计划文件分别是
计划文件按实验臂命名，`experiment.json` 汇总调用计划。`paired` 生成基础/全开两臂；
`core` 生成基础、DMW+NSG、全开三臂；`causal` 生成全部七臂。
recorded 模式的候选调用数是 `case 数 × repeats × 臂数`；live 模式才是
`(历史条数 + 1) × case 数 × repeats × 臂数`。后台维护、embedding、裁判调用另计。
离线生成不访问模型。

## 提供模型后执行

复制 `configs/momo.example.json`，填写固定 revision，并按需加入真实单价：

```json
"pricing": {"currency": "USD", "input_per_million": 2.0, "output_per_million": 8.0}
```

这里的单价只是字段示例，不代表任何模型的实际价格。未填写单价时费用为 null。
凭据仅通过 `api_key_env` 指向环境变量；不要将密钥写入配置。
使用最终配置重新生成计划，然后显式执行：

```powershell
python -m benchmarks.morp scenario-run target/morp-pair-data --plans target/morp-pair-plans --out target/morp-pair-run --allow-ai
```

按 case/repeat 交替哪个场景先运行。两个场景顺序执行，不并行竞争资源。
重复此命令会继续原计划；已记录成功或错误的预测不会重新计费或挑选重试结果。
崩溃期间的原生成功响应可通过 request ID 重放恢复，已保存响应计时随 checkpoint 保留。
中断期间尚未保存的网络耗时和供应商计费可能不完整。

## 质量与模型协助审查

分别对两组生成 `judge-plan`，再用两个不同、固定版本的裁判配置运行 `judge`。
候选模型和两个裁判的具体模型可以稍后确定。裁判输入隐藏候选模型、场景名和运行指标，
两组用同一套 rubric；相同裁判配置重复执行不算两个独立裁判。

以下以基础组为例；全部开启组替换路径中的 `basic_context`：

```powershell
python -m benchmarks.morp judge-plan target/morp-pair-data --predictions target/morp-pair-run/basic_context/predictions.jsonl --out target/basic-judge-plan.json
python -m benchmarks.morp judge --plan target/basic-judge-plan.json --config judge-a.json --out target/basic-votes-a.jsonl --allow-ai
python -m benchmarks.morp judge --plan target/basic-judge-plan.json --config judge-b.json --out target/basic-votes-b.jsonl --allow-ai
python -m benchmarks.morp score target/morp-pair-data --plan target/morp-pair-plans/basic_context.plan.json --predictions target/morp-pair-run/basic_context/predictions.jsonl --votes target/basic-votes-a.jsonl target/basic-votes-b.jsonl --out target/basic-report.json
```

无需裁判即可给客观题打分；主观题需要两名裁判，严重分歧保留为待人工审查。
裁判引用必须来自候选回答。可以先省略 `--votes` 得到客观分、覆盖率和性能报告，
但未评主观题不能生成完整质量对照。裁判可各自配置 pricing，用量和费用与候选分开。
对于默认开启长思考的 OpenAI Chat Completions 服务，可以在裁判配置中填写
`"thinking":"disabled"`；运行器会发送 `{"thinking":{"type":"disabled"}}`，
避免短格式裁判的思考 token 挤占最终 JSON。其他值会被拒绝。
沿用 `calibration-plan` / `calibration-score` 做裁判校准；未校准结果仍标记 provisional。

两组评分完成后：

```powershell
python -m benchmarks.morp scenario-compare target/basic-report.json target/all-report.json --out target/paired-report.json
```

质量差值、探针延迟差值、已观测响应费用差值均为 `all_enabled - basic_context`。
质量正值较好；延迟/费用负值较低。按场景族聚类，重复和语言变体不当作独立样本。
只有配对齐全且数据完整时才发布相应性能差值；置信区间不是显著性或产品优势保证。
两个单组报告在 `slices.dependency` 中分别列出 `context` / `extracted` 的样本数、未评分数
和诊断均分；配对报告在 `quality_by_dependency` 中分别给出两类的场景族平均差值。
预期上，`context` 检查基础会话能力以及全开后是否被陈旧提取结果干扰，差值不要求为正；
`extracted` 才是跨会话提取的主要增益指标。不得只报告合并总分。

## 计量边界

- `probe_seconds`：客户端非流式完整响应耗时，报告均值、P50、P95、P99，不是首 token 延迟。
- `ingestion_response_seconds`：历史响应耗时之和。
- `maintenance_barrier_seconds` / `post_probe_barrier_seconds`：探针前后等待/排空维护的耗时。
- `attempt_seconds`：本次 case 执行耗时；恢复执行不等于原始完整运行时长。
- `tokens`：响应中实际返回的 input/output tokens，兼容 OpenAI 与原生字段名。
- `observed_response_estimate`：按配置单价计算的已观测响应费用；不推算缺失 usage。
- 原生响应不包含完整的后台维护、embedding 或网关内部账单，**原生 total_estimate 始终为 null**。
  实际总成本需要补充供应商/网关账单；不能据此宣称全部开启更便宜。平价公式也不计算缓存折扣。
- 无效 JSON 回答仍保留已返回用量；失败请求可能已计费但未返回 usage，明确记为未测量。
- 裁判费用单独按币种汇总，不与候选费用混算，不跨币种相加。

当前仓库没有真实场景成绩；离线 mock 和 HTTP 测试只验证执行与计量契约。
