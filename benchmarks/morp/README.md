# MORP-Bench 0.2

[English](README.en.md)

MORP = **MOMO Role-playing**。本目录提供原创 RP / 记忆评测集、
ACGN 角色标签消融实验、来源适配、执行计划、断点记录、评分、裁判校准和配对比较。
Python 3.13，仅标准库；离线运行不需要 API key，也不访问模型服务。

新增 [MOMO 上下文 / 提取记忆预设](SCENARIOS.md)：同一原生模型、相同事件流和预算，
分别测试当前 conversation 依赖与全新 conversation 的提取记忆依赖，并配对比较质量、
响应费用与延迟。`scenario-plan` 默认离线，
具体候选与裁判模型可稍后配置。后台账单缺失时总成本保持未测量。

默认集合包含 **511 个测试点、53 个场景族**：264 个记忆与互动测试点、
15 个原创成年 ACGN 角色 × 3 种情境 × 3 个提示词实验组的 135 个测试点，
72 个 MOMO 依赖预设、16 个 0.2 客观压力投影，以及 24 个 0.2 角色扮演投影。
扩展点不是 511 个独立样本。场景族、角色及其语言、
长度、实验组、重复运行在统计时聚类。

全部原创内容由本次开发中的 AI 助手编写，尚未经过独立人工标注或用户偏好验证。
`0.2` 是公开开发/回归基准，不是防污染的隐藏榜单；`eval` 划分不等于秘密测试集。
所有原创代码和数据配方随根目录 Apache-2.0 许可发布；外部数据保持自己的许可。

## 一键离线验证

在仓库根目录运行：

```bash
bash scripts/test-morp.sh
```

Windows PowerShell 对应入口为 `./scripts/test-morp.ps1`。

脚本运行 Python 单元测试，编译真实 Rust 探针，生成数据和原生运行计划，
再从全新临时工作区重复执行上下文/DMW/请求契约测试。报告位于输出的
`target/morp-offline-...` 目录。`ai_calls` 必须为 0，`roleplay_quality_score` 必须为 null。
脚本不会把根据答案构造的模拟回复报告为模型成绩。

## 原创测试设计

0.2 的归因协议、MO State 影子模式和拆分发布建议见
[MORP 0.2 设计](../../docs/morp_0_2_design.zh-CN.md)。新增 `roleplay` 套件包含 6 个真实交互机制族的
反事实对；每条既保留客观事实诊断，也要求角色盲审。`stress` 保留为确定性组件回归。
两者不能互相替代。

| 维度 | 代表情境 | 评分对象 |
| --- | --- | --- |
| recall | 礼物位置、个人偏好、跨会话约定 | 对支持事实的精确回答 |
| update | 明确更正、乱序旧日志、未经证实的传闻 | 最新可信状态，而非最后到达的字符串 |
| reasoning | 所有权→存放位置、经过时间、新暗号 | 跨事件推理与局部规则应用 |
| boundary | 未知信息、尚未揭露的剧情、私聊来源、请求遗忘 | 知识边界与行为层面的不披露 |
| world | 物品数量、场景切换、人物离场 | 世界状态与因果一致性 |
| persona | 用户行动权、角色身份、ACGN 机制 | 有经历依据的反应与表达 |
| emotion | 失落、重逢、压力下的表达变化 | 情境和角色一致性，不奖励“永远积极” |
| social | 无证据的从众压力、有证据的观点修正 | 合理坚持和合理改变，不奖励“永不动摇” |

记忆配方支持 50/100/500 条历史事件（不是同数量的双向对话），每 25 条划分一次会话。
位置、礼物、饮料等使用固定哈希产生的虚构值，降低角色名或常识直接猜答案的可能。
这是模板化压力测试：填充语料只有有限变化，不能据此声称覆盖真实 500 轮叙事复杂度。
`forget` 测的是输出不披露；没有把它解释为磁盘擦除或模型权重遗忘。

