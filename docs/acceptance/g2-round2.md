# G2 退出竞态复验

状态：修复及两平台验证完成，两位 reviewer 独立复验均通过，主会话已读取并处理结果。
本轮只修改桥接库上行复制，并新增 `tests/pipe_failure.rs`；其他 G2 实现保持原快照。

## 问题复现与修复

原实现在 hc 子进程关闭 stdin 时，将上行 BrokenPipe 当作整体失败，立即丢弃下行。
在 Windows 上加入持续大输入、4 字节下行缓冲、延迟消费的回归后，
原实现只收到 `before-failure` 的前 4 字节，而不是完整 14 字节，回归失败。

修复后的上行复制使用固定 8 KiB 缓冲，明确区分：

- 输入读取错误：仍按错误处理，执行清理，不误当成 hc 正常关闭。
- hc stdin 写入或 shutdown 的 BrokenPipe：停止上行，继续可取消地排空 stdout，
  最后等待子进程并保留原始退出码。
- 其他子 stdin 写入错误：仍按错误处理。

三个新增用例：

1. `closed_child_input_does_not_truncate_its_final_response`：
   等待上行停止后才消费下行；收到完整 14 字节及 `ChildExit(17)`。
2. `broken_input_reader_is_not_mistaken_for_closed_child_stdin`：
   输入读取本身报 BrokenPipe 时仍返回 I/O 错误。
3. `cancellation_remains_available_while_draining_a_closed_child`：
   下行保持背压时取消仍能结束并回收子进程。

## 实际复验

Windows 和 WSL2/Linux 各执行桥接 crate 的 3 个 unit、6 个原 integration、
3 个新增 regression，**各 12 个用例通过**。
双方 `clippy --all-targets --locked -- -D warnings` 通过，`cargo fmt --all -- --check` 通过。
没有重做无关 G1 套件。

两平台真实 hc 0.1.19 smoke 再次退出 0：

- Windows：plain/framed RPC、原终端存续、core 身份/PID 不变、独立 core/目录清理均通过。
- Linux：上述项目及实际 `machine add --via`、0–255 二进制 exec、
  移除并重加 machine 后原终端存续均通过。
- Windows 原生 `--via` 仍不适用；macOS、网络传输、OAuth 等边界没有扩大。

## 更新的审查快照

| 文件 | SHA-256 |
| --- | --- |
| `crates/core-bridge/src/lib.rs` | `5C36570EDADEDDA64D2F580B080FC1C3DBE663E6FB7D0DC76D30C0B804D5AA8F` |
| `crates/core-bridge/tests/pipe_failure.rs` | `5CE56B60FCF6E5A771DBF8FC50BEB16E3B2CF68F37C97F0C5F38A5BF10657D99` |

原始失败和修复后运行日志保存在本地 Git 元数据目录；
以上结果为主会话执行证据，reviewer 只读核对，不重新运行命令。

最终结论：Copilot CLI 与 Pi 均确认错误来源区分正确、尾响应及退出码保留、
排空可取消且回归有针对性；首轮问题闭合，无新增高置信阻塞，G2 可以通过。
