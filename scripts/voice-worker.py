"""Private JSON-lines speech worker. No listener, credentials, or audio files.

Rust owns its lifetime; stdout is protocol-only. Setup downloads models once.
Runtime is offline and uses an isolated cache. Speech recognition and synthesis
are serialized so multiple tabs cannot allocate duplicate models.
"""
import argparse
import base64
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import time
import wave

# Avoid separate full-CPU ONNX pools for each small model component.
# Supported by Moonshine's native ort_maybe_force_single_thread helper.
os.environ['MOONSHINE_ORT_SINGLE_THREAD'] = '1'

MAX_LINE = 1_500_000
MAX_SECONDS = 30
LANGUAGES = ('en', 'vi')


def decode_wav(encoded):
    raw = base64.b64decode(encoded, validate=True)
    if len(raw) > 960_044:
        raise ValueError('Recording exceeds 30 seconds')
    with wave.open(io.BytesIO(raw)) as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) != (1, 2, 16000):
            raise ValueError('Expected mono 16 kHz PCM16 WAV')
        if not 1600 <= wav.getnframes() <= MAX_SECONDS * 16000:
            raise ValueError('Record between 0.1 and 30 seconds')
        data = wav.readframes(wav.getnframes())
        if len(data) != wav.getnframes() * 2:
            raise ValueError('Incomplete recording')
    import numpy as np
    return np.frombuffer(data, dtype='<i2').astype('float32') / 32768


class Speech:
    def __init__(self, home):
        self.home = home
        self.asr = self.tts = None
        self.asr_language = self.tts_language = None
        self.manifest = json.loads((home / 'models.json').read_text(encoding='utf-8'))

    def load_asr(self, language):
        if self.asr_language != language:
            from moonshine_voice import Transcriber, ModelArch
            if self.asr is not None:
                self.asr.close()
            self.asr = None
            self.asr_language = None
            self.asr = Transcriber(self.home / self.manifest['asr'][language], ModelArch.TINY_STREAMING)
            self.asr_language = language

    def load_tts(self, language):
        if self.tts_language != language:
            import onnxruntime as ort
            from kokoro_onnx import Kokoro
            options = ort.SessionOptions()
            options.intra_op_num_threads = 2
            options.inter_op_num_threads = 1
            options.add_session_config_entry('session.intra_op.allow_spinning', '0')
            options.add_session_config_entry('session.set_denormal_as_zero', '1')
            session = ort.InferenceSession(str(self.home / 'kokoro/kokoro-v1.0.int8.onnx'), sess_options=options, providers=['CPUExecutionProvider'])
            self.tts = Kokoro.from_session(session, str(self.home / 'kokoro/voices-v1.0.bin'))
            self.tts_language = language

    def release_asr(self):
        if self.asr is not None:
            self.asr.close()
        self.asr = None
        self.asr_language = None

    def handle(self, request):
        kind = request.get('operation')
        if kind == 'health':
            self.load_asr('en')
            self.release_asr()
            self.load_tts('en')
            return {'ready': True, 'asr': 'Moonshine Tiny Streaming 34M', 'tts': 'Kokoro 82M / George / English', 'local': True}
        if kind == 'transcribe':
            samples = decode_wav(request.get('audio', ''))
            language = request.get('language', 'en')
            if language not in LANGUAGES:
                raise ValueError('Unsupported recognition language')
            self.load_asr(language)
            try:
                transcript = self.asr.transcribe_without_streaming(samples, 16000)
                text = ' '.join(line.text.strip() for line in transcript.lines).strip()
            finally:
                # The native recognizer owns several ORT sessions. Release them
                # between turns so their pools do not compete with synthesis.
                self.release_asr()
            return {'text': text[:8000], 'language': language}
        if kind == 'speak':
            text = request.get('text')
            if not isinstance(text, str) or not text.strip() or len(text) > 450:
                raise ValueError('Speech chunks must contain 1–450 characters')
            language = request.get('language', 'en')
            if language != 'en':
                raise ValueError('Use an installed computer voice for Vietnamese speech')
            self.load_tts(language)
            import numpy as np
            audio, rate = self.tts.create(text, voice='bm_george', lang='en-gb')
            audio = np.asarray(audio, dtype='float32')
            if audio.size > rate * 60:
                raise ValueError('Generated audio exceeds one minute')
            data = (np.clip(audio, -1, 1) * 32767).astype('<i2').tobytes()
            out = io.BytesIO()
            with wave.open(out, 'wb') as wav:
                wav.setnchannels(1)
                wav.setsampwidth(2)
                wav.setframerate(rate)
                wav.writeframes(data)
            return {'audio': base64.b64encode(out.getvalue()).decode('ascii'), 'mime': 'audio/wav', 'seconds': audio.size / rate}
        raise ValueError('Unknown voice operation')


