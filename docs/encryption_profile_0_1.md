# MOMO Encryption Profile 0.1

**状态：** Implementation Profile
**用途：** 私有 `.moc` 本地备份加密
**更新日期：** 2026-08-10

本文只描述当前 Rust Core 主线使用的私有 `.moc` 加密。账号级同步加密、恢复密钥、
服务端 verifier 与同步 payload 信封不属于本加密 profile 的范围。

## 信封与密钥层次

每次导出私有 `.moc` 时生成独立的 256-bit 随机数据加密密钥（DEK）。DEK 使用
AES-256-GCM 加密内层完整 MOC；用户密码经 Argon2id 派生 256-bit 密钥加密密钥
（KEK），KEK 只封装 DEK。

```text
用户密码 + 128-bit 随机 salt
          │ Argon2id
          ▼
         KEK ── AES-256-GCM ──► wrapped_key（随机 DEK）
                                      │
                                      └─ AES-256-GCM ──► ciphertext（内层 .moc）
```

每次加密分别生成 96-bit `wrapped_key_nonce` 与 `payload_nonce`，不得复用。信封中的
二进制字段使用无填充 Base64。实现使用可清零缓冲区保存 KEK 和 DEK，并保证认证失败
时不返回部分明文。

## v1 信封字段

`private/payload.enc` 是 JSON 对象，包含：

- `format = "momo-encrypted-envelope"`；
- `format_version = 1`；
- `cipher = "AES-256-GCM"`；
- `kdf = "Argon2id"`；
- `kdf_parameters`：`memory_kib`、`iterations`、`parallelism`；
- `salt`、`wrapped_key_nonce`、`wrapped_key`、`payload_nonce`、`ciphertext`。

DEK 封装使用固定域分离 AAD；负载使用固定前缀与 `momo-private-moc-v1` 组合 AAD。
修改算法名、版本、参数、密码、密文或 AAD 均必须导致导入失败。

## KDF 参数策略

当前实现使用 Argon2id v1.3，并提供三档建议参数。调用方可以显式选择档位；如果
调用方要求自适应参数时，`KdfParameters::adaptive_default()` 会根据当前机器总内存与
逻辑并发数选择新导出的默认档。

| 档位 | memory_kib | iterations | parallelism | 用途 |
| --- | ---: | ---: | ---: | --- |
| fast | 32768 | 2 | 1 | 低端设备或频繁临时导出，优先速度。 |
| standard | 65536 | 3 | 1 | 默认平衡档。 |
| hard | 262144 | 4 | 1 | 用户明确选择更高离线攻击成本时使用。 |

私有 `.moc` 信封携带自身 KDF 参数，因此 v2 之后提高新默认值不要求旧 v2 包重加密
后才能读取。v1 到 v2 不提供兼容承诺；v2 是后续扩展兼容的基准线。

自适应规则是保守的：

- 总内存低于 4 GiB 或并发不足 2：fast；
- 总内存至少 4 GiB 且并发至少 2：standard；
- 总内存至少 16 GiB 且并发至少 8：hard。

该规则只影响新导出的默认参数；导入解密始终以信封里记录的参数为准。

导入端先验证不可信参数，当前只接受：8 MiB–1 GiB 内存、1–10 次迭代、并行度
1–8。该上限用于阻止恶意信封诱导无界资源消耗，不表示所有上限组合都适合作为导出
默认值。

## 已验证与未完成边界

自动化测试覆盖二进制负载往返、错误密码、错误 AAD、密文篡改、随机 salt/nonce
不复用和恶意 KDF 参数拒绝；私有 `.moc` 集成测试覆盖自动识别、错误密码拒绝与正确
恢复。

三档建议和自适应选择已有单元测试。后续如果需要更细，可以继续补充不同目标运行环境
上的耗时记录，用来调整阈值，而不是改变已导出包的读取方式。
