import { DurableObject } from 'cloudflare:workers';
import {
  ProtocolError,
  boundedBody,
  digest,
  failure,
  isHex,
  randomHex,
  record,
  route,
  strictJson,
} from './protocol.ts';
import { decodeFrame } from './frame.ts';
import {
  acceptFrame,
  channelLimits,
  initialFlow,
  quotas,
  type Flow,
  type Limits,
} from './limits.ts';
import { restoredChannelValid } from './recovery.ts';
import type { Env } from './index.ts';
import { ADMISSION_OBJECT, admitted } from './admission.ts';
import { PUBLISHER_LEASE_MS } from './admission-ledger.ts';

type Control = {
  kind: 'control';
  device: string;
  boot: string;
  ready: boolean;
  expires: number;
  renewAt?: number;
  /** Optional capabilities the publisher advertised on its control upgrade. */
  features?: string[];
};
type Pair = {
  kind: 'pair';
  purpose: 'pair' | 'control' | 'attach';
  device: string;
  boot: string;
  channel: string;
  reservation: string;
  role: 'publisher' | 'connector';
  ready: boolean;
  expires: number;
  flow: Flow;
  limits: Limits;
  messages: number;
};
type Attachment = Control | Pair;
type Channel = {
  id: string;
  reservation: string;
  publisher: string;
  connector: string;
  boot: string;
  host_ticket: string;
  client_ticket: string;
  host_used: number;
  client_used: number;
  expires: number;
  purpose: string;
  target: string;
  grant_version: number;
  authenticated: number;
  rate: number;
  window: number;
};

