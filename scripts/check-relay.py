#!/usr/bin/env python3
"""Deployment protocol probe, with no key, invitation or private profile."""
import json
import sys
import time
import urllib.error
import urllib.request

origin = sys.argv[1].rstrip('/')
request = urllib.request.Request(origin + '/v1/challenge', data=b'{}', headers={
    'Content-Type': 'application/json',
    'User-Agent': 'herdr-relay-deployment-check/1',
})
for attempt in range(5):
    try:
        urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        # A freshly deployed Worker can briefly shed load; retry like 5xx.
        if (error.code >= 500 or error.code == 429) and attempt < 4:
            retry = error.headers.get('Retry-After', '')
            time.sleep(min(int(retry), 30) if retry.isdigit() else 5)
            continue
        if error.code != 400 or json.load(error).get('code') != 'PROTOCOL_ERROR':
            raise SystemExit(f'Unexpected protocol response: HTTP {error.code}')
        print('Production TLS and protocol validation passed.')
        break
    except urllib.error.URLError:
        if attempt == 4:
            raise
        time.sleep(2)
    else:
        raise SystemExit('Malformed unsigned request was unexpectedly accepted.')
