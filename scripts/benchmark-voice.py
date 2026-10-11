"""Local synthetic smoke benchmark; no microphone and no network access."""
import importlib.util
import json
from pathlib import Path
import socket
import time

def offline(*args, **kwargs):
    raise RuntimeError('Benchmark must not access the network')

socket.socket.connect = offline
socket.socket.connect_ex = offline
spec = importlib.util.spec_from_file_location('voice_worker', Path(__file__).with_name('voice-worker.py'))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
home = Path(__file__).resolve().parent.parent / '.voice'
speech = worker.Speech(home)
results = {}
for language, text in [('en', 'Hello. The voice assistant is ready.')]:
    runs = []
    for index in range(3):
        started = time.perf_counter()
        reply = speech.handle({'operation': 'speak', 'text': text, 'language': language})
        tts_ms = round((time.perf_counter() - started) * 1000)
        fixture = worker.resample_fixture(reply['audio'])
        started = time.perf_counter()
        transcript = speech.handle({'operation': 'transcribe', 'audio': fixture, 'language': language})
        runs.append({'tts_ms': tts_ms, 'asr_ms': round((time.perf_counter() - started) * 1000), 'audio_seconds': reply['seconds'], 'transcript': transcript['text']})
        print(json.dumps({'language': language, 'run': index, **runs[-1]}), flush=True)
    results[language] = runs
assert 'torch' not in __import__('sys').modules
(home / 'benchmark.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
