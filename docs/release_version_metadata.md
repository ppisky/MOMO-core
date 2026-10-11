# 编译版本信息 / Build identity

Core 在编译时读取当前仓库的完整 Git 提交 SHA、该提交上的精确标签和已跟踪文件的修改状态，并嵌入二进制。请求期间不读取 Git，不查询远端最新版本。

`GET /health` 返回：

- `build_info.git_revision`：实际编译提交的完整 SHA。
- `build_info.git_tag`：当前提交的精确标签；无标签为 null，不使用附近标签冒充版本。
- `build_info.dirty`：构建时已跟踪文件是否有未提交修改。
- `response_schema`、`control_schema`：独立的请求接口标准，与构建提交无关。
- `core_version`：保留的内部 Cargo 包兼容值，不作为实际发布版本展示。

只有源码没有 `.git` 元数据时，三个 build_info 字段全部为 null。存在 Git 元数据但 Git 工具不可用、仓库不可读或没有提交时，构建明确失败，不伪报“未提供”。支持普通仓库及 linked worktree。Cargo 监听 Git 引用和已跟踪文件，提交变化、打标签和源文件变化都会重新写入构建身份；运行已经生成的二进制不会随仓库变化。

网页显示实际提交、精确标签和修改状态，并单独展示请求接口标准。旧服务没有 build_info 时提示升级；明确的无 Git 构建才显示“未提供”。已发布 RC6 不追溯修改，必须重新编译并运行新二进制才能查询这些字段。

验证：`python scripts/test-build-metadata.py` 覆盖提交与标签、未提交修改、仅提交变化的增量构建、无 Git 源码及 linked worktree。服务 HTTP 测试检查实际 health 序列化；发布还需比对实际二进制响应与目标 Git 提交。

## English

Core embeds the full Git commit, an exact tag (if present), and tracked modification status at compile time. Health responses expose these as `build_info.git_revision`, `git_tag`, and `dirty`. Requests never inspect the runtime checkout. Missing Git metadata produces null fields; a broken checkout or unavailable Git executable fails the build instead of pretending metadata is absent.

Request schemas are exposed separately as `response_schema` and `control_schema`. The legacy `core_version` remains an internal package compatibility value, not the product build identity. Cargo tracks refs and source changes, including linked worktrees. Existing RC6 binaries do not acquire these fields without rebuilding and restarting.
