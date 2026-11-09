# Herdr Hub

为 Herdr App / herdr-core 提供自托管的设备协调与连接层，使用 Rust，
首版面向 macOS、WSL2 发起端和 Windows Dev Box 远端。

计划中的行为：GitHub 单管理员登录、设备手动审批、中继先可用，
可直连时自动切换 P2P，两条路径均端到端加密。两端首版采用 CLI，TUI 后续增加。

**G1 可行性和 G2 本地 hc 接入已通过，当前不是完整可部署产品。**
仓库包含需求、阶段计划、传输实验、真实 WSL2 bootstrap 样本和原生 hc 桥接组件，
已完成 Windows/Linux 验证及双 reviewer 验收；尚无管理网站、设备控制面或完整网络连接 agent。

- [需求与验收标准](docs/requirements-and-acceptance.md)
- [持久化工作计划](docs/work-plan.md)
- [方案与可行性记录](docs/architecture-proposal.md)
- [hc bootstrap 实测与接入边界](docs/hc-bootstrap.md)
- [WSL2 网络专项分析](docs/wsl-networking.md)
- [G1 阶段验收记录](docs/acceptance/g1.md)
- [G2 本地桥接组件](docs/g2-core-bridge.md)
- [G2 阶段验收记录](docs/acceptance/g2.md)

## 本机实验

安装 Rust 及相应平台的构建工具后，在仓库根目录执行：

```text
cargo fmt --all -- --check
cargo test --workspace --locked -- --nocapture
cargo clippy --workspace --all-targets --locked -- -D warnings
```

探针仅绑定 loopback，关闭默认公共 relay、公共地址发现与端口映射。
实验用的密钥和 TLS 证书在内存中临时生成；不需要 GitHub 或 Azure 凭据。
运行时不使用外部 P2P 服务，首次编译仍需从 crates.io 下载依赖。
本机实验不能代替跨 NAT、App Service、macOS 或 WSL2 真实验收。
另有独立 Linux 测试用户运行的 bootstrap 采集脚本；它不属于 `cargo test`，
需要明确指定 Linux hc 可执行文件，且只记录命令参数，不执行收到的 bootstrap。
