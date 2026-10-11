# 发布版本信息 / Release version metadata

状态：下一次发布前待实现。本文是发布要求，不表示当前二进制或 HTTP 接口已提供这些信息。

## 当前行为

`runtime_api::core_version()` 使用编译时 `CARGO_PKG_VERSION`，`GET /health` 将其作为 `core_version` 返回。RC6 的 workspace 包版本是 `1.0.0`，只有 Git 发布标签带有 `-rc.6`。仅访问当前接口的客户端无法区分正式版与预发布版本。

## 下一次发布要求

- 编译产物嵌入完整发布版本，保留 SemVer 预发布部分；候选版不能只报告稳定包版本。
- 版本信息与发布标签由同一份发布元数据校验，标签和实际嵌入值不一致时阻止发布。若内部包版本继续独立维护，必须与产品发布版本明确区分。
- `/health` 提供实际编译产物的完整发布版本及可判定的发布渠道；可用时携带源码 revision。具体新增字段在实现时同步 HTTP 文档及客户端校验，保持现有字段兼容。
- 从源码压缩包构建时也能取得发布版本，不依赖运行时读取 `.git` 或访问 GitHub。无法确认的 revision 或发布渠道明确为未知，不能伪装成正式版。
- 普通开发构建应与官方发布产物可区分；不能把附近标签或当前最新 Release 当成该二进制版本。
- 网页从服务响应显示完整版本和发布渠道；旧服务缺少发布信息时保留其报告值，并承认渠道未知，不用网页自身版本代替。
- 发布验收覆盖候选版、正式版、开发构建和无 Git 元数据的源码包；检查真实二进制的 health 响应与发布元数据一致。

现有 RC6 标签和已发布二进制保持不变；新增信息需要构建并运行具备该能力的新版本。

## English

**Planned for the next release; not implemented yet.** The current health endpoint reports the Cargo package version. RC6 packages use `1.0.0`, while the prerelease suffix exists only in the release tag, so a client cannot identify the release channel from this response.

Future builds must embed the full product version, including prerelease identifiers, and expose that information through health responses. Release metadata and tags must agree. Source archives must work without Git metadata or network access; unknown revisions and channels must remain unknown. Development builds must be distinguishable from official releases. Existing clients and health fields remain compatible, and acceptance checks must verify actual binaries rather than infer their versions from repository tags.
