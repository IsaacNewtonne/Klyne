"""Exercise real Studio speech routes, cancellation and worker restart. No microphone."""
import concurrent.futures
import http.client
import importlib.util
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time

repo = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('worker', repo / 'scripts/voice-worker.py')
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
with socket.socket() as reservation:
    reservation.bind(('127.0.0.1', 0))
    port = reservation.getsockname()[1]

def request(path='/api/voice', body=None, intent=True):
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=125)
    try:
        headers = {'Content-Type': 'application/json'}
        if intent:
            headers['X-Klyne-Request'] = '1'
        conn.request('GET' if body is None else 'POST', path, json.dumps(body) if body is not None else None, headers)
        reply = conn.getresponse()
        return reply.status, json.loads(reply.read())
    finally:
        conn.close()

with tempfile.TemporaryDirectory(prefix='voice-smoke-', dir=repo / 'workspace') as root:
    server = subprocess.Popen([str(repo / 'target/debug/klyne-studio.exe'), '--root', root, '--port', str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        deadline = time.monotonic() + 20
        while True:
            try:
                status, info = request()
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.1)
        assert status == 200 and info['installed'], info
        status, result = request(body={'request_id': 'health', 'operation': 'health'}, intent=False)
        assert status != 200, 'Missing intent must reject POST'
        status, result = request(body={'request_id': 'health', 'operation': 'health'})
        assert status == 200 and result['ready'], result
        status, spoken = request(body={'request_id': 'speech', 'operation': 'speak', 'text': 'Hello. The voice assistant is ready.', 'language': 'en'})
        assert status == 200 and spoken['seconds'] > 1, spoken
        status, transcript = request(body={'request_id': 'recognize', 'operation': 'transcribe', 'audio': worker.resample_fixture(spoken['audio']), 'language': 'en'})
        assert status == 200 and 'voice assistant is ready' in transcript['text'].lower(), transcript
        with concurrent.futures.ThreadPoolExecutor() as pool:
            pending = pool.submit(request, body={'request_id': 'cancel-me', 'operation': 'speak', 'text': 'This is a longer sentence to exercise cancelling active local speech synthesis. ' * 5, 'language': 'en'})
            deadline = time.monotonic() + 10
            while not request()[1]['busy']:
                assert time.monotonic() < deadline and not pending.done(), 'No cancellable operation'
                time.sleep(.01)
            status, cancelled = request('/api/voice/cancel', {'request_id': 'cancel-me'})
            assert status == 200 and cancelled['cancelled'], cancelled
            assert pending.result(timeout=15)[0] != 200
        status, result = request(body={'request_id': 'restart', 'operation': 'health'})
        assert status == 200 and result['ready'], result
        print(json.dumps({'transcript': transcript['text'], 'tts_ms': spoken['elapsed_ms'], 'asr_ms': transcript['elapsed_ms'], 'cancel_restart': 'passed', 'intent_guard': 'passed'}))
    finally:
        server.terminate()
        server.wait(timeout=15)
