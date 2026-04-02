from pathlib import Path
import json
import os

from rich.console import Console


def main() -> None:
    console = Console()
    output = Path("sandbox_output.json")
    payload = {
        "cwd": os.getcwd(),
        "app_mode": os.environ.get("APP_MODE"),
        "venv": os.environ.get("VIRTUAL_ENV"),
        "message": "hello from uv sandbox",
    }
    output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    console.print(payload["message"], style="bold green")
    console.print(output.resolve())