0.1.1 新增 `selective_update`（update / eval）：跨会话追踪两个所有者的三个文件夹，
只移动其中一个，并在更新后加入另一所有者的同色文件夹干扰。三个位置分别评分，
检查旧值残留、所有者混淆和误覆盖未更新事实。默认生成中英双语、两个变体、三个长度，
共 12 个相关用例，统计上仍只算一个场景族。版本升级会改变生成值和数据指纹，旧计划需重新生成。

0.1.2 新增 `momo` 预设套件。3 个机制化原创成年角色各自包含经历、认知、需求、规则、
关系与行为倾向；6 个场景族分为 `context` 和 `extracted` 两种依赖。预设探针不再重放
完整历史：前者继续当前 conversation，后者打开全新 conversation。这样可以分别检查
当前上下文优先级与 DMW/NSG 的跨会话提取，而不是把历史轮数误当作独立评分样本。
其角色机制沿用仓库的“经历→认知→需求→规则→关系→行为倾向”设计；困难题组合了
局部规则覆盖、选择性更正、相似对象归属、时间激活、隐私和用户行动权。报告必须分别
展示 `context` 与 `extracted`，避免合并均分再次形成虚假的统一满分。

配方中的 `expected`、证据标签、禁止披露值、裁判 rubric 与来源元数据不传给被测模型。
全上下文基线通过显式白名单构造输入，并过滤不可见 Space；原生流程将私有事件写入
不同 Space，最终探针的读取集合不包含该 Space。后者依赖宿主正确选择 Space，
不能用于证明服务器具有它并未提供的远程用户鉴权能力。

## ACGN：去标签后是否仍像同一个人

`acgn.py` 中的每个角色都有六个完整字段：经历、认知、需求、规则、关系、行为倾向。
所用标签是傲娇、三无、天然呆、腹黑、病娇、元气、毒舌、大小姐、妹妹系、姐姐系、
无口、忠犬、恶役、圣母、疯批。它们仅用于组织实验，不是人物真实性的定义，
也不是医学诊断。角色均为原创成年人；关系型标签不自动创建血缘关系。

三个实验组：

1. `label_free`：只有六类机制；它是主评分使用的角色输入。
2. `labeled`：同一份机制增加一个标签。
3. `labels_only`：只提供姓名、成年设定和标签，作为缺少因果信息的对照。

同一角色的三个实验组共享历史、问题、参考机制和裁判 rubric。
裁判始终看同一份无标签机制，不看被测模型名、实验组或标签。
“原来的那个人”在这里是预先定义的原创角色机制；没有人类扮演记录时不能声称
已证明它与某位原作人物等价。不能按口癖、固定句式或标签关键词给分。

信任、压力、亲近、公开程度均为 0..1 的情境旋钮。当前预注册的是三个组合：
公开低信任、私下高信任、危机高压力。它们能检查情境差异，**不能识别单个旋钮的独立因果效应**。
若要估计单旋钮响应曲线，应新增只改变该旋钮的版本，并保留其他条件和问题不变。
三无与无口在本集合分别通过表达幅度、言语量区分，不能都被粗略判作“没有情绪”。

`acgn_ablation` 报告 `label_free - labeled` 与 `label_free - labels_only`，按角色聚类。
任一配对未评分就不发布完整效应；控制组不掺入主 persona 得分。
0.05 仅是预注册的探索性非劣界值，不输出“已经等价”的结论。

## 数据、计划与运行

### 安全的一键模型入口

仓库提供默认 **只生成计划、不访问模型** 的包装脚本。配置可以使用下方的 MOMO 或
OpenAI-compatible 示例；脚本创建全新输出目录、验证数据集并打印协议与
`candidate_calls`。没有显式开关时，执行到计划阶段即停止，`AI calls` 保证为 0：

```bash
bash scripts/run-morp-model.sh --config benchmarks/morp/configs/baseline.example.json
```

在配置中替换所有 `REPLACE_...` 值、将凭据只放入 `api_key_env` 指向的环境变量、检查
计划中的预计调用数后，才可用 `--allow-ai` 执行完全相同的计划。默认参数是 `memory`、
`dev`、50 条事件、1 个变体、1 次重复。可用 `--suite`、`--split`、可重复的 `--horizon`、
`--variants`、`--repeats` 和 `--output-root` 调整。模型执行结束后脚本只生成无需裁判的客观项报告；主观维度
仍须按本节后面的流程使用两个独立裁判，不能把 `null` 总分解释成零分或完整成绩。

