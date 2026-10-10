# Herdr Relay 决策索引

更新日期：2026-10-10。ADR-0001–0009 记录产品方向；ADR-0010/0011 的通道、TLS 与配对已进入候选实现并完成受控跨平台验证；ADR-0012 取代 ADR-0005/0010 的同实例并发部分。当前实现覆盖见[候选记录](../validation/2026-10-10-candidates.md)，未完成项见[发布门槛](../release-readiness.md)；决定已接受不等于完整产品验收已完成。

| 决策 | 内容 | 来源 |
|---|---|---|
| [ADR-0001](0001-rust-endpoints-typescript-relay.md) | Rust 端点、TypeScript Cloudflare Worker/DO | 原 DESIGN.md 记录的既有决定 |
| [ADR-0002](0002-hosted-and-self-hosted-relay.md) | 默认共享托管，同时支持开源自建 | Q1 |
| [ADR-0003](0003-accountless-device-pairing.md) | 免注册设备配对，主机本地管理授权 | Q2 |
| [ADR-0004](0004-cli-and-cloudflare-first-release.md) | 首版 CLI 与 Cloudflare Relay，支持用户服务与前台模式 | Q3、Q4、Q11 |
| [ADR-0005](0005-device-roles-and-concurrent-clients.md) | 设备角色对称、同设备多实例；同实例并发部分已被 ADR-0012 取代 | Q5、Q6、Q7 |
| [ADR-0006](0006-directional-device-wide-authorization.md) | 按设备授权全部已共享实例，两个方向独立 | Q8、Q9 |
| [ADR-0007](0007-single-use-pairing-grant.md) | 一次性配对串无需二次确认，兑换要求发布方服务在线 | Q10、Q15 |
| [ADR-0008](0008-session-sharing-and-auto-reconnect.md) | session 重启恢复共享，网络中断自动重新 attach | Q12、Q13 |
| [ADR-0009](0009-public-free-relay-with-quotas.md) | 默认服务公开免费，并设资源配额 | Q14 |
| [ADR-0010](0010-independent-native-client-channels.md) | 每次 attach 独立 native socket 与加密通道 | 本机探针与源码核对；工程设计采用 |
| [ADR-0011](0011-endpoint-tls-and-pairing.md) | 内层 TLS、公钥固定与原子配对事务 | 工程选择；已有受控云端和四平台候选记录 |
| [ADR-0012](0012-single-active-client-per-target.md) | 同实例单活动 Relay 客户端；后来连接报忙且不抢占 | Q16 用户修正；忙碌与释放语义为对应工程选择 |
| [ADR-0013](0013-private-repository-public-crate.md) | 私有 GitHub、MIT 与公开 crates.io；生产域名及 10 美元预算 | 2026-10-10 用户确认 |

完整范围和验收见 [REQUIREMENTS.md](../../REQUIREMENTS.md)，术语见 [CONTEXT.md](../../CONTEXT.md)。
