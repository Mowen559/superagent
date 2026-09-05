# Hub 代理智能连通性测试报告 (superagent_smart_test)

- **测试时间**: 2026-08-19 10:56:56
- **模型获取接口**: `GET http://127.0.0.1:3001/v1/models`
- **聊天对话接口**: `POST http://127.0.0.1:3001/v1/chat/completions`
- **测试策略**: 动态获取可用模型列表（非硬编码），逐一校验连通性
- **获取到的模型总数**: 7

## 1. 动态获取的模型列表

| 序号 | 模型标识 (Model ID) | 关联账号 (Account) | 提供商 (Provider) |
|---|---|---|---|
| 1 | `gemini-3.1-flash-lite` | mowenwen440@gmail.com | gemini_native |
| 2 | `claude-sonnet-4-6` | mowenwen440@gmail.com | gemini_native |
| 3 | `claude-opus-4-6-thinking` | mowenwen440@gmail.com | gemini_native |
| 4 | `gemini-pro-agent` | mowenwen440@gmail.com | gemini_native |
| 5 | `gemini-3.1-pro-low` | mowenwen440@gmail.com | gemini_native |
| 6 | `gemini-3.7-flash-tiered` | mowenwen440@gmail.com | gemini_native |
| 7 | `gemini-3.6-flash-high` | mowenwen440@gmail.com | gemini_native |

## 2. 连通性测试结果

| 序号 | 模型标识 (Model ID) | 连通状态 | 响应耗时 (ms) | 响应内容 / 错误说明 |
|---|---|---|---|---|
| 1 | `gemini-3.1-flash-lite` | ✅ 成功 | 1583 ms | OK. |
| 2 | `claude-sonnet-4-6` | ✅ 成功 | 1993 ms | OK. |
| 3 | `claude-opus-4-6-thinking` | ✅ 成功 | 2708 ms | OK |
| 4 | `gemini-pro-agent` | ✅ 成功 | 2796 ms | OK. |
| 5 | `gemini-3.1-pro-low` | ✅ 成功 | 3611 ms | OK |
| 6 | `gemini-3.7-flash-tiered` | ✅ 成功 | 1820 ms | OK |
| 7 | `gemini-3.6-flash-high` | ✅ 成功 | 1624 ms | OK. |

## 3. 测试统计

- **总计模型数**: 7
- **成功连通数**: 7
- **连通失败数**: 0
- **连通成功率**: 100.0%