# AI Partner

一个有独立人格的 AI 桌面伴侣，基于 Rust + [Iced](https://github.com/iced-rs/iced) 构建。

## 特性

- 多 LLM 提供商支持，带负载均衡和速率限制
- 工作空间系统：SOUL.md 定义人格，AGENTS.md / CONVENTIONS.md 定义项目规范
- 记忆系统：数据库记忆（带遗忘曲线衰减）+ 文件笔记（memory/ 目录）
- RAG 文档检索：对话摘要自动向量化存储
- MCP 工具服务器集成
- 技能系统：按项目目录组织的可复用指令集
- 子代理：内置 compactor（对话压缩）和 skill_selector
- 跨平台：Windows / Linux，系统托盘常驻

## 项目结构

```
crates/
  shared/    # 共享类型：消息、配置、存储、RAG、技能
  core/      # 核心引擎：Runtime、Agent Loop、Provider、工具、子代理
  ui/        # Iced 桌面界面：窗口管理、系统托盘、组件
```

## 构建

```bash
cargo build --release
```

需要 Rust 1.85+。Windows 需要 MSVC 工具链，Linux 需要 `pkg-config`、`libgtk-3-dev`。

## 配置

在项目根目录创建 `config.json`：

```json
{
  "chat": {
    "active": null,
    "providers": [
      {
        "id": "my-provider",
        "kind": "chat",
        "name": "openai",
        "base_url": "https://api.openai.com/v1",
        "api_key": "sk-...",
        "model": "gpt-4o",
        "max_output": 4096,
        "weight": 1,
        "requests_per_minute": 60,
        "enabled": true
      }
    ]
  },
  "embedding": {
    "active": null,
    "providers": []
  },
  "mcp": [],
  "workspace": null
}
```

- `chat` / `embedding`：按模型类型分组的 provider 列表，`active` 为 null 时自动负载均衡
- `mcp`：MCP 工具服务器列表
- `workspace`：工作空间路径，null 时默认为 `{CWD}/.ai-partner/`

## 工作空间

首次运行会自动创建工作空间目录（默认 `.ai-partner/`），包含：

- `SOUL.md` — AI 人格定义
- `AGENTS.md` — 项目专属指令
- `CONVENTIONS.md` — 编码规范
- `memory/` — 文件笔记和日记
- `skills/` — 技能目录

## 运行

```bash
cargo run --release
```

程序启动后常驻系统托盘，关闭窗口不会退出。