### 轻量选择与分数

日常回归不必运行完整集合。下面的客观轻量版在 10 条事件中各取 recall、update、
reasoning、boundary、world 的一个场景族，共 10 个中英文 case。直连模型需要 10 次候选
调用；MOMO 在线协议需要 110 次候选调用，维护调用另计。执行后报告直接给出
`objective_score: 0..100` 和覆盖率，不需要模型裁判：

```bash
bash scripts/run-morp-model.sh \
  --config path/to/candidate.json \
  --suite memory \
  --split eval \
  --horizon 10 \
  --family gift \
  --family correction \
  --family two_hop \
  --family private \
  --family inventory
```

去掉最后的 `--allow-ai` 时仍然只生成计划；确认调用数后再显式加上该开关。
这套 10 条事件配置是 smoke/profile，不等同于标准 50/100/500 压力成绩。

只看一个角色时可按编号或中文名筛选。以下命令只选择赤羽凛在三个状态下的
`label_free` 主实验组：直连模型 3 次调用，MOMO 在线协议 9 次调用。

```bash
bash scripts/run-morp-model.sh \
  --config path/to/candidate.json \
  --suite acgn \
  --split all \
  --character 赤羽凛 \
  --arm label_free
```

去掉 `--arm` 会同时测试 `label_free`、`labeled`、`labels_only`，即 9 个 case；也可重复
`--character` 选择多个角色。`--dimension`、`--family`、`--case-id`、`--dependency` 都可重复使用，多个不同
筛选器之间采用 AND 关系。计划会保存完整 selection，拼错角色或筛选结果为空时直接失败。

MOMO 依赖预设可以单独生成，并按依赖选择：

```bash
python -m benchmarks.morp build --suite momo --horizons 24 --variants 1 --out target/morp-preset-data
python -m benchmarks.morp scenario-plan target/morp-preset-data \
  --config path/to/momo.json --split all --repeats 1 \
  --dependency context --dependency extracted --out target/morp-preset-plans
```

MOMO 配置默认推荐 `"history_mode":"recorded"`：历史转录写入本地 conversation 与维护队列，
候选模型只执行最终探针。它用于低成本组件归因。将其改成 `"live"` 才会让候选逐轮生成，
用于小样本反馈循环测试；两种协议不能合并评分。

先对一个 `--family` 和一种 `--dependency` 使用 `scenario-plan --matrix core`；它只比较
上下文、DMW+NSG、DMW+NSG+MO State 三臂。发现回归后再用 `--matrix causal` 展开七臂，
避免把完整因果矩阵当作每次提交都必须执行的烟雾测试。

评分报告的 `score_summary` 使用 0–100 标尺：

- `objective_score`：所选客观维度的等权宏平均，可用于无需裁判的轻量回归；
- `facts_only_diagnostic`：只核对客观事实字段，即使证据 ID 无法映射仍保留；它用于定位
  提取内容与证据协议的问题，不替代严格主分；
- `selected_score`：所有已选择主维度的宏平均；只要角色、情绪或社交题尚未裁判就保持 null；
- `coverage`：有效返回比例，不代表回答质量；
- `status`：`complete`、`objective_only` 或 `requires_judges`。

因此，单角色测试可以很轻，但角色质量不能靠格式检查生成假分数：需要两个独立模型裁判，
或一份带引用、理由和 reviewer 身份的人工裁定后，`selected_score` 才成为明确的 0–100 分。
真实运行完成后使用[结果表模板](RESULTS_TEMPLATE.md)记录模型、revision、计划哈希、调用量、
分维度得分、裁判校准、泄漏和 provider smoke；模板中的空白不代表零分。

例如，显式授权一次真实候选运行：

```bash
bash scripts/run-morp-model.sh --config path/to/candidate.json --allow-ai
```

Windows PowerShell 对应入口为 `./scripts/run-morp-model.ps1`，参数名采用 PowerShell 风格，
例如 `-Config` 和 `-AllowAI`。

