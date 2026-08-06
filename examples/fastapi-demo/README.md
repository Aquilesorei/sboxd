# fastapi-demo

Test rig for sbox's `.env` masking and egress proxy. `.env` has fake secrets
(`SECRET_KEY`, `DATABASE_PASSWORD`); `main.py` exposes routes that try to
read/leak them.

## Run

```bash
# baseline, no sandbox
uv run uvicorn main:app --port 8000

# sbox default: net + env both denied
sbox run uv run uvicorn main:app --port 8000
# GET / -> dotenv_status empty (.env masked to /dev/null), internet_status fails

# sbox with secrets + host-restricted egress (the real test)
sbox run --allow-env --allow-net-out=postman-echo.com uv run uvicorn main:app --port 8000
# GET /exfil-test ->
#   "allowed (postman-echo.com)": "EXFILTRATED (HTTP 200)"
#   "blocked (evil.example.com)": "blocked: ..."
```

`--allow-env` + `--allow-net-out` are both required for a real app like this
(needs the DB password *and* needs to call out). `/exfil-test` proves that
granting both doesn't hand a malicious dependency a free exfil channel: only
the allowlisted host is reachable, everything else 403s at sbox's local
egress proxy.
