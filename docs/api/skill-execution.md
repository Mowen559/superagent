# 技能执行与配置模型可见性

`skill_tool list/read` 共用 CapabilityScout 发现顺序，path 为所选技能/资源的绝对路径。
`read` 保留 skill_name/content，新增 skill_path、resource_root、resource_guidance。
可选 `resource_path` 仅允许所选技能目录内的 UTF-8 文本（最大 1 MiB），不扩大普通文件工具的工作区边界。

`GET /v1/models` 保留 object/data/sources 结构。可用单模型配置对应的条目新增：

```json
{"id":"user-configured-model","object":"model","sources":[],"configured":true,"configured_default":true}
```

configured 仅指本地配置完整，不代表上游健康/业务测试成功，也不伪造账号来源。禁用或缺少密钥/地址/模型的配置不标记。
SDK listModels 接受真实账户 sources 或 configured 标记，配置默认模型优先；静态候选模型不视为已配置。
现有 SSE 任务 status=completed 才代表正常完成，不能仅凭生成文件或验证文字断言任务成功。
