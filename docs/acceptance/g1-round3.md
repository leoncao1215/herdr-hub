# G1 工具收尾复验

第二轮两位 reviewer 均认为 G1 可以通过。本轮只复验它们提出的三个非阻塞工具问题，
不改变既定传输方案，也不扩大产品完成声明。

| 问题 | 修复 | 验证 |
| --- | --- | --- |
| HTTP 代理后台 panic 漏报 | 保存 TaskTracker 返回的 JoinHandle，关闭时在任务退出后检查每个结果 | 新增 `proxy_shutdown_reports_background_panics`，注入 panic 后必须返回错误；旧实现会失败 |
| 清理覆盖生命周期主错误 | 主结果、endpoint 清理和 relay 清理都先保存，主结果优先合并 | 原三个生命周期场景保持通过 |
| 采集初始化失败遗留目录 | 临时目录创建之后的所有初始化移入 try/finally；未尝试启动 core 时不发 stop | 两个 Python 回归：连接器写入失败、版本命令失败，均确认临时目录为空；实际三例采集再次通过 |

实际执行：

```text
cargo fmt --all -- --check: exit 0
cargo test --workspace --locked -- --nocapture:
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.25s
cargo clippy --workspace --all-targets --locked -- -D warnings: exit 0

python3 -B -m unittest discover -s experiments/transport-probe/scripts -p "test_*.py" -v:
Ran 2 tests in 0.012s
OK

capture_hc_bootstrap.py ABSOLUTE_LINUX_HC_PATH: exit 0
Final capture still matches the recorded fixture.
```

Rust 检查仍在 Windows loopback 上运行；Python 两例及实际采集在专用 WSL2 普通用户中运行。
panic 回归会有意打印 `intentional proxy test panic`，测试通过要求该 panic 被关闭操作检出，
不是忽略了意外 panic。正常单连接 I/O 中断仍局部记录，不令整个代理退出。

本轮只改变以下四个代码文件；其他代码沿用第二轮快照：

| 文件 | SHA-256 |
| --- | --- |
| `experiments/transport-probe/tests/support/http_proxy.rs` | `9769BC4008C48D917DAC59E851508C332FF738AEBE79F7F777D7FBA50E50ADB4` |
| `experiments/transport-probe/tests/support/lifecycle.rs` | `5C42C68BD895ED71E5D06DBE75383824AEB81B3993632A81D866E70EB5FC47DC` |
| `experiments/transport-probe/scripts/capture_hc_bootstrap.py` | `9048DC6E17054F8AFA1FB081EF133DEACAB0EF3CFEC9AE10DBA901DE75F46790` |
| `experiments/transport-probe/scripts/test_capture_hc_bootstrap.py` | `06C8DC1B47CFBA91A2944B0119E70111D4B5874830E85BF144C31B30A554441E` |

最终结果：Copilot CLI 与 Pi Agent 均已完成限定范围复验，主会话已读取报告。
两者确认三个工具问题闭合，未发现新增阻塞，维持 G1 可通过、可进入 G2 的结论。
G1 总记录已签发通过。两位均只做只读审查，没有冒充独立重跑测试。
