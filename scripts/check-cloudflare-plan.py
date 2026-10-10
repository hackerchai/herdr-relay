#!/usr/bin/env python3
"""Keep this deployment on Workers Free; never purchase or change a plan."""
import json
import os
import sys
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
paid = [
    item for item in data['result']
    if any(word in (item.get('rate_plan', {}).get('id', '') + ' ' + item.get('rate_plan', {}).get('public_name', '')).lower()
           for word in ('workers', 'durable object'))
    and item.get('state', '').lower() != 'cancelled'
]
if paid:
    print('A Workers/DO subscription is present. The USD 10 policy requires a new billing review.', file=sys.stderr)
    raise SystemExit(1)
print('Workers paid subscription absent: retain Free plan and provider-enforced free quotas.')
