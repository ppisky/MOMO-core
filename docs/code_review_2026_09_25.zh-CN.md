# 源码迁入与首轮代码审查（2026-09-25）

## 迁入结果

- 来源：`/media/a850xb/新加卷/MOMO-core/`。
- 工作区：`/home/a850xb/文档/MOMO_core/`。
- 按源目录当前工作树复制，包含尚未提交的修改及未被忽略的新增文件；已删除的文件不恢复。
- 复制 277 个文件，共 2,520,879 字节，逐文件 SHA-256 校验一致。所有复制文件均通过 NUL 字节筛查，文件类型为源码、测试、脚本、配置、文档和文本协议夹具。
- 未复制 `.git` 历史、`target` 编译产物、运行数据和被 Git 忽略的缓存；原目录未作修改。

## 审查范围与验证限制

本轮重点阅读运行时资源管理、Space 锁、记忆提交恢复、维护队列、存储与相关 HTTP 入口，不是全仓库审计。以下问题最初来自静态调用链分析；按用户后续要求完成修复与回归，结果见文末。

- `python3 -m unittest discover -s benchmarks/morp/tests -q`：102 项通过。
- `contracts/1.0` 下 `sha256sum --check SHA256SUMS`：8 项通过。
- 首轮环境未找到 `cargo` / `rustc`；随后按用户要求安装 Rust 1.96.1、Cargo、Clippy 和 rustfmt。
- `cargo fmt --all -- --check`：通过。
- `CARGO_BUILD_JOBS=2 cargo test --workspace --all-features --locked`：编译成功，292 个 Rust 测试通过，0 失败；文档测试通过（无测试用例）。日志保存在本机 `/tmp/momo-cargo-test.log`，该临时路径不保证长期保留。
- 上述为修复前验证记录；未执行完整 MORP probe、release 构建或真实模型调用。
- 测试在当前工作区新生成 `target`，不是从原项目复制的二进制。
- 首轮审查保持业务源码原样，后续已在本工作区修复下列问题；来源目录仍未改动。

## 发现

### 1. [P1] 普通记忆写入的锁不能覆盖调用取消后的后台写入

位置：`crates/momo-core/src/api/runtime_api/memory.rs:544–552`；共用执行器位于 `crates/momo-core/src/api/runtime_api.rs:97–104`。

`apply_memory_patch` 把 Space 锁留在外层 async future，再通过 `spawn_blocking` 执行磁盘修改。嵌入方取消或丢弃该调用时，外层锁随 future 释放，但已开始的阻塞任务仍会继续。第二个同 Space 写入随后可以获得锁，与第一个任务同时执行文件快照、写入或回滚，可能覆盖更新或破坏文件与索引的一致性。`update_memory_document` 等普通写入及 `apply_nsg_patch` 采用相同模式。新增的 `finish_commit` 已保护维护/审批提交，但没有覆盖这些入口。

建议：将 owned guard 移入实际写入闭包，确保写入完成才释放；需要参与优雅停机的写入也纳入任务跟踪。回归测试应使用屏障暂停实际文件任务，取消外层调用，再断言第二个同 Space 写入仍无法进入，放行第一个任务后才能继续。

### 2. [P2] 检索以未经规范化的 UUID 字符串取锁，绕过同 Space 互斥

位置：`crates/momo-core/src/api/runtime_api/memory.rs:79–87,129–131`。

检索仅验证字符串可解析为 UUID，却仍以原字符串建立锁集合。写入入口则先解析 UUID，再用 `scope_id.to_string()` 获取锁。相同 UUID 的大写、小写或不带连字符形式会对应同一个记忆工作区，却命中不同的锁。因此，传入大写 UUID 的检索可以与小写规范形式的写入并行，违反检索正文与来源指纹来自同一版本的约束。HTTP 的 `validate_space_id` 同样只验证并返回原字符串，并未统一该表示。

建议：在建立锁集合前将 Space ID 解析为 `Uuid` 并去重、排序，锁键统一使用规范形式；维护、协调及持久化身份也应检查同类表示问题。回归测试应持有规范形式的写锁，启动同 UUID 大写形式的检索，确认检索会等待。

### 3. [P2] 加权预算溢出后，余数分配循环可长期占用执行线程

位置：`crates/momo-core/src/api/runtime_api/memory.rs:251–259`。

`weighted_space_budgets` 先使用饱和乘法，再对每一个剩余 token 循环。`max_tokens` 来自 HTTP 请求的 `usize` 字段，此路径未限制上界。64 位平台上，单个 Space、合法权重 100、`max_tokens=18446744073709551615` 会得到初始预算 184467440737095516，余数为 18262276632972456099，继而执行同等次数的同步循环。这不是正常舍入产生的少量余数；循环发生在拿到 Space 锁之后，阻塞执行线程并长期占有该锁。

已用 Python 复算相同的 64 位饱和乘法及除法，确认上述迭代次数；未运行巨量循环或发送挂起请求。

建议：限制外部 token 预算，采用加宽整数或商余分解避免中间乘法溢出，将剩余预算用商和余数在 O(Space 数量) 内分配。回归覆盖最大整数、允许上限和常规舍入，验证总预算守恒及及时返回。

## 修复结果

1. `runtime_api.rs` 新增统一的 `run_space_write`。DMW / NSG 的 12 个普通写入口使用该方法，Space 锁移入阻塞文件任务，运行时跟踪任务并在停机时等待。清空记忆操作也使用运行时持有的任务，让 Space 锁覆盖文件、SQLite 与向量清理。
2. `MomoRuntime::lock_mo_state_space` 将合法 UUID 统一成规范锁键。多 Space 检索先按解析后的 UUID 去重、排序，防止同一锁重复获取和不同拼写造成的锁顺序差异；对外观察记录仍保留调用方传入的名称。
3. 加权预算改为先计算商和余数再分配，避免中间乘法溢出；最后的舍入补偿只遍历 Space 数量，不再按 token 数量循环。保留现有合法预算范围，不增加任意的产品上限。

新增 7 个回归测试，覆盖写入取消后的锁与停机等待、失败后锁释放、UUID 多种写法、检索等待写入、重复观察锁去重、最大预算和清空记忆取消后的数据库清理。测试主体位于 `crates/momo-core/tests/unit/`。

- `cargo test --workspace --all-features --locked`：299 个测试通过，0 失败。
- 另将修复前后实际预算函数分别抽取为独立 Rust 程序，以最大预算运行：旧函数超过 2 秒被测试进程终止；新函数正常完成且总预算守恒。没有修改来源目录或对运行服务发送挂起请求。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`：通过。

本次未整理已有的 `runtime.rs`、`commit.rs` 内嵌测试，也未宣称完成其他模块审计或真实模型效果评测。
