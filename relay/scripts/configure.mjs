#!/usr/bin/env node
// Generate a private, ignored deployment config; never read deployment credentials.
import { parseArgs } from 'node:util';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve, relative } from 'node:path';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
try {
  const { values } = parseArgs({
    options: {
      name: { type: 'string' },
      origin: { type: 'string' },
      output: { type: 'string' },
      'test-devices': { type: 'string' },
    },
  });
  const { name, origin } = values;
  if (!name || !/^[a-z0-9][a-z0-9-]{0,62}$/.test(name) || !origin)
    throw Error('Provide --name WORKER_NAME --origin https://WORKER_NAME.SUBDOMAIN.workers.dev');
  const url = new URL(origin);
  if (url.protocol !== 'https:' || url.origin !== origin || url.username || url.password)
    throw Error('Origin must be a canonical HTTPS origin without credentials, path or query');
  const devices = values['test-devices'];
  if (devices && !devices.split(',').every((id) => /^[0-9a-f]{64}$/.test(id)))
    throw Error('Test device IDs must be comma-separated lowercase 64-character hex IDs');
  const path = resolve(root, values.output ?? '.wrangler/selfhost.json');
  const config = JSON.parse(readFileSync(resolve(root, 'wrangler.jsonc'), 'utf8'));
  config.name = name;
  config.main = relative(dirname(path), resolve(root, 'src/index.ts'));
  config.$schema = relative(
    dirname(path),
    resolve(root, 'node_modules/wrangler/config-schema.json'),
  );
  config.vars = {
    RELAY_ORIGIN: origin,
    ADMISSION_ENABLED: 'false',
    ...(devices ? { TEST_DEVICE_IDS: devices } : {}),
  };
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(config, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  console.log(
    `Created ${path}\nOrigin: ${origin}\nAdmission is disabled. Review quotas and set ADMISSION_ENABLED to true before use.`,
  );
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
