# MOMO Core 1.0.0-rc.6

RC6 integrates the Core capabilities required by the graphical host while preserving independent repositories and the frozen `momo.responses/1.0` and `momo.control/1.0` product wire. This remains a prerelease. As with previous RC builds, internal workspace package versions remain `1.0.0`; the release tag is `v1.0.0-rc.6`.

## Changes / 变更

- Persist runtime policy and sparse per-Space overrides with revision checks; complete record provenance and activity-driven memory lifecycle controls.
- Add complete character configuration and atomic opening-message creation, scoped memory/NSG editing, default-assistant identity binding, and durable Space inventory/migration/removal.
- Persist conversation archive/restore independently of deletion, preserving conversation identity, history and assets.
- Configure outbound OpenAI Chat Completions and Responses directly, including Chat-to-Responses fallback for compatible endpoints; remove the unused Gemini protocol option. Keep the native MOMO response API distinct from these outbound provider protocols.
- Resolve Core-owned recommended request parameters and independent purpose overrides for visual fallback, memory extraction and NSG governance. Connect configured embedding models to NSG indexing and query retrieval; omit dimensions to follow the model default or send an explicit positive dimension.
- Add bounded read-only MO State management: current snapshot, the latest 20 operations, captured evidence IDs and actual completed-response injection diagnostics. Link state operations to their replies in the original completion transaction. Derived DDM caches can be queried/reset through the existing scoped control API.
- Pin the development and CI toolchain to Rust `1.99.0`. Separately license the overlapping-circle logo and simplified version under CC BY-SA 4.0; Core source remains Apache-2.0.

本次汇总 RC5 后的空间、资产、角色配置、模型连接、用途参数、向量检索、对话归档与 MO State 管理能力。图形界面不进入 Core；宿主通过 Core 原生 HTTP/SSE 和本机管理白名单接入。

## Upgrade / 升级

Back up the complete data directory with its active owner stopped before upgrading. SQLite migrations `0025` through `0031` add lifecycle/provenance, character profiles, structured message context, Space removals, conversation archives and state-response links. Existing conversations start unarchived; older MO State operations retain their snapshots with unknown actual injection because no response link was recorded. Follow the documented recovery process rather than discarding journals or forcing conflicts through an older binary.

Runtime policy and Prompt Space overrides persist locally. Model-account storage belongs to the HTTP host, configured through `MOMO_MODEL_CONNECTIONS_PATH`; embedded Core does not depend on the graphical host's configuration database. Custom purpose request parameters remain independent of foreground dialogue parameters. Built-in prompts and recommended request defaults need not be duplicated by host exports.

升级前停止数据目录的使用进程并完整备份。旧状态记录缺少回复关联时，注入审计保持未知；不会根据当前开关倒推。归档不是删除，不清除记忆或语义网资产。

## Boundaries / 边界

The server binds to loopback by default and has no end-user authentication layer. Hosts must authorize Spaces and whitelist local management APIs. This release does not claim complete multi-module MOC import crash atomicity, multi-process writes, provider quality improvements, or desktop-runtime/browser compatibility. DDM remains experimental; optional profiling is a development feature, not an application dependency.

See [HTTP boundaries](http_api_1_0.md), [model connections](model_connections.md), [purpose requests](model_purpose_request_configuration.md), [conversation archives](conversation_archive.zh-CN.md), [MO State management](mo_state_management.zh-CN.md), and [logo licensing](../assets/brand/README.md).

## Verification / 验证

Publication requires the complete local release gate and Linux, Windows and security CI on the exact release commit. The GitHub Release records the final checks and immutable tag. Reproduce with `./scripts/test-release.ps1` or `bash scripts/test-release.sh`; skipped checks do not count as passes.