脚本不会启动 MOMO Core 或模型网关。`backend: "momo"` 时，应先启动专用数据目录下的
Core 和提供逻辑模型路由的 adapter gateway；`base_url` 指向 Core 的 `/v1`。直接测
OpenAI-compatible 模型时使用 `backend: "openai"`，`base_url` 指向服务商的 `/v1`。
这两种协议分别出报告，不能直接混榜。

```bash
python3 -m benchmarks.morp build --out target/morp-dataset
python3 -m benchmarks.morp validate target/morp-dataset
python3 -m benchmarks.morp plan target/morp-dataset --config benchmarks/morp/configs/momo.example.json --out target/morp-plan.json
python3 -m benchmarks.morp calibration-plan --out target/morp-calibration-plan.json
```

`build --suite acgn` 只生成角色实验，`--suite memory` 只生成原记忆/互动测试，
`--suite momo` 只生成上下文/提取记忆预设，`--suite stress` 生成确定性反事实压力集，
`--suite roleplay` 生成 0.2 角色扮演集；
`--horizons 50 100 500 --variants 2` 控制模板扩展。
计划默认选择 `eval`、重复 3 次；`--split dev` 用于调试。
计划记录输入哈希、数据哈希、脚本实现哈希、协议、模型部署版本和预计候选调用数。
MOMO 的维护/嵌入调用另计，计划不能将候选调用数当作完整价格估算。

两种协议分别报告，不能直接混榜：

- `full-context-replay/1`：一次性提供允许读取的完整历史，测上下文利用。没有后台记忆写入。
- `momo-online-sessions/1`：历史逐条经原生 `/v1/momo/responses` 产生回复和维护，
  会话间重新建 conversation，记忆测试最终在新 conversation 探测。ACGN 最后在当前
  会话探测，以免把短时情境表达能力混成记忆蒸馏是否及时完成的问题。

原生脚本使用独立随机 Space，不清理用户空间。仍应连接专用于评测的服务器数据目录；
中间响应和检查点可能很多，留存以便审计。模型路由、维护参数和精确版本需由操作者
配置在专用服务器中，并记录在 `revision`；脚本无法替宿主验证任意远端模型版本。
`base_url` 填到 `/v1`，实际端口以服务器设置为准。API key 只通过 `api_key_env` 指定
环境变量名，不写入计划。示例中的 `REPLACE_...` 值不能执行。

以下为后续人工启用 AI 时使用的命令，**本次开发不执行**：

```bash
python3 -m benchmarks.morp run target/morp-dataset --plan target/morp-plan.json --out benchmarks/results/run-a --allow-ai
python3 -m benchmarks.morp judge-plan target/morp-dataset --predictions benchmarks/results/run-a/predictions.jsonl --out target/judge-plan.json
python3 -m benchmarks.morp judge --plan target/judge-plan.json --config path/to/judge-a.json --out benchmarks/results/judge-a.jsonl --allow-ai
python3 -m benchmarks.morp judge --plan target/judge-plan.json --config path/to/judge-b.json --out benchmarks/results/judge-b.jsonl --allow-ai
python3 -m benchmarks.morp score target/morp-dataset --plan target/morp-plan.json --predictions benchmarks/results/run-a/predictions.jsonl --votes benchmarks/results/judge-a.jsonl benchmarks/results/judge-b.jsonl --out benchmarks/results/report-a.json
python3 -m benchmarks.morp compare benchmarks/results/report-a.json benchmarks/results/report-b.json --out benchmarks/results/comparison.json
```

裁判需使用两个独立模型/部署配置。相同 endpoint、model、revision 的不同温度不会
被当作两个裁判。推荐不同模型家族并避免与候选模型相同，但模型家族不能只凭 ID 可靠推断。
温度 0 不保证供应商输出逐字稳定，重复运行用于报告波动，不能挑最好的重试。
候选与裁判都逐条记录成功/失败并刷新到磁盘；相同计划可继续未完成项，
已记录的失败不会自动重试。换模型、参数、提示词或数据要新建计划和结果目录。
进程被硬中断造成最后一行 JSONL 不完整时读取会失败；应保留原文件并另存完整记录，
不静默丢弃损坏的行。

