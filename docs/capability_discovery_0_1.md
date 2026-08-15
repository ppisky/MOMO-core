# 供应商能力描述协议 0.1

**定位：** 这是受管部署和供应商集成使用的内部协议，不是普通用户设置项。客户端设置页不要求用户填写协议地址、理解缓存状态或手动刷新。

供应商可以通过受信任的 HTTP(S) 配置提供模型能力描述。客户端不会根据模型名称猜测能力；未配置受管描述时直接使用保守默认值。

```json
{
  "schema_version": 1,
  "ttl_seconds": 3600,
  "models": {
    "example-model": {
      "schema_version": 1,
      "tokenizer": {"kind": "o200k_base"},
      "context_window": 128000,
      "max_output_tokens": 16384,
      "streaming": true,
      "parameters": ["top_p", "max_tokens", "stop"],
      "allow_unknown_parameters": false
    }
  }
}
```

## 安全与回退

- 响应上限为 256 KiB，schema、模型标识、Token 上限和参数白名单必须全部通过校验后才会原子替换缓存。
- TTL 最短 1 秒、最长 24 小时。过期供应商缓存可清理。
- 未发现、未知模型、缓存过期或在线发现失败时使用保守 Token 估算和默认能力，不声称精确兼容。
- 发现 URL 仅接受 HTTP(S)。认证令牌只通过 `Authorization: Bearer` 发送，不进入 capability 缓存或日志。

## 验收

`momo_core::CapabilityRegistry` 覆盖文档注册、在线 HTTP 发现、缓存命中、过期回退、非法 profile 拒绝和未知模型安全回退。
