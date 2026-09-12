"""Opt-in end-to-end repair drill. Uses the real Codex login in an isolated copy.

Injects a deterministic response-parser defect, reproduces it, and checks that
the independent agent repairs, tests, activates and resumes that conversation.
Never touches the real Studio data or original source. Run from the repo root.
"""
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time

import klyne_recovery as recovery


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class Model(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        context = json.loads(body["messages"][1]["content"])
        value = {"summary":"Answer the greeting", "tasks":[{"agent":"Assistant","instruction":"Say hello"}]} if context["role"] == "planner" else {"decision":"complete","summary":"RECOVERY_PROBE hello"}
        response = json.dumps({"message":{"content":json.dumps(value)}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, *_):
        pass


def main():
    repository = Path(__file__).resolve().parent.parent
    report = repository / "workspace" / f"recovery-drill-{int(time.time())}"
    report.mkdir()
    source = report / "original"
    recovery.source_snapshot(repository, source)
    parser = source / "apps/studio/src/chat.rs"
    text = parser.read_text(encoding="utf-8")
    needle = 'fn parse_model_response(output: &str) -> io::Result<Value> {'
    assert needle in text
    parser.write_text(text.replace(needle, needle + '\n    if output.contains("RECOVERY_PROBE") { return Err(err("Parser rejects a valid response containing RECOVERY_PROBE")); }', 1), encoding="utf-8")
    (source / ".gitignore").write_text("/target/\n", encoding="utf-8")
    subprocess.run(["git", "init", str(source)], check=True, capture_output=True)
    subprocess.run(["git", "add", "-A"], cwd=source, check=True, capture_output=True)
    recovery.run(["cargo","build","--offline","-p","klyne-studio","--bins"], source,
                 report / "build-injected.log", threading.Event())
    binary = source / "target/debug" / ("klyne-studio.exe" if os.name == "nt" else "klyne-studio")
    supervisor = binary.with_name("klyne-supervisor" + binary.suffix)
    port, recovery_port = free_port(), free_port()
    runtime = report / "runtime"
    config = {"enabled":True,"automatic_code_repair":True}
    if "--local" in sys.argv:
        installed = recovery.api(11434, "/api/tags")["models"]
        config["repair_provider"] = {"kind":"ollama","endpoint":"http://127.0.0.1:11434","model":installed[0]["name"]}
    recovery.atomic_json(runtime / "recovery/config.json", config)
    model = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Model)
    threading.Thread(target=model.serve_forever, daemon=True).start()
    with (report / "guardian.log").open("wb") as log:
        guardian = recovery.spawn([sys.executable, Path(recovery.__file__).resolve(), "--source", source,
            "--root", runtime, "--binary", binary, "--supervisor", supervisor,
            "--port", port, "--recovery-port", recovery_port], stdout=log, stderr=log)
    print(f"Drill report: {report}", flush=True)
    try:
        for _ in range(100):
            try:
                recovery.api(port, "/api/runtime/ready")
                break
            except OSError:
                time.sleep(.2)
        chat = recovery.api(port, "/api/chats", {"message":"Say hello", "access":{}, "provider":{
            "kind":"ollama", "model":"fixture", "endpoint":f"http://127.0.0.1:{model.server_port}"}})
        deadline = time.monotonic() + 900
        previous = None
        while time.monotonic() < deadline:
            status = recovery.api(recovery_port, "/status")
            if status.get("phase") != previous:
                print(status.get("phase"), status.get("message"), flush=True)
                previous = status.get("phase")
            if previous == "Needs attention":
                raise RuntimeError(status["message"])
            if previous == "Resumed":
                result = recovery.api(port, f"/api/chats/{chat['id']}")
                if result["status"] == "Completed":
                    recovery.atomic_json(report / "verified.json", result)
                    print("PASS: injected failure repaired, independently tested, activated and original conversation completed.", flush=True)
                    return
            time.sleep(1)
        raise RuntimeError("Repair drill deadline exceeded")
    finally:
        recovery.terminate(guardian)
        model.shutdown()


if __name__ == "__main__":
    main()
