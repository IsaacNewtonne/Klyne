"""Opt-in local-model smoke audit; uses disposable projects, never user files."""
import argparse
import hashlib
import json
import pathlib
import socket
import subprocess
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    parser.add_argument('--endpoint', default='http://127.0.0.1:11434')
    parser.add_argument('--case', choices=['file', 'revision', 'multiple'])
    args = parser.parse_args()
    repo = pathlib.Path(__file__).resolve().parents[1]
    root = repo / 'workspace' / ('live-audit-' + str(time.time_ns()))
    root.mkdir(parents=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(path, body=None):
        request = urllib.request.Request(
            f'http://127.0.0.1:{port}{path}',
            data=json.dumps(body).encode() if body is not None else None,
            headers={'Content-Type': 'application/json', 'X-Klyne-Request': '1'},
        )
        with opener.open(request, timeout=10) as response:
            return json.load(response)

    source = 'Payday lands and the rent comes due.\nI count my coins while the night turns blue.\nThe bus runs late and my shoes wear thin.\nI clock back on and start again.\n'
    cases = [
        ('file', 'write file proof.txt :: checked', {'proof.txt': 'checked'}),
        ('revision', 'Rewrite source.txt with denser internal rhymes, more natural phrasing and less cringe. Keep the working-life theme. Save the full revised lyrics as revised.txt and preserve source.txt.', {'revised.txt': None}),
        ('multiple', 'Create alpha.txt containing exactly alpha and beta.txt containing exactly beta. Read both files to check the results.', {'alpha.txt': 'alpha', 'beta.txt': 'beta'}),
    ]
    if args.case:
        cases = [case for case in cases if case[0] == args.case]
    report = []
    with (root / 'server.log').open('w', encoding='utf-8') as log:
        child = subprocess.Popen(
            [str(repo / 'target/debug/klyne-studio.exe'), '--root', str(root / 'studio'), '--port', str(port)],
            stdout=log, stderr=log,
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
        )
        try:
            for _ in range(100):
                if child.poll() is not None:
                    raise RuntimeError('Fixture server exited; see server.log')
                try:
                    api('/api/chats')
                    break
                except (OSError, ValueError):
                    time.sleep(0.1)
            else:
                raise RuntimeError('Fixture server did not start')
            for name, instruction, expected in cases:
                project = root / name
                project.mkdir()
                (project / 'source.txt').write_text(source, encoding='utf-8')
                created = api('/api/chats', {
                    'message': instruction, 'workspace': str(project),
                    'provider': {'kind': 'ollama', 'endpoint': args.endpoint, 'model': args.model},
                    'access': {'terminal': True, 'web': False, 'apps': False, 'desktop': False},
                    'execution': {'max_steps': 32, 'timeout_seconds': 180, 'max_review_rounds': 3, 'command_policy': 'ask'},
                })
                if 'id' not in created:
                    raise RuntimeError(str(created))
                chat_id = created['id']
                deadline = time.monotonic() + 210
                while True:
                    chat = api('/api/chats/' + chat_id)
                    if chat['status'] not in ('Planning', 'Working', 'Reviewing'):
                        break
                    if time.monotonic() >= deadline:
                        api('/api/chats/' + chat_id + '/stop', {})
                        raise TimeoutError('Live case exceeded deadline: ' + name)
                    time.sleep(0.5)
                (root / (name + '-chat.json')).write_text(json.dumps(chat, indent=2), encoding='utf-8')
                checks = {}
                for filename, content in expected.items():
                    path = project / filename
                    actual = path.read_text(encoding='utf-8') if path.is_file() else ''
                    checks[filename] = actual == content if content is not None else bool(actual.strip()) and actual != source
                checks['source_preserved'] = (project / 'source.txt').read_text(encoding='utf-8') == source
                result = {'case': name, 'status': chat['status'], 'used': chat['used'], 'checks': checks,
                          'passed': chat['status'] == 'Completed' and all(checks.values()),
                          'last_message': chat['messages'][-1]['text'] if chat['messages'] else '',
                          'quality_note': 'Creative quality requires human review; file checks prove only delivery and preservation.' if name == 'revision' else ''}
                report.append(result)
                print(json.dumps(result), flush=True)
        finally:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            binary = repo / 'target/debug/klyne-studio.exe'
            (root / 'report.json').write_text(json.dumps({'model': args.model, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'results': report}, indent=2), encoding='utf-8')
            print('Evidence: ' + str(root), flush=True)
    return 0 if len(report) == len(cases) and all(r['passed'] for r in report) else 1


if __name__ == '__main__':
    raise SystemExit(main())