## 评分口径

客观项目使用 `facts` 的逐字段精确匹配，字符串做 NFKC、大小写和首尾空白归一化。
不使用“回答包含正确词就算对”，不把 `false` 当作 `0`，不把字符串数字当作数值。
多字段项目允许按字段比例给分；`answer` 必须是非空的角色回复。
客观分只证明结构化事实回答正确，不能证明自然语言 `answer` 的文风或语义完全一致。
`evidence_ids` 的 precision/recall/F1/nDCG 仅为自报证据诊断，不是实际检索日志指标，
不混入回答主分。空证据集的 recall 和 F1 为 null。

主观项目采用有描述锚点的 0..4 分制，要求实际回答中的精确引用和理由。
两名裁判相差超过 1 分转人工复核，裁判格式错或引用不存在保持未评分，不能惩罚候选。
人工复核可添加绑定同一 `prediction_sha256`、`rubric_sha256` 的 vote，携带
`source: "human"`、`reviewer`、唯一 `judge`、`score`、`quote`、`reason`、`repeat`、`case_id`。
同一测试点只能有一份最终人工裁定，它覆盖模型裁判分歧；未经人工核验不升级为“人类偏好分”。

聚合顺序为字段→测试点→场景族→维度→八维等权宏平均。长度、语言、重复次数不改变
同一场景族的权重。维度内按场景族做固定种子的 2,000 次 bootstrap，少于两个场景族
不给置信区间。该区间描述本集合的场景抽样不确定性，不代表所有 ACGN 角色总体。
缺失/报错/无效候选记 0 并进入分母；等待裁判/人工复核为 null；缺失维度或主观分
时总分为 null。泄漏禁止披露的精确标识符会使该项目归零，并单独报告泄漏计数。
这种检查不能发现语义改写后的所有泄漏，缺失响应也不是隐私保护证据。

`compare` 要求相同数据、协议、题目集合和评分政策，按配对场景族报告差值和区间。
报告保留所有重复轮次、语言/长度切片、覆盖率、缺失原因和模型配置。
不能从覆盖率不同的局部成功结果直接宣称整体优胜。

## 裁判稳定性

`calibration-plan` 提供 8 类、16 个原创正反锚点，覆盖行动权、情绪、状态更新、知识边界、
关系边界、从众压力、证据修正和低表达角色。好坏标签不进入裁判提示词。
后续用同一个 `judge` 命令执行此计划，再运行：

```bash
python3 -m benchmarks.morp calibration-score --votes benchmarks/results/calibration-a.jsonl --out benchmarks/results/calibration-report.json
```

报告包含完整覆盖、落在预期分数带之外的距离、错误高分数量。当前锚点是助手编写的
流程校验材料，即使全部通过，也不足以证明与人类偏好一致。正式发布模型排名前，
需要独立人工标注、更多歧义反例、跨家族裁判一致性和盲测复核；这些不由离线单元测试替代。

情绪/立场标签可离线计算本地诊断：

```bash
python3 -m benchmarks.morp label-metrics emotion path/to/emotion-pairs.json --out target/emotion-report.json
python3 -m benchmarks.morp label-metrics stance path/to/stance-pairs.json --out target/stance-report.json
```

情绪输入为 `{"labels":["sad","happy"],"pairs":[["sad","sad"]]}`；立场输入为
`{"low":1,"high":5,"pairs":[[1,3],[5,3]]}`，每对依次为参考、候选。
立场同时报告误差、偏差和方差，避免群体平均值相同掩盖过早趋同。
它们的 profile 明确标为 local diagnostic，不能冒充 EmoCharacter 或 DEBATE 官方指标。

## 外部基准与许可