def setup(home):
    from moonshine_voice import ModelArch
    from moonshine_voice.download import get_model_for_language
    home = home.resolve()
    # Never leave a stale ready marker after an interrupted upgrade.
    (home / 'ready.json').unlink(missing_ok=True)
    manifest = {'asr': {}}
    for language in LANGUAGES:
        model, _ = get_model_for_language(language, ModelArch.TINY_STREAMING, cache_root=home / 'models')
        manifest['asr'][language] = str(Path(model).relative_to(home))
    for filename, size in [('kokoro-v1.0.int8.onnx', 92361271), ('voices-v1.0.bin', 28214398)]:
        download_asset(home / 'kokoro' / filename, filename, size)
    (home / 'models.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    speech = Speech(home)
    # Exercise the real runtime before declaring the installation ready.
    results = {}
    for language, text in [('en', 'Hello. The voice assistant is ready.')]:
        started = time.monotonic()
        reply = speech.handle({'operation': 'speak', 'text': text, 'language': language})
        tts_ms = round((time.monotonic() - started) * 1000)
        started = time.monotonic()
        recognized = speech.handle({'operation': 'transcribe', 'audio': resample_fixture(reply['audio']), 'language': language})
        import re
        if re.sub(r'[^a-z ]', '', recognized['text'].lower()).strip() != 'hello the voice assistant is ready':
            raise RuntimeError('Speech round trip did not reproduce the test sentence')
        results[language] = {'transcript': recognized['text'], 'tts_cold_ms': tts_ms, 'asr_cold_ms': round((time.monotonic() - started) * 1000), 'audio_seconds': reply['seconds']}
    import importlib.metadata
    ready = {'version': 3, 'asr': 'moonshine-tiny-streaming-34m', 'tts': 'kokoro-82m-int8-bm_george', 'packages': {p: importlib.metadata.version(p) for p in ('moonshine-voice', 'kokoro-onnx', 'onnxruntime')}, 'round_trip': results, 'vietnamese_recognition': 'available; not accuracy validated', 'vietnamese_synthesis': 'use an installed computer voice'}
    (home / 'ready.json').write_text(json.dumps(ready, indent=2), encoding='utf-8')
    print(json.dumps(ready))


def download_asset(target, filename, size):
    import requests
    if target.is_file() and target.stat().st_size == size:
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix('.part')
    url = 'https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/' + filename
    with requests.get(url, stream=True, timeout=(15, 60)) as response:
        response.raise_for_status()
        count = 0
        with temporary.open('wb') as output:
            for chunk in response.iter_content(1024 * 1024):
                count += len(chunk)
                if count > size:
                    raise ValueError('Unexpected model size')
                output.write(chunk)
    if count != size:
        raise ValueError('Incomplete model download')
    temporary.replace(target)


def resample_fixture(encoded):
    import numpy as np
    with wave.open(io.BytesIO(base64.b64decode(encoded))) as wav:
        rate = wav.getframerate()
        samples = np.frombuffer(wav.readframes(wav.getnframes()), dtype='<i2').astype('float32')
    # Setup fixture only; microphone audio already arrives at 16 kHz.
    positions = np.arange(round(len(samples) * 16000 / rate)) * rate / 16000
    pcm = np.clip(np.interp(positions, np.arange(len(samples)), samples), -32768, 32767).astype('<i2')
    out = io.BytesIO()
    with wave.open(out, 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(16000)
        wav.writeframes(pcm.tobytes())
    return base64.b64encode(out.getvalue()).decode('ascii')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--home', type=Path, required=True)
    parser.add_argument('--setup', action='store_true')
    args = parser.parse_args()
    os.environ['HF_HOME'] = str(args.home / 'cache')
    os.environ['HF_HUB_DISABLE_TELEMETRY'] = '1'
    os.environ['HF_HUB_DISABLE_PROGRESS_BARS'] = '1'
    if args.setup:
        args.home.mkdir(parents=True, exist_ok=True)
        setup(args.home)
        return
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    speech = Speech(args.home)
    while True:
        line = sys.stdin.buffer.readline(MAX_LINE + 1)
        if not line:
            break
        if len(line) > MAX_LINE or not line.endswith(b'\n'):
            break
        started = time.monotonic()
        try:
            request = json.loads(line)
            with contextlib.redirect_stdout(sys.stderr):
                result = speech.handle(request)
            result['elapsed_ms'] = round((time.monotonic() - started) * 1000)
        except Exception as error:
            # Avoid emitting transcripts, buffers, paths or environment values in errors.
            result = {'error': f'Local speech failed ({type(error).__name__}). Check voice setup or retry.'}
        print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
