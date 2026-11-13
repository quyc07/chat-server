# chat-server

拥有一个属于自己的聊天App

## 快速开始

### 启动后端程序

```shell
cargo run
```

首次启动会在 `data/` 下创建 SQLite 数据库、自动执行迁移，然后把 HTTP 服务起在 `:3000`。

### 配置

| 变量 | 说明 |
|---|---|
| `JWT_SECRET` | JWT 签名密钥。未设置时回退到 `src/auth.rs` 里硬编码的 `abc`，仅供本地调试 |
| `RUN_MODE` | 读取 `config/<mode>`，默认 `dev`（仓库里暂无该目录，可忽略） |

参考 [`.env.example`](.env.example)。注意程序本身**不加载** `.env`，只有 `docker compose` 会自动读它。

### Docker

```shell
docker compose up --build
```

镜像由 [Dockerfile](Dockerfile) 构建；`JWT_SECRET` 未设置时 compose 会直接报错，而不是悄悄用默认密钥。

## 发布

推一个 `vX.Y.Z` 形式的 tag 会触发 [CD](.github/workflows/cd.yml)：

- 5 个平台的二进制压缩包 + sha256 上传到 GitHub Release（macOS x86_64/arm64、Linux x86_64/arm64、Windows x86_64）
- 容器镜像推到 `ghcr.io/quyc07/chat-server`（tag 来自 git tag）
