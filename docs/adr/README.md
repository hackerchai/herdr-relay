# Herdr Relay 决策索引

Current release: v0.2.1, published 2026-10-10. Implementation, CI, public distribution, service upgrades and actual runtime coverage are recorded in the [release acceptance](../validation/2026-10-10-release-0.2.1.md). Earlier candidate records remain historical evidence.

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

| [ADR-0014](0014-interactive-cli-and-server-lifecycle.md) | Interactive CLI, named hosts, pairing links, server autostart and release updates | 2026-10-10 maintainer request |

| [ADR-0015](0015-public-binary-release-channel.md) | Public binary releases and anonymous updates; private source repository retained | 2026-10-10 maintainer confirmation |

完整范围和验收见 [REQUIREMENTS.md](../../REQUIREMENTS.md)，术语见 [CONTEXT.md](../../CONTEXT.md)。
