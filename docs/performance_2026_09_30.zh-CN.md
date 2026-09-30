# Turso 0.8.1 与 hotpath 本机验证

日期：2026-09-30。基线提交 `0495004` 之上的本轮改动。环境为 Windows 10
19045 / x86_64、Ryzen 7 PRO 6850H、Rust 1.96.1，使用仓库 Release 配置，未设置
`target-cpu=native`。原始报告与源码哈希保存在
[`benchmarks/performance/2026-09-30`](../benchmarks/performance/2026-09-30/)。

## 决定与改动

- Turso 从 `0.8.0-pre.4` 升级到 `0.8.1`，同步更新 lockfile 中的 Turso 组件，继续
  `default-features = false`。现有 Builder、Connection、查询和事务调用无需修改。
  [官方发布记录](https://github.com/tursodatabase/turso/releases/tag/v0.8.1)。
- hotpath `0.27.0` 接入 `momo-memory` / `momo-storage` 的可选 `hotpath` feature，
  测量检索、索引来源检查/重建、生命周期准备、向量读取与排名。示例入口生成报告。
  默认生产依赖树不包含 hotpath；没有改写锁类型、业务流程或增加常驻服务。

本轮依照 [hotpath 性能分析指南](https://hotpath.rs/blog/profiling-rust-guide)
先定位数据访问与文件处理开销，使用函数计时。没有启用 SQL/HTTP 自动追踪、内存分配
追踪、CPU 采样或网络控制界面；脚本关闭了默认 loopback metrics listener。

## hotpath 找到的定位线索

所有正式工作负载均在编译完成后串行执行，无模型请求。记忆检索先预热一次，再测
25 次；生命周期准备和文件应用测 5 次，包含第一次身份计数登记。表中均为中位数：

| 合成记忆数 | 检索 | 生命周期准备与应用 |
| ---: | ---: | ---: |
| 100 | 5.585 ms | 33.565 ms |
| 1,000 | 25.838 ms | 276.703 ms |
| 5,000 | 114.476 ms | 1,394.112 ms |

1,000 条时，`load_index` 平均 20.13 ms，而检索函数平均 26.14 ms；5,000 条时分别
97.64 / 115.67 ms。`index_source_signature` 对应平均 20.09 / 97.40 ms，说明即使
缓存已经预热，来源文件遍历和属性检查仍占主要部分。索引“被缓存”不等于没有扫描成本。

生命周期的 `prepare_lifecycle_activity` 在 1,000 / 5,000 条时平均 271.43 ms / 1.37 s。
报告中的 `build_index_data` 调用包括一次初始建索引与五次生命周期准备；源码中随后
还会读取长期文档。重复扫描与解析是下一轮优化的具体候选，但优化必须保留外部编辑
检测、来源一致性与恢复约束。

5,000 条、384 维、top-64 的内存 Turso 缓存，写入一次为 437.491 ms；25 次精确检索
中位数为 78.874 ms。hotpath 中排名函数平均 79.17 ms，其内部 `list_nsg_vectors`
平均 74.15 ms。当前先读取整个候选向量集合、解析 JSON，再计算余弦排名；应先测量
候选读取/反序列化的改进空间。这里没有旧版 Turso 同机 A/B，不能宣称升级加速。

函数计时包含子调用和 async 等待，不能相加成独占 CPU 时间。此次结果含插桩开销，
不是无插桩产品延迟，也不覆盖真实 provenance 策略、完整 SQL 提交日志、磁盘向量库、
多请求锁争用或端到端模型链路。5 次生命周期样本尤其不足以建立业务尾延迟承诺。

## 复现与验证

Windows：

```powershell
./scripts/profile-core.ps1 -Documents 100,1000,5000
```

脚本仅生成临时合成数据和 `target/core-profile-*` 报告，结束后恢复其修改过的进程
环境变量。本次使用清理后的依赖集，按 100 / 1,000 / 5,000 条规模串行运行。
其他平台命令及诊断功能边界见 [开发指南](development.en.md#offline-performance-investigation)。

已完成严格 workspace Clippy、337 项 Rust 全特性测试、102 项 MORP Python 测试、
格式检查，以及本机 Release 性能示例构建和实际运行。新增 Turso 磁盘回归覆盖关闭后
重新打开、向量替换、排名和删除后的再次重开。Cargo audit 报告零已知漏洞、无警告。
默认 `momo-server` 普通依赖树确认不引入 hotpath。

未执行旧 `0.8.0-pre.4` 数据文件到新引擎的跨版本迁移样本、远端双平台 CI、真实模型
评测或完整发布流程；本报告不代表新的 RC 已发布或稳定版门槛已经关闭。
