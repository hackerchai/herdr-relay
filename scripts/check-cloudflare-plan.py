#!/usr/bin/env python3
"""Report the deployment's Workers plan before deploying; never purchase or change a plan.

The maintainer selected Workers Paid (USD 5 base). Spending beyond the base is
bounded by the relay's global quotas in wrangler.production.jsonc. Plan names
are reported, not matched, so a provider renaming cannot strand a release
after its crate is published. An unreadable billing API still stops deployment.
"""
import json
import os
import urllib.request

account = os.environ['CLOUDFLARE_ACCOUNT_ID']
token = os.environ['CLOUDFLARE_API_TOKEN']
request = urllib.request.Request(
    f'https://api.cloudflare.com/client/v4/accounts/{account}/subscriptions',
    headers={'Authorization': f'Bearer {token}'},
)
with urllib.request.urlopen(request, timeout=30) as response:
    data = json.load(response)
if not data.get('success') or not isinstance(data.get('result'), list):
    raise SystemExit('Cannot confirm the Workers billing plan; deployment stopped.')
if data.get('result_info', {}).get('total_pages', 1) > 1:
    raise SystemExit('Unexpected subscription pagination; deployment stopped.')


def name(item):
    plan = item.get('rate_plan', {})
    return (plan.get('id', '') + ' ' + plan.get('public_name', '')).lower()


relevant = [
    item for item in data['result']
    if any(word in name(item) for word in ('workers', 'durable object'))
    and item.get('state', '').lower() != 'cancelled'
]
print('Workers plan:', ', '.join(name(item).strip() for item in relevant) or 'Free')


# The public relay stays open to everyone: never deploy it with an access token.
with open('wrangler.production.jsonc') as config:
    if 'RELAY_ACCESS_TOKEN' in config.read():
        raise SystemExit('The public relay must not configure RELAY_ACCESS_TOKEN; deployment stopped.')
secrets = urllib.request.Request(
    f'https://api.cloudflare.com/client/v4/accounts/{account}/workers/scripts/herdr-relay/secrets',
    headers={'Authorization': f'Bearer {token}'},
)
try:
    with urllib.request.urlopen(secrets, timeout=30) as response:
        names = [s.get('name') for s in json.load(response).get('result') or []]
except Exception as error:  # Unreadable secrets cannot enable auth: the Worker ignores it here.
    print('Could not list Worker secrets:', error)
else:
    if 'RELAY_ACCESS_TOKEN' in names:
        raise SystemExit('Remove the RELAY_ACCESS_TOKEN secret from the public relay; deployment stopped.')
    print('Public relay access: open')