export class DeviceRelay extends DurableObject<Env> {
  private readonly instance = randomHex();
  private readonly constructedAt = Date.now();
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS invitations (id TEXT PRIMARY KEY, expires INTEGER NOT NULL)',
    );
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS channels (id TEXT PRIMARY KEY,reservation TEXT NOT NULL,publisher TEXT NOT NULL,connector TEXT NOT NULL,boot TEXT NOT NULL,host_ticket TEXT NOT NULL,client_ticket TEXT NOT NULL,host_used INTEGER NOT NULL,client_used INTEGER NOT NULL,expires INTEGER NOT NULL)',
    );
    const columns = ctx.storage.sql
      .exec<{ name: string }>('PRAGMA table_info(channels)')
      .toArray()
      .map((c) => c.name);
    if (!columns.includes('purpose'))
      ctx.storage.sql.exec("ALTER TABLE channels ADD COLUMN purpose TEXT NOT NULL DEFAULT 'pair'");
    if (!columns.includes('target'))
      ctx.storage.sql.exec("ALTER TABLE channels ADD COLUMN target TEXT NOT NULL DEFAULT ''");
    if (!columns.includes('grant_version'))
      ctx.storage.sql.exec(
        'ALTER TABLE channels ADD COLUMN grant_version INTEGER NOT NULL DEFAULT 0',
      );
    if (!columns.includes('authenticated'))
      ctx.storage.sql.exec(
        'ALTER TABLE channels ADD COLUMN authenticated INTEGER NOT NULL DEFAULT 0',
      );
    if (!columns.includes('rate'))
      ctx.storage.sql.exec('ALTER TABLE channels ADD COLUMN rate INTEGER NOT NULL DEFAULT 262144');
    if (!columns.includes('window'))
      ctx.storage.sql.exec('ALTER TABLE channels ADD COLUMN window INTEGER NOT NULL DEFAULT 0');
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS counters (key TEXT PRIMARY KEY,value INTEGER NOT NULL)',
    );
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS grants (device TEXT PRIMARY KEY, version INTEGER NOT NULL)',
    );
    ctx.storage.sql.exec('CREATE TABLE IF NOT EXISTS targets (id TEXT PRIMARY KEY)');
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS closed_channels (id TEXT PRIMARY KEY, device TEXT NOT NULL, expires INTEGER NOT NULL)',
    );
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS metadata_expiry (id INTEGER PRIMARY KEY CHECK(id=1), expires INTEGER NOT NULL)',
    );
    this.count('constructions');
    this.restore();
  }
  async fetch(request: Request): Promise<Response> {
    try {
      this.ctx.storage.sql.exec(
        'INSERT OR IGNORE INTO metadata_expiry VALUES (1,?)',
        Date.now() + this.retention(),
      );
      const destination = route(request.method, new URL(request.url).pathname);
      const device = request.headers.get('X-Relay-Device');
      if (!isHex(device, 64)) throw new ProtocolError('ACCESS_REQUIRED', 401);
      await this.expire();
      if (destination.operation === 'release')
        return await this.release(request, device, destination.channel!);
      if (destination.operation === 'control')
        return await this.publish(request, device, destination.publisher);
      const control = this.control();
      if (!control) throw new ProtocolError('HOST_OFFLINE', 409);
      const host = control.deserializeAttachment() as Control;
      if (host.device !== destination.publisher) throw new ProtocolError('HOST_OFFLINE', 409);
      if (!host.ready && !['state', 'status'].includes(destination.operation))
        throw new ProtocolError('HOST_OFFLINE', 409);
      if (destination.operation === 'invitations') {
        if (device !== host.device) throw new ProtocolError('NOT_AUTHORIZED', 403);
        const body = record(strictJson(await boundedBody(request)), [
          'invitation_id',
          'expires_at',
        ]);
        const now = Math.floor(Date.now() / 1000);
        if (
          !isHex(body.invitation_id, 32) ||
          !Number.isSafeInteger(body.expires_at) ||
          (body.expires_at as number) <= now ||
          (body.expires_at as number) > now + 600
        )
          throw new ProtocolError();
        const count = this.ctx.storage.sql
          .exec<{ count: number }>('SELECT COUNT(*) AS count FROM invitations')
          .one().count;
        const existing =
          this.ctx.storage.sql
            .exec('SELECT id FROM invitations WHERE id=?', body.invitation_id)
            .toArray().length > 0;
        if (!existing && count >= 128) throw new ProtocolError('QUOTA_EXCEEDED', 429);
        // Only an opaque route, never a redemption secret. Retain the route
        // for the publisher's bounded lost-response recovery interval.
        this.ctx.storage.sql.exec(
          'INSERT OR REPLACE INTO invitations VALUES (?,?)',
          body.invitation_id,
          ((body.expires_at as number) + 86400) * 1000,
        );
        await this.arm();
        return Response.json({ registered: true });
      }
      if (destination.operation === 'authenticated') {
        const body = record(strictJson(await boundedBody(request)), ['boot_id']);
        const row = this.ctx.storage.sql
          .exec<Channel>('SELECT * FROM channels WHERE id=?', destination.channel!)
          .toArray()[0];
        if (
          device !== host.device ||
          body.boot_id !== host.boot ||
          !row ||
          row.boot !== host.boot ||
          !row.host_used ||
          !row.client_used ||
          row.purpose === 'pair' ||
          !this.current(row)
        )
          throw new ProtocolError('NOT_AUTHORIZED', 403);
        await this.env.ADMISSION.getByName(ADMISSION_OBJECT).authenticated(
          'channel:' + row.publisher + ':' + row.id,
          row.reservation,
        );
        if (!this.current(row)) throw new ProtocolError('CHANNEL_EXPIRED', 409);
        this.ctx.storage.sql.exec(
          'UPDATE channels SET authenticated=1 WHERE id=? AND reservation=?',
          row.id,
          row.reservation,
        );
        await this.ctx.storage.sync();
        return Response.json({ authenticated: true });
      }
      if (destination.operation === 'status') {
        if (device !== host.device) throw new ProtocolError('NOT_AUTHORIZED', 403);
        record(strictJson(await boundedBody(request)), []);
        const channels = this.ctx.storage.sql
          .exec<Channel>('SELECT * FROM channels')
          .toArray()
          .map((row) => {
            const sockets = this.channelSockets(row.id, row.reservation).map(
              (ws) => ws.deserializeAttachment() as Pair,
            );
            const a = sockets.find((s) => s.role === 'publisher')?.flow,
              b = sockets.find((s) => s.role === 'connector')?.flow;
            return {
              id: row.id,
              target: row.target,
              purpose: row.purpose,
              pending: !row.authenticated,
              publisher_inflight: a && b ? Number(BigInt(a.sent) - BigInt(b.ack)) : 0,
              connector_inflight: a && b ? Number(BigInt(b.sent) - BigInt(a.ack)) : 0,
              peak: Math.max(a?.peak ?? 0, b?.peak ?? 0),
              roles: sockets.map((s) => ({
                role: s.role,
                sent: s.flow.sent,
                ack: s.flow.ack,
                messages: s.messages,
              })),
            };
          });
        return Response.json({
          instance_id: this.instance,
          constructed_at: this.constructedAt,
          channels,
          counters: this.ctx.storage.sql
            .exec<{ key: string; value: number }>('SELECT * FROM counters')
            .toArray(),
          quotas: quotas(this.env),
        });
      }
      if (destination.operation === 'state') return await this.syncState(request, device, host);
      if (destination.operation === 'access')
        return await this.openPair(request, device, control, host, true);
      if (destination.operation === 'pair')
        return await this.openPair(request, device, control, host);
      if (destination.operation === 'join')
        return await this.join(
          request,
          device,
          destination.channel!,
          destination.role! as Pair['role'],
          host,
        );
      throw new ProtocolError();
    } catch (error) {
      return failure(error);
    } finally {
      this.ctx.waitUntil(this.arm());
    }
  }
  private async publish(request: Request, device: string, publisher: string): Promise<Response> {
    this.upgrade(request);
    const boot = request.headers.get('X-Relay-Boot');
    if (device !== publisher || !isHex(boot, 32)) throw new ProtocolError('NOT_AUTHORIZED', 403);
    const lease = admitted(
      await this.env.ADMISSION.getByName(ADMISSION_OBJECT).hold(
        'publisher',
        'publisher:' + device,
        boot,
        PUBLISHER_LEASE_MS,
      ),
    );
    // A fresh proof from this publisher replaces a half-open older control
    // connection. End its channels before accepting the new boot generation.
    for (const ws of this.ctx.getWebSockets()) this.close(ws, 'HOST_RESTARTED');
    for (const row of this.ctx.storage.sql.exec<Channel>('SELECT * FROM channels').toArray())
      this.endChannel(row.id, row.reservation, 'HOST_RESTARTED');
    this.ctx.storage.sql.exec('DELETE FROM grants');
    this.ctx.storage.sql.exec('DELETE FROM targets');
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, ['control']);
    server.serializeAttachment({
      kind: 'control',
      device,
      boot,
      ready: false,
      expires: lease.expires!,
      renewAt: lease.expires! - PUBLISHER_LEASE_MS / 2,
      features: [...features(request)],
    } satisfies Control);
    server.send(JSON.stringify({ type: 'online', boot_id: boot }));
    await this.arm();
    return new Response(null, { status: 101, webSocket: client });
  }
  private async openPair(
    request: Request,
    device: string,
    control: WebSocket,
    host: Control,
    access = false,
  ): Promise<Response> {
    const limits = quotas(this.env);
    const body = record(
      strictJson(await boundedBody(request)),
      access ? ['channel_id', 'purpose', 'target_id'] : ['invitation_id', 'channel_id'],
    );
    if (!isHex(body.channel_id, 32) || device === host.device) throw new ProtocolError();
    const purpose = access ? body.purpose : 'pair';
    const target = access && purpose === 'attach' ? body.target_id : '';
    let grantVersion = 0;
    if (access) {
      if (
        !['control', 'attach'].includes(purpose as string) ||
        (purpose === 'attach' ? !isHex(target, 32) : body.target_id !== null)
      )
        throw new ProtocolError();
      const grant = this.ctx.storage.sql
        .exec<{ version: number }>('SELECT version FROM grants WHERE device=?', device)
        .toArray()[0];
      if (!grant) throw new ProtocolError('NOT_AUTHORIZED', 403);
      grantVersion = grant.version;
      if (
        purpose === 'attach' &&
        !this.ctx.storage.sql.exec('SELECT id FROM targets WHERE id=?', target as string).toArray()
          .length
      )
        throw new ProtocolError('TARGET_NOT_SHARED', 409);
    } else if (!isHex(body.invitation_id, 32)) throw new ProtocolError();
    const stream = channelLimits(
      limits.stream,
      features(request).has(STREAM_V2) && !!host.features?.includes(STREAM_V2),
    );
    const hostTicket = randomHex(32),
      clientTicket = randomHex(32);
    const [hostHash, clientHash] = await Promise.all([digest(hostTicket), digest(clientTicket)]);
    const expires = Date.now() + 30000;
    const reservation = randomHex();
    if (
      purpose === 'attach' &&
      this.ctx.storage.sql
        .exec("SELECT id FROM channels WHERE purpose='attach' AND target=?", target as string)
        .toArray().length
    )
      throw new ProtocolError('TARGET_BUSY', 409);
    if (
      purpose === 'attach' &&
      !(await this.env.ROOMS.getByName(host.device + ':' + target).reserve(reservation, expires))
    )
      throw new ProtocolError('TARGET_BUSY', 409);
    let globalReserved = false;
    try {
      admitted(
        await this.env.ADMISSION.getByName(ADMISSION_OBJECT).hold(
          'channel',
          'channel:' + host.device + ':' + body.channel_id,
          reservation,
          30000,
        ),
      );
      globalReserved = true;
      this.ctx.storage.transactionSync(() => {
        if (this.control() !== control) throw new ProtocolError('HOST_OFFLINE', 409);
        if (
          this.ctx.storage.sql
            .exec('SELECT id FROM closed_channels WHERE id=?', body.channel_id as string)
            .toArray().length
        )
          throw new ProtocolError('CHANNEL_CONFLICT', 409);
        if (
          access &&
          !this.ctx.storage.sql
            .exec('SELECT device FROM grants WHERE device=? AND version=?', device, grantVersion)
            .toArray().length
        )
          throw new ProtocolError('NOT_AUTHORIZED', 403);
        if (
          purpose === 'attach' &&
          this.ctx.storage.sql
            .exec("SELECT id FROM channels WHERE purpose='attach' AND target=?", target as string)
            .toArray().length
        )
          throw new ProtocolError('TARGET_BUSY', 409);
        if (
          !access &&
          !this.ctx.storage.sql
            .exec(
              'SELECT id FROM invitations WHERE id=? AND expires>?',
              body.invitation_id as string,
              Date.now(),
            )
            .toArray().length
        )
          throw new ProtocolError('PAIR_INVALID', 403);
        if (
          this.ctx.storage.sql
            .exec('SELECT id FROM channels WHERE id=?', body.channel_id as string)
            .toArray().length
        )
          throw new ProtocolError('CHANNEL_CONFLICT', 409);
        const count = (where: string) =>
          this.ctx.storage.sql
            .exec<{ count: number }>('SELECT COUNT(*) AS count FROM channels WHERE ' + where)
            .one().count;
        if (
          count('authenticated=0') >= limits.pending ||
          (purpose === 'attach'
            ? count("purpose='attach'") >= limits.attach
            : count("purpose!='attach'") >= limits.short)
        )
          throw new ProtocolError('QUOTA_EXCEEDED', 429);
        this.ctx.storage.sql.exec(
          'INSERT INTO channels (id,reservation,publisher,connector,boot,host_ticket,client_ticket,host_used,client_used,expires,purpose,target,grant_version,authenticated,rate,window) VALUES (?,?,?,?,?,?,?,0,0,?,?,?,?,0,?,?)',
          body.channel_id as string,
          reservation,
          host.device,
          device,
          host.boot,
          hostHash,
          clientHash,
          expires,
          purpose as string,
          target as string,
          grantVersion,
          stream.bytes_per_second,
          stream.window ?? 0,
        );
      });
    } catch (error) {
      if (globalReserved)
        await this.env.ADMISSION.getByName(ADMISSION_OBJECT).release(
          'channel:' + host.device + ':' + body.channel_id,
          reservation,
        );
      if (purpose === 'attach')
        await this.env.ROOMS.getByName(host.device + ':' + target).release(reservation);
      throw error;
    }
    await this.ctx.storage.sync();
    if (this.control() !== control) {
      this.endChannel(body.channel_id, reservation, 'HOST_OFFLINE');
      throw new ProtocolError('HOST_OFFLINE', 409);
    }
    control.send(
      JSON.stringify(
        access
          ? {
              type: 'access',
              channel_id: body.channel_id,
              connector: device,
              boot_id: host.boot,
              ticket: hostTicket,
              purpose,
              target_id: target || null,
              grant_version: grantVersion,
              limits: stream,
            }
          : {
              type: 'pair',
              channel_id: body.channel_id,
              connector: device,
              boot_id: host.boot,
              ticket: hostTicket,
            },
      ),
    );
    await this.arm();
    return Response.json(
      {
        channel_id: body.channel_id,
        boot_id: host.boot,
        ticket: clientTicket,
        grant_version: access ? grantVersion : null,
        limits: stream,
      },
      { headers: { 'Cache-Control': 'no-store' } },
    );
  }
  private async join(
    request: Request,
    device: string,
    channel: string,
    role: Pair['role'],
    host: Control,
  ): Promise<Response> {
    this.upgrade(request);
    const ticket = request.headers.get('X-Relay-Ticket');
    if (!isHex(ticket, 64)) throw new ProtocolError('TICKET_INVALID', 403);
    const hash = await digest(ticket);
    const row = this.ctx.storage.transactionSync(() => {
      const row = this.ctx.storage.sql
        .exec<Channel>('SELECT * FROM channels WHERE id=?', channel)
        .toArray()[0];
      if (!row || row.expires <= Date.now() || row.boot !== host.boot || !this.control())
        throw new ProtocolError('TICKET_INVALID', 403);
      if (
        row.purpose !== 'pair' &&
        !this.ctx.storage.sql
          .exec(
            'SELECT device FROM grants WHERE device=? AND version=?',
            row.connector,
            row.grant_version,
          )
          .toArray().length
      )
        throw new ProtocolError('NOT_AUTHORIZED', 403);
      const isHost = role === 'publisher';
      if (
        device !== (isHost ? row.publisher : row.connector) ||
        hash !== (isHost ? row.host_ticket : row.client_ticket)
      )
        throw new ProtocolError('TICKET_INVALID', 403);
      if (isHost ? row.host_used : row.client_used)
        throw new ProtocolError('CHANNEL_CONFLICT', 409);
      this.ctx.storage.sql.exec(
        isHost
          ? 'UPDATE channels SET host_used=1 WHERE id=?'
          : 'UPDATE channels SET client_used=1 WHERE id=?',
        channel,
      );
      return row;
    });
    // There is no await between consuming a role and accepting its socket.
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, ['channel:' + channel]);
    server.serializeAttachment({
      kind: 'pair',
      device,
      boot: row.boot,
      channel,
      reservation: row.reservation,
      role,
      purpose: row.purpose as Pair['purpose'],
      ready: false,
      expires: row.expires,
      flow: initialFlow(Date.now(), rowLimits(row)),
      limits: rowLimits(row),
      messages: 0,
    } satisfies Pair);
    const peers = this.channelSockets(channel, row.reservation);
    if (peers.length === 2) {
      if (row.purpose === 'attach') {
        let globalExpiry: number;
        try {
          globalExpiry = admitted(
            await this.env.ADMISSION.getByName(ADMISSION_OBJECT).renew(
              'channel:' + row.publisher + ':' + row.id,
              row.reservation,
            ),
          ).expires!;
        } catch (error) {
          this.endChannel(
            channel,
            row.reservation,
            error instanceof ProtocolError ? error.code : 'RELAY_BUSY',
          );
          throw error;
        }
        this.count('room_renewals');
        const roomExpiry = await this.env.ROOMS.getByName(row.publisher + ':' + row.target).renew(
          row.reservation,
        );
        const expires = roomExpiry ? Math.min(roomExpiry, globalExpiry) : null;
        if (
          !expires ||
          !this.current(row) ||
          !this.control() ||
          this.channelSockets(channel, row.reservation).length !== 2
        ) {
          this.endChannel(channel, row.reservation, 'CHANNEL_EXPIRED');
          throw new ProtocolError('CHANNEL_EXPIRED', 409);
        }
        this.ctx.storage.sql.exec(
          'UPDATE channels SET expires=? WHERE id=? AND reservation=?',
          expires,
          channel,
          row.reservation,
        );
        await this.arm();
      }
      for (const ws of peers) {
        const attachment = ws.deserializeAttachment() as Pair;
        attachment.ready = true;
        ws.serializeAttachment(attachment);
        ws.send(JSON.stringify({ type: 'route_ready', channel_id: channel, boot_id: row.boot }));
      }
    }
    return new Response(null, { status: 101, webSocket: client });
  }
  webSocketMessage(ws: WebSocket, message: ArrayBuffer | string): void {
    const state = ws.deserializeAttachment() as Attachment | null;
    if (!state) {
      this.close(ws, 'PROTOCOL_ERROR');
      return;
    }
    if (state.kind === 'control') {
      this.endControl(ws, state.boot);
      return;
    }
    try {
      const host = this.control()?.deserializeAttachment() as Control | undefined;
      if (this.env.ADMISSION_ENABLED !== 'true' || !host || host.expires <= Date.now())
        throw new ProtocolError('RELAY_BUSY', 503);
      const row = this.ctx.storage.sql
        .exec<Channel>(
          'SELECT * FROM channels WHERE id=? AND reservation=?',
          state.channel,
          state.reservation,
        )
        .toArray()[0];
      if (typeof message === 'string' || !state.ready || !row || row.expires <= Date.now())
        throw new ProtocolError();
      const frame = decodeFrame(message);
      state.messages = Math.min(state.messages + 1, Number.MAX_SAFE_INTEGER);
      if (frame.channel !== state.channel || (state.purpose !== 'attach' && state.messages > 256))
        throw new ProtocolError();
      const peers = this.channelSockets(state.channel, state.reservation).filter(
        (peer) => peer !== ws,
      );
      if (peers.length !== 1) throw new ProtocolError();
      const peer = peers[0],
        other = peer.deserializeAttachment() as Pair;
      if (other.boot !== state.boot || other.role === state.role || !other.ready)
        throw new ProtocolError();
      if (!state.flow || !other.flow) throw new ProtocolError('PROTOCOL_CHANGED', 409);
      const now = Date.now();
      if (
        (BigInt(state.flow.sent) > BigInt(other.flow.ack) && state.flow.progress + 30000 <= now) ||
        (BigInt(other.flow.sent) > BigInt(state.flow.ack) && other.flow.progress + 30000 <= now)
      )
        throw new ProtocolError('SLOW_CONSUMER', 409);
      if (
        state.purpose !== 'attach' &&
        frame.kind === 1 &&
        BigInt(state.flow.sent) + BigInt(frame.payload.length) > 65536n
      )
        throw new ProtocolError();
      const firstOutstanding = frame.kind === 1 && state.flow.sent === other.flow.ack;
      const accepted = acceptFrame(state.flow, other.flow, frame, Date.now(), state.limits);
      state.flow = accepted.state;
      other.flow = accepted.peer;
      ws.serializeAttachment(state);
      peer.serializeAttachment(other);
      if (accepted.forward) peer.send(message);
      if (firstOutstanding) this.ctx.waitUntil(this.arm());
    } catch (error) {
      this.endChannel(
        state.channel,
        state.reservation,
        error instanceof ProtocolError ? error.code : 'PROTOCOL_ERROR',
      );
    }
  }
  webSocketClose(ws: WebSocket): void {
    this.disconnected(ws);
  }
  webSocketError(ws: WebSocket): void {
    this.disconnected(ws);
  }
  private disconnected(ws: WebSocket): void {
    const state = ws.deserializeAttachment() as Attachment | null;
    if (state?.kind === 'pair')
      this.endChannel(state.channel, state.reservation, 'CONNECTION_LOST');
    else if (state?.kind === 'control') this.endControl(ws, state.boot);
  }
  private endControl(ws: WebSocket, boot: string): void {
    const host = ws.deserializeAttachment() as Control;
    this.ctx.waitUntil(
      this.env.ADMISSION.getByName(ADMISSION_OBJECT).release('publisher:' + host.device, boot),
    );
    this.ctx.storage.sql.exec(
      'INSERT OR REPLACE INTO metadata_expiry VALUES (1,?)',
      Date.now() + this.retention(),
    );
    this.close(ws, 'HOST_OFFLINE');
    const rows = this.ctx.storage.sql
      .exec<{ id: string; reservation: string }>(
        'SELECT id,reservation FROM channels WHERE boot=?',
        boot,
      )
      .toArray();
    for (const row of rows) this.endChannel(row.id, row.reservation, 'HOST_OFFLINE');
    this.ctx.waitUntil(this.arm());
  }
  private endChannel(channel: string, reservation: string, reason: string): void {
    const sockets = this.channelSockets(channel, reservation);
    const peak = Math.max(
      0,
      ...sockets.map((ws) => {
        const n = (ws.deserializeAttachment() as Pair).flow?.peak;
        return Number.isInteger(n) && n >= 0 && n <= 65536 ? n : 0;
      }),
    );
    if (peak)
      this.ctx.storage.sql.exec(
        "INSERT INTO counters VALUES ('peak_inflight',?) ON CONFLICT(key) DO UPDATE SET value=MAX(value,excluded.value)",
        peak,
      );
    if (
      sockets.length &&
      [
        'SLOW_CONSUMER',
        'RATE_LIMITED',
        'PROTOCOL_ERROR',
        'STATE_CONFLICT',
        'GLOBAL_QUOTA_EXCEEDED',
        'RELAY_BUSY',
      ].includes(reason)
    )
      this.ctx.storage.sql.exec(
        'INSERT INTO counters VALUES (?,1) ON CONFLICT(key) DO UPDATE SET value=MIN(value+1,9007199254740991)',
        'closed_' + reason.toLowerCase(),
      );
    for (const ws of sockets) this.close(ws, reason);
    const rows = this.ctx.storage.sql
      .exec<Channel>(
        'DELETE FROM channels WHERE id=? AND reservation=? RETURNING *',
        channel,
        reservation,
      )
      .toArray();
    for (const row of rows) {
      this.ctx.waitUntil(
        this.env.ADMISSION.getByName(ADMISSION_OBJECT).release(
          'channel:' + row.publisher + ':' + row.id,
          reservation,
        ),
      );
      if (row.purpose === 'attach')
        this.ctx.waitUntil(
          this.env.ROOMS.getByName(row.publisher + ':' + row.target).release(reservation),
        );
    }
  }
  private async release(request: Request, device: string, channel: string): Promise<Response> {
    record(strictJson(await boundedBody(request)), []);
    // Also record a release whose open response was lost. The tombstone stops
    // a delayed open transaction from creating a channel after cancellation.
    const row = this.ctx.storage.transactionSync(() => {
      const row = this.ctx.storage.sql
        .exec<Channel>('SELECT * FROM channels WHERE id=?', channel)
        .toArray()[0];
      const closed = this.ctx.storage.sql
        .exec<{ device: string }>('SELECT device FROM closed_channels WHERE id=?', channel)
        .toArray()[0];
      if ((row && row.connector !== device) || (closed && closed.device !== device))
        throw new ProtocolError('NOT_AUTHORIZED', 403);
      if (
        !closed &&
        this.ctx.storage.sql
          .exec<{ count: number }>(
            'SELECT COUNT(*) AS count FROM closed_channels WHERE device=?',
            device,
          )
          .one().count >= 128
      )
        throw new ProtocolError('RATE_LIMITED', 429);
      this.ctx.storage.sql.exec(
        'INSERT OR REPLACE INTO closed_channels VALUES (?,?,?)',
        channel,
        device,
        Date.now() + 120000,
      );
      if (row) this.endChannel(channel, row.reservation, 'CONNECTION_LOST');
      return row;
    });
    // Await the exact reservation release before acknowledging cleanup. Late
    // callbacks still compare reservation IDs and cannot free a newer owner.
    if (row?.purpose === 'attach')
      await this.env.ROOMS.getByName(row.publisher + ':' + row.target).release(row.reservation);
    if (row)
      await this.env.ADMISSION.getByName(ADMISSION_OBJECT).release(
        'channel:' + row.publisher + ':' + row.id,
        row.reservation,
      );
    await this.ctx.storage.sync();
    await this.arm();
    return Response.json({ released: true });
  }
  private control(): WebSocket | undefined {
    const sockets = this.ctx
      .getWebSockets('control')
      .filter((ws) => ws.readyState === WebSocket.OPEN);
    return sockets.length === 1 ? sockets[0] : undefined;
  }
  private channelSockets(channel: string, reservation: string): WebSocket[] {
    return this.ctx
      .getWebSockets('channel:' + channel)
      .filter(
        (ws) =>
          ws.readyState === WebSocket.OPEN &&
          (ws.deserializeAttachment() as Pair).reservation === reservation,
      );
  }
  private close(ws: WebSocket, reason: string): void {
    try {
      ws.close(4000, reason);
    } catch {
      /* already closed */
    }
  }
  private upgrade(request: Request): void {
    if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket')
      throw new ProtocolError('UPGRADE_REQUIRED', 426);
  }
  private async expire(): Promise<void> {
    const now = Date.now();
    const control = this.control(),
      host = control?.deserializeAttachment() as Control | undefined;
    if (
      control &&
      host &&
      (this.env.ADMISSION_ENABLED !== 'true' ||
        !Number.isSafeInteger(host.expires) ||
        host.expires <= now)
    )
      this.endControl(control, host.boot);
    for (const row of this.ctx.storage.sql
      .exec<{ id: string; reservation: string }>(
        'SELECT id,reservation FROM channels WHERE expires<=?',
        now,
      )
      .toArray())
      this.endChannel(row.id, row.reservation, 'CHANNEL_EXPIRED');
    this.ctx.storage.sql.exec('DELETE FROM invitations WHERE expires<=?', now);
    this.ctx.storage.sql.exec('DELETE FROM closed_channels WHERE expires<=?', now);
    if (
      !this.control() &&
      !this.ctx.storage.sql.exec('SELECT id FROM channels LIMIT 1').toArray().length &&
      !this.ctx.storage.sql.exec('SELECT id FROM metadata_expiry WHERE expires>?', now).toArray()
        .length
    ) {
      for (const table of [
        'grants',
        'targets',
        'counters',
        'invitations',
        'closed_channels',
        'metadata_expiry',
      ])
        this.ctx.storage.sql.exec('DELETE FROM ' + table);
    }
  }
  private count(key: string): void {
    this.ctx.storage.sql.exec(
      'INSERT INTO counters VALUES (?,1) ON CONFLICT(key) DO UPDATE SET value=MIN(value+1,9007199254740991)',
      key,
    );
  }
  private restore(): void {
    // Reconstruct only from both durable authorities: role attachments and SQL.
    // No replacement socket or fresh credit is inferred from a consumed ticket.
    const sockets = this.ctx.getWebSockets().filter((ws) => ws.readyState === WebSocket.OPEN);
    const attached = new Map<WebSocket, Attachment>();
    for (const ws of sockets) {
      try {
        const s = ws.deserializeAttachment() as Attachment | null;
        if (
          !s ||
          !['control', 'pair'].includes(s.kind) ||
          !isHex(s.device, 64) ||
          !isHex(s.boot, 32)
        )
          throw Error();
        if (
          s.kind === 'control' &&
          (typeof s.ready !== 'boolean' ||
            !Number.isSafeInteger(s.expires) ||
            s.expires <= Date.now() ||
            (s.renewAt !== undefined &&
              (!Number.isSafeInteger(s.renewAt) || s.renewAt > s.expires)))
        )
          throw Error();
        attached.set(ws, s);
      } catch {
        this.close(ws, 'STATE_CONFLICT');
      }
    }
    const controls = [...attached.entries()].filter(([, s]) => s.kind === 'control');
    const host = controls.length === 1 ? (controls[0][1] as Control) : undefined;
    if (!host) for (const ws of attached.keys()) this.close(ws, 'STATE_CONFLICT');
    const rows = this.ctx.storage.sql.exec<Channel>('SELECT * FROM channels').toArray();
    for (const [ws, s] of attached)
      if (
        s.kind === 'pair' &&
        !rows.some((r) => r.id === s.channel && r.reservation === s.reservation)
      )
        this.close(ws, 'STATE_CONFLICT');
    for (const row of rows) {
      const peers = [...attached.values()].filter(
        (s) => s.kind === 'pair' && s.channel === row.id && s.reservation === row.reservation,
      );
      const grant =
        row.purpose === 'pair' ||
        this.ctx.storage.sql
          .exec(
            'SELECT device FROM grants WHERE device=? AND version=?',
            row.connector,
            row.grant_version,
          )
          .toArray().length === 1;
      const target =
        row.purpose !== 'attach' ||
        (rows.filter((r) => r.purpose === 'attach' && r.target === row.target).length === 1 &&
          this.ctx.storage.sql.exec('SELECT id FROM targets WHERE id=?', row.target).toArray()
            .length === 1);
      if (
        !host ||
        row.publisher !== host.device ||
        row.boot !== host.boot ||
        !grant ||
        !target ||
        !restoredChannelValid(row, peers, Date.now())
      )
        this.endChannel(row.id, row.reservation, 'STATE_CONFLICT');
    }
  }
  private async arm(): Promise<void> {
    const deadlines = this.ctx.storage.sql
      .exec<{ expires: number }>(
        'SELECT expires FROM invitations UNION ALL SELECT expires FROM channels UNION ALL SELECT expires FROM closed_channels',
      )
      .toArray();
    const control = this.control();
    if (!control)
      deadlines.push(
        ...this.ctx.storage.sql
          .exec<{ expires: number }>('SELECT expires FROM metadata_expiry')
          .toArray(),
      );
    const active =
      this.ctx.storage.sql.exec("SELECT id FROM channels WHERE purpose='attach' LIMIT 1").toArray()
        .length > 0;
    if (control) {
      const host = control.deserializeAttachment() as Control;
      deadlines.push({
        expires: Math.min(host.expires, host.renewAt ?? host.expires - PUBLISHER_LEASE_MS / 2),
      });
    }
    for (const ws of this.ctx.getWebSockets()) {
      const s = ws.deserializeAttachment() as Attachment;
      if (s?.kind !== 'pair' || !s.flow) continue;
      const other = this.channelSockets(s.channel, s.reservation)
        .find((p) => p !== ws)
        ?.deserializeAttachment() as Pair | undefined;
      if (other?.flow && BigInt(s.flow.sent) > BigInt(other.flow.ack))
        deadlines.push({ expires: s.flow.progress + 30000 });
    }
    if (deadlines.length) {
      const next = Math.max(
        Date.now() + 1,
        Math.min(active ? Date.now() + 30000 : Infinity, ...deadlines.map((d) => d.expires)),
      );
      const current = await this.ctx.storage.getAlarm();
      if (current === null || next < current) await this.ctx.storage.setAlarm(next);
    } else await this.ctx.storage.deleteAlarm();
  }
  private current(row: Channel): boolean {
    return (
      this.ctx.storage.sql
        .exec(
          'SELECT id FROM channels WHERE id=? AND reservation=? AND expires>?',
          row.id,
          row.reservation,
          Date.now(),
        )
        .toArray().length === 1
    );
  }
  async alarm(): Promise<void> {
    this.count('alarms');
    await this.expire();
    const control = this.control(),
      host = control?.deserializeAttachment() as Control | undefined;
    if (control && host && (host.renewAt ?? host.expires - PUBLISHER_LEASE_MS / 2) <= Date.now()) {
      try {
        const lease = admitted(
          await this.env.ADMISSION.getByName(ADMISSION_OBJECT).renew(
            'publisher:' + host.device,
            host.boot,
          ),
        );
        if (this.control() === control)
          control.serializeAttachment({
            ...(control.deserializeAttachment() as Control),
            expires: lease.expires!,
            renewAt: lease.expires! - PUBLISHER_LEASE_MS / 2,
          });
      } catch (error) {
        if (this.control() === control) {
          if (
            host.expires <= Date.now() ||
            (error instanceof ProtocolError &&
              !['RELAY_BUSY', 'GLOBAL_QUOTA_EXCEEDED', 'RATE_LIMITED'].includes(error.code))
          )
            this.endControl(control, host.boot);
          else
            control.serializeAttachment({
              ...(control.deserializeAttachment() as Control),
              // Only the existing authority is retained. A failed RPC cannot
              // invent capacity or extend the lease, even across hibernation.
              renewAt: Math.min(
                host.expires,
                Date.now() +
                  Math.max(
                    30000,
                    error instanceof ProtocolError ? (error.retryAfter ?? 0) * 1000 : 0,
                  ),
              ),
            });
        }
      }
    }
    const rows = this.ctx.storage.sql
      .exec<Channel>("SELECT * FROM channels WHERE purpose='attach'")
      .toArray();
    for (const row of rows) {
      const peers = this.channelSockets(row.id, row.reservation);
      if (peers.length !== 2 || !peers.every((ws) => (ws.deserializeAttachment() as Pair).ready))
        continue;
      const [a, b] = peers.map((ws) => (ws.deserializeAttachment() as Pair).flow);
      if (
        !a ||
        !b ||
        (BigInt(a.sent) > BigInt(b.ack) && a.progress + 30000 <= Date.now()) ||
        (BigInt(b.sent) > BigInt(a.ack) && b.progress + 30000 <= Date.now())
      ) {
        this.endChannel(row.id, row.reservation, 'SLOW_CONSUMER');
        continue;
      }
      this.count('room_renewals');
      let globalExpiry: number;
      try {
        globalExpiry = admitted(
          await this.env.ADMISSION.getByName(ADMISSION_OBJECT).renew(
            'channel:' + row.publisher + ':' + row.id,
            row.reservation,
          ),
        ).expires!;
      } catch (error) {
        this.endChannel(
          row.id,
          row.reservation,
          error instanceof ProtocolError ? error.code : 'RELAY_BUSY',
        );
        continue;
      }
      const roomExpiry = await this.env.ROOMS.getByName(row.publisher + ':' + row.target).renew(
        row.reservation,
      );
      const expires = roomExpiry ? Math.min(roomExpiry, globalExpiry) : null;
      if (!expires || !this.current(row) || !this.control()) {
        this.endChannel(row.id, row.reservation, 'CHANNEL_EXPIRED');
        continue;
      }
      this.ctx.storage.sql.exec(
        'UPDATE channels SET expires=? WHERE id=? AND reservation=?',
        expires,
        row.id,
        row.reservation,
      );
    }
    await this.arm();
  }
  private retention(): number {
    const value = this.env.METADATA_RETENTION_HOURS ?? '25';
    if (!/^[1-9][0-9]*$/.test(value) || Number(value) > 168)
      throw new ProtocolError('RELAY_UNCONFIGURED', 503);
    return Number(value) * 3600000;
  }
  private async syncState(request: Request, device: string, host: Control): Promise<Response> {
    if (device !== host.device) throw new ProtocolError('NOT_AUTHORIZED', 403);
    const body = record(strictJson(await boundedBody(request)), ['boot_id', 'grants', 'targets']);
    if (
      body.boot_id !== host.boot ||
      !Array.isArray(body.grants) ||
      body.grants.length > 64 ||
      !Array.isArray(body.targets) ||
      body.targets.length > 8
    )
      throw new ProtocolError();
    const grants = body.grants.map((value) => {
      const g = record(value, ['device_id', 'version']);
      if (!isHex(g.device_id, 64) || !Number.isSafeInteger(g.version) || (g.version as number) < 1)
        throw new ProtocolError();
      return g;
    });
    if (body.targets.some((t) => !isHex(t, 32))) throw new ProtocolError();
    this.ctx.storage.transactionSync(() => {
      if ((this.control()?.deserializeAttachment() as Control | undefined)?.boot !== host.boot)
        throw new ProtocolError('HOST_OFFLINE', 409);
      this.ctx.storage.sql.exec('DELETE FROM grants');
      this.ctx.storage.sql.exec('DELETE FROM targets');
      for (const g of grants)
        this.ctx.storage.sql.exec(
          'INSERT INTO grants VALUES (?,?)',
          g.device_id as string,
          g.version as number,
        );
      for (const target of body.targets as string[])
        this.ctx.storage.sql.exec('INSERT INTO targets VALUES (?)', target);
      const control = this.control()!;
      control.serializeAttachment({ ...(control.deserializeAttachment() as Control), ready: true });
    });
    const channels = this.ctx.storage.sql
      .exec<Channel>("SELECT * FROM channels WHERE purpose!='pair'")
      .toArray();
    for (const row of channels) {
      if (
        !this.ctx.storage.sql
          .exec(
            'SELECT device FROM grants WHERE device=? AND version=?',
            row.connector,
            row.grant_version,
          )
          .toArray().length
      )
        this.endChannel(row.id, row.reservation, 'NOT_AUTHORIZED');
      else if (
        row.purpose === 'attach' &&
        !this.ctx.storage.sql.exec('SELECT id FROM targets WHERE id=?', row.target).toArray().length
      )
        this.endChannel(row.id, row.reservation, 'TARGET_OFFLINE');
    }
    await this.ctx.storage.sync();
    return Response.json({ registered: true });
  }
}

export const STREAM_V2 = 'stream-v2';
/** Capabilities an endpoint advertises; unknown names are ignored. */
export function features(request: Request): Set<string> {
  return new Set(
    (request.headers.get('X-Relay-Features') ?? '')
      .split(',')
      .map((f) => f.trim())
      .filter((f) => /^[a-z0-9-]{1,32}$/.test(f))
      .slice(0, 16),
  );
}
function rowLimits(row: Channel): Limits {
  return row.window
    ? { bytes_per_second: row.rate, window: row.window }
    : { bytes_per_second: row.rate };
}
