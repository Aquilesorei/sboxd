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

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8000)
