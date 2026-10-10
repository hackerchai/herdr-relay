# ADR-0001：Rust 端点与 TypeScript Cloudflare 中继

日期：2026-10-09。

状态：已确认方向的追溯记录；来源为 [DESIGN.md](../../DESIGN.md)，不是本轮新增决策。2026-10-10 已有 Rust/TypeScript 实现、受控部署和[四平台候选记录](../validation/2026-10-10-candidates.md)，完整发布验收仍未完成。

## 背景

目标是通过公网中继访问已运行的 Herdr，保留原生客户端界面。两端需要适配本机 IPC 和进程生命周期；云端需要接受 WebSocket 并路由端点之间的数据。

历史研究曾列出全 Rust Worker、Cloudflare Tunnel 等备选。DESIGN.md 明确记录用户于 2026-10-09 确认了语言分工，并将产品定位为不使用 MCP 的 Herdr 专用 Relay 插件。

## 决策

1. 两端使用同一个 Rust 可执行文件，分别提供主机和客户端模式。
2. Cloudflare Worker 与 Durable Object 使用 TypeScript，直接使用 Workers 平台接口。
3. 产品面向 Herdr，MCP 不进入调用链路。

## 影响

- 本机与云端独立构建、发布，通过独立于语言的协议协作。
- 用户使用已构建的本机二进制，无需安装 Rust、Node.js 或 npm；各平台发布支持仍需决定和验证。
- 不将本机 Rust CLI 直接编译上传为云端 Worker。
- 两种语言需要共享协议说明和跨语言验证样例，不能依赖各自类型定义恰好一致。
- 采用 Rust 不改变 Herdr 插件和原生 transport 的现有限制。

## 未由本决策确定

首版支持的平台、托管模式、目标数量、并发策略、配对方式、密码学协议、命令名称和性能指标均未在本 ADR 中定案。参见 [需求澄清](../../REQUIREMENTS.md)。