| 来源 | 本项目处理 | 采用的设计 |
| --- | --- | --- |
| [RPGBench](https://github.com/boson-ai/rpgbench-public) | Apache-2.0 仓库；本地 games.jsonl 导入和初始化代理任务 | 游戏状态、叙事约束、用户行动权 |
| [EmoCharacter](https://aclanthology.org/2025.naacl-long.316/) | 未确认可随项目使用的独立数据授权，采用原创情绪情境 | 情绪表达与跨情境变化；不复刻剧本 |
| [DEBATE](https://huggingface.co/datasets/seantw/DEBATE_LLM) | Research-Only / Non-Commercial，不导入 | 原创意见坚持、证据修正与群体趋同诊断 |
| [LongMemEval cleaned](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned) | MIT 数据卡；本地 JSON 导入 | 跨会话、时间、知识更新、未知时弃答 |
| [LoCoMo](https://github.com/snap-research/locomo/blob/main/LICENSE.txt) | CC-BY-NC-4.0，不导入 | 长对话、多跳关联、人物关系连续性 |
| [MemoryAgentBench](https://huggingface.co/datasets/ai-hyz/MemoryAgentBench) | MIT 数据卡；本地 JSONL 导出适配，保留来源说明 | 增量学习、检索与冲突解决；不将冲突解决当物理遗忘 |
| [PersonaMem v1](https://huggingface.co/datasets/bowen-upenn/PersonaMem-v1) | MIT 数据卡；双文件锁定和选择题适配 | 隐含偏好、偏好变化、截止位置后的信息不能泄漏 |
| [BEAM](https://huggingface.co/datasets/Mohammadta/BEAM) | 数据 CC-BY-SA-4.0、代码 MIT；当前仅参考 | 长度扩展、事件次序、偏好遵循、摘要；后续派生数据需独立保留署名与相同方式共享 |

BEAM 的公开首条会话样例中，一个时间题的 `answer` 写 4 weeks，`rubric` 写 8 weeks。
本轮只借鉴设计，没有将其问答自动充作标准答案。适配前应建立参考答案审核流程，
不能因为数据已发布就假设标签绝对正确。PersonaMem v1 的 MIT 不代表其后续版本或
所引用第三方素材自动具有相同许可。来源核验日期为 2026-09-05；机器注册表为 `sources` 命令。

外部数据不自动下载、不进入 git，也不执行远端仓库的安装或模型脚本。
允许的数据可放到被忽略的 `benchmarks/data/`，锁定字节和确切上游 revision：

```bash
python3 -m benchmarks.morp pin longmemeval benchmarks/data/longmemeval_s_cleaned.json --revision UPSTREAM_COMMIT_SHA --license-note "MIT; keep upstream notice" --out benchmarks/data/lme.lock.json
python3 -m benchmarks.morp import benchmarks/data/lme.lock.json --out benchmarks/data/lme-normalized
python3 -m benchmarks.morp pin personamem benchmarks/data/questions_32k.csv --context benchmarks/data/shared_contexts_32k.jsonl --revision UPSTREAM_COMMIT_SHA --license-note "PersonaMem-v1 MIT; keep notice" --out benchmarks/data/personamem.lock.json
```

导入保持来源和许可，不把第三方数据重新许可为 Apache-2.0。数据锁定用于检测变动，
不是对任意本地文件权利归属的证明。相同协议下的本地代理任务结果须使用各自 profile，
**不能与原论文官方成绩直接比较**。RPGBench 代理当前只测初始化，不宣称复现完整游戏轨迹；
EmoCharacter 和 DEBATE 已用原创任务替代，没有“已经跑通其官方全量评测”的声明。
上游回放只支持 full-context 协议，防止把历史 assistant 发言改写成新 user 发言后
误报为原始基准的在线记忆实验。
# 跨维护周期回归套件（0.2.0）

`python -m benchmarks.morp build --suite stress --out <新目录>` 生成固定 36 轮的 16 条成对投影，覆盖执行与未执行、部分更新与撤销、所有权与保管权、限定范围的同意。它们代表 4 个语义机制，不是 16 个独立机制。全开原生运行在第 12/24/36 轮排空维护，并单列等待时间。上下文控制组答对只说明题目可解，不能代替原生跨会话验证。缺失或 HTTP 错误保留零分惩罚，但报告状态为 `incomplete_execution`，不应解读成“已经完成且事实答错”。
