# Contributing to MOMO Core

[简体中文](#简体中文)

## Issues

Issues are open for reproducible bugs and concrete design proposals. An Issue is a
request for investigation or discussion; opening one does not guarantee that a
change will be accepted or scheduled.

Before opening an Issue:

1. Search existing Issues and documentation.
2. Check the latest release or current `main` when practical, and identify the affected release or commit.
3. Keep one independent problem or proposal per Issue. Use a short title that describes the observable result.
4. Provide the operating system, architecture, Rust version, affected component, and any relevant feature flags or configuration.
5. For bugs, include the smallest reproduction possible, the exact steps, expected and actual behavior, and whether the problem is consistent or intermittent.
6. For proposals, begin with the problem or use case. Describe the intended behavior, boundaries, alternatives, and compatibility or data impact.
7. Paste text and minimal code instead of screenshots when possible. Attach only the evidence needed to understand the Issue.
8. Remove API keys, credentials, private conversations, character data, prompts, logs, private paths, and other identifying information.

Maintainers may ask for more information, edit labels or titles, link related
Issues, or close reports that are duplicates, cannot be reproduced, lack the
requested information, or fall outside the scope of MOMO Core. If new evidence
becomes available, add it to the existing Issue instead of opening a duplicate.

## Pull requests

Pull requests are open. For substantial behavior, API, storage, or specification
changes, opening an Issue first is recommended so the direction can be discussed
before implementation. A Pull Request may still be reviewed, changed, declined,
or closed at the maintainers' discretion.

## 简体中文

### Issues

Issues 用于反馈可复现问题或提出具体设计建议。Issue 代表一次调查或讨论请求，创建后不代表相关改动一定会被接受或排期。

创建 Issue 前请：

1. 搜索现有 Issues 和文档，避免重复。
2. 条件允许时先在最新版本或当前 `main` 上验证，并填写受影响的版本标签或 commit。
3. 每个 Issue 只描述一个独立问题或建议；标题应简短，并直接说明可观察到的结果。
4. 提供操作系统、架构、Rust 版本、受影响组件，以及相关的功能开关或配置。
5. 问题报告需包含尽可能小的复现样例、准确步骤、预期与实际行为，并说明问题是稳定出现还是偶发。
6. 功能建议需先说明问题或使用场景，再说明期望行为、边界、替代方案以及兼容性或数据影响。
7. 能用文字和最小代码表达时，请不要只发截图；只附上理解问题所需的材料。
8. 删除 API Key、凭据、私有对话、角色数据、提示词、日志、私有路径及其他可识别信息。

维护者可能要求补充信息、调整标题或标签、关联相关 Issue，也可能关闭重复、无法复现、缺少必要信息或不属于 MOMO Core 范围的问题。如果之后获得了新证据，请补充到原 Issue，不要重复创建。

### Pull requests

Pull Request 现已开放。涉及较大行为变化、公开 API、存储格式或规范调整时，建议先创建 Issue 讨论方向，但这不是提交 PR 的硬性前提。维护者会根据项目方向决定审阅、要求修改、拒绝或关闭 PR。
