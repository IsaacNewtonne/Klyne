import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import klyne_recovery as recovery


class RecoveryTests(unittest.TestCase):
    def test_local_agent_corrects_empty_patch_proposals_without_cloud_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root/'source'
            file = workspace/'apps/studio/src/example.rs'
            file.parent.mkdir(parents=True)
            file.write_text('fn bug() {}')
            recovery.atomic_json(workspace/'recovery-incident.json', {'failure':{'errors':[]}})
            recovery.atomic_json(root/'runtime/recovery/config.json', {'repair_provider':{'kind':'ollama','endpoint':'http://127.0.0.1:11434','model':'local'}})
            report = root/'report';report.mkdir()
            guardian = recovery.Guardian(root,root/'runtime',root/'binary',root/'supervisor',1,2)
            prompts = []
            def local_worker(command, *args):
                prompts.append(recovery.read_json(Path(command[-2]))['prompt'])
                changes = [] if len(prompts) == 1 else [{'path':'apps/studio/src/example.rs','before':'fn bug() {}','after':'fn fixed() {}'}]
                recovery.atomic_json(Path(command[-1]), {'decision':'repair','plan':'Fix the bug','summary':'Proposed fix','reads':[],'changes':changes})
            with patch.object(recovery, 'run', side_effect=local_worker), patch.object(recovery, 'codex_command', side_effect=AssertionError('Cloud is forbidden')):
                guardian.agent(workspace,report,'Repair')
            self.assertEqual(len(prompts), 2)
            self.assertIn('validation_error', prompts[1])
            self.assertEqual(file.read_text(), 'fn fixed() {}')

    def test_local_source_inspection_finds_the_definition_in_a_bounded_excerpt(self):
        lines = ['parse_model_response(output);'] + ['// unrelated'] * 900 + ['fn parse_model_response(output: &str) {', 'reject_bad_input();', '}']
        excerpt = recovery.source_excerpt('\n'.join(lines), 'find=parse_model_response')
        self.assertIn('fn parse_model_response', excerpt['contents'])
        self.assertIn('reject_bad_input', excerpt['contents'])
        self.assertGreater(excerpt['start_line'], 890)
        self.assertLessEqual(len(excerpt['contents'].splitlines()), 100)

    def test_unavailable_local_model_keeps_repair_queued_automatically(self):
        import socket
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                port = listener.getsockname()[1]
            guardian = recovery.Guardian(root, root, root/'binary', root/'supervisor', 4317, 4318)
            recovery.atomic_json(root/'recovery/config.json', {'enabled':True,'repair_provider':{'kind':'ollama','endpoint':f'http://127.0.0.1:{port}','model':'local'}})
            incident = root/'recovery/incidents/123.json'
            recovery.atomic_json(incident, {'chat_id':'123','failure':{}})
            with patch.object(guardian, 'start_runtime'), patch.object(guardian, 'health', return_value={'ready':True}), patch.object(guardian.shutdown, 'wait', side_effect=[False, False, True]):
                guardian._monitor()
            self.assertEqual(guardian.state['phase'], 'Waiting for local model')
            self.assertTrue(incident.exists())
            self.assertFalse((root/'recovery/reports/123').exists())
            self.assertGreater(guardian.next_model_probe, time.monotonic())

    def test_local_repair_request_uses_native_json_and_vision(self):
        import http.server
        captured = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                captured.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                data = json.dumps({'message':{'content':json.dumps({'decision':'inspect','reads':['apps/studio/src/chat.rs']})}}).encode()
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, *_): pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root/'screen.png').write_bytes(b'fixture image')
                recovery.atomic_json(root/'request.json', {'provider':{'kind':'ollama','endpoint':f'http://127.0.0.1:{server.server_port}','model':'local'},'schema':{'type':'object'},'prompt':'diagnose','image':str(root/'screen.png')})
                with patch.object(recovery, 'codex_command', side_effect=AssertionError('Cloud fallback is forbidden')):
                    recovery.ollama_request(root/'request.json', root/'response.json')
                self.assertEqual(captured[0]['format'], {'type':'object'})
                self.assertFalse(captured[0]['think'])
                self.assertFalse(captured[0]['stream'])
                self.assertIn('images', captured[0]['messages'][0])
                self.assertEqual(recovery.read_json(root/'response.json')['decision'], 'inspect')
        finally:
            server.shutdown()
            server.server_close()

    def test_local_repair_rejects_remote_servers_and_missing_models(self):
        for endpoint in ('https://example.com', 'http://example.com', 'http://user@localhost:11434', 'http://localhost:11434/redirect'):
            with self.assertRaises(RuntimeError):
                recovery.local_endpoint({'endpoint':endpoint,'model':'local'})
        with self.assertRaises(RuntimeError):
            recovery.local_endpoint({})

    def test_patch_proposals_are_validated_before_any_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "apps/studio/src/example.rs"
            source.parent.mkdir(parents=True)
            source.write_text("fn bug() {}")
            patches = [{"path":"apps/studio/src/example.rs", "before":"fn bug() {}", "after":"fn fixed() {}"},
                       {"path":"Cargo.toml", "before":"", "after":"invalid"}]
            with self.assertRaises(RuntimeError):
                recovery.apply_changes(root, patches)
            self.assertEqual(source.read_text(), "fn bug() {}")
            recovery.apply_changes(root, patches[:1])
            self.assertEqual(source.read_text(), "fn fixed() {}")
            with self.assertRaisesRegex(RuntimeError, "missing or ambiguous"):
                recovery.apply_changes(root, patches[:1])

    @unittest.skipUnless(os.environ.get("KLYNE_TEST_BINARY"), "Set KLYNE_TEST_BINARY for the live watchdog test")
    def test_independent_dashboard_survives_a_runtime_crash_and_restores_service(self):
        import socket
        def free_port():
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                return listener.getsockname()[1]
        with tempfile.TemporaryDirectory() as directory:
            port, recovery_port = free_port(), free_port()
            root = Path(directory)
            binary = Path(os.environ["KLYNE_TEST_BINARY"]).resolve()
            supervisor = binary.with_name("klyne-supervisor" + binary.suffix)
            with (root / "guardian.log").open("wb") as log:
                child = recovery.spawn([sys.executable, Path(recovery.__file__).resolve(),
                    "--source", Path(__file__).resolve().parent.parent, "--root", root,
                    "--binary", binary, "--supervisor", supervisor,
                    "--port", port, "--recovery-port", recovery_port], stdout=log, stderr=log)
            def wait_for_new_pid(previous=None):
                deadline = time.monotonic() + 35
                while time.monotonic() < deadline:
                    try:
                        health = recovery.api(port, "/api/runtime/ready", timeout=.5)
                        if health.get("pid") != previous:
                            return health["pid"]
                    except (OSError, ValueError):
                        pass
                    time.sleep(.2)
                self.fail((root / "guardian.log").read_text())
            try:
                pid = wait_for_new_pid()
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True, check=True)
                else:
                    import signal
                    os.kill(pid, signal.SIGKILL)
                status = recovery.api(recovery_port, "/status")
                self.assertIn("phase", status)
                self.assertNotEqual(wait_for_new_pid(pid), pid)
                recovery.api(recovery_port, "/stop", {})
                self.assertTrue((root / "recovery/paused.json").exists())
            finally:
                recovery.terminate(child)

    def test_uncertain_stopped_and_changed_conversations_are_not_replayed(self):
        chat = {"id":"123-0", "status":"Blocked", "provider":{}, "access":{}, "used":2, "messages":[{}, {}]}
        incident = {"failure":{"request_step":2,"messages_count":1}}
        recovery.unchanged_chat(chat, incident)
        for change in ({"pending":{"tool":"write_file"}}, {"status":"Stopped"}, {"used":3}, {"messages":[{}]}):
            with self.assertRaises(RuntimeError):
                recovery.unchanged_chat(dict(chat, **change), incident)

    def test_command_timeout_and_stop_terminate_the_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for cancelled in (False, True):
                stop = threading.Event()
                if cancelled:
                    stop.set()
                start = time.monotonic()
                with self.assertRaises(RuntimeError):
                    recovery.run([sys.executable, "-c", "import time;time.sleep(30)"], root,
                                 root / "process.log", stop, .2)
                self.assertLess(time.monotonic() - start, 6)

    def test_snapshot_preserves_dirty_and_untracked_source_but_not_live_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "original"
            source.mkdir()
            subprocess.run(["git", "init", str(source)], capture_output=True, check=True)
            (source / "Cargo.toml").write_text("original")
            subprocess.run(["git", "add", "Cargo.toml"], cwd=source, check=True)
            (source / "Cargo.toml").write_text("dirty")
            (source / "apps").mkdir()
            (source / "apps/new.rs").write_text("new file")
            (source / "apps/.env").write_text("private")
            (source / "workspace").mkdir()
            (source / "workspace/secret").write_text("private")
            recovery.source_snapshot(source, root / "snapshot")
            self.assertEqual((root / "snapshot/Cargo.toml").read_text(), "dirty")
            self.assertEqual((root / "snapshot/apps/new.rs").read_text(), "new file")
            self.assertFalse((root / "snapshot/apps/.env").exists())
            self.assertFalse((root / "snapshot/workspace").exists())

    def test_repair_cannot_change_its_acceptance_tests_or_supervisor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / "apps/studio/tests/existing.rs"
            file.parent.mkdir(parents=True)
            file.write_text("original assertion")
            baseline = {"apps/studio/tests/existing.rs":hashlib.sha256(file.read_bytes()).hexdigest()}
            file.write_text("weakened assertion")
            with self.assertRaisesRegex(RuntimeError, "protected"):
                recovery.validate_changes(root, baseline)
            file.write_text("original assertion")
            new = root / "apps/studio/tests/regression.rs"
            new.write_text("new regression")
            self.assertEqual(recovery.validate_changes(root, baseline), ["apps/studio/tests/regression.rs"])
            supervisor = root / "apps/studio/src/bin/klyne-supervisor.rs"
            supervisor.parent.mkdir(parents=True)
            supervisor.write_text("bypass gates")
            with self.assertRaisesRegex(RuntimeError, "protected"):
                recovery.validate_changes(root, baseline)

    def test_restart_budget_persists_across_guardian_restarts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            recovery.atomic_json(root / "recovery/restarts.json", [time.time()] * 3)
            guardian = recovery.Guardian(root, root, root / "binary", root / "supervisor", 4317, 4318)
            with patch.object(recovery, "spawn") as spawn:
                self.assertFalse(guardian.start_runtime())
                spawn.assert_not_called()
            self.assertEqual(guardian.state["phase"], "Needs attention")

    def test_disabled_repairs_save_diagnostics_without_starting_an_agent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            guardian = recovery.Guardian(root, root, root / "binary", root / "supervisor", 4317, 4318)
            incident = root / "recovery/incidents/123-0.json"
            recovery.atomic_json(incident, {"chat_id":"123", "failure":{"pending":None}})
            with patch.object(guardian, "capture", return_value="no browser"), patch.object(guardian, "agent") as agent:
                guardian.process_incident(incident)
                guardian.process_incident(incident)
                agent.assert_not_called()
            result = recovery.read_json(root / "recovery/reports/123-0/result.json")
            self.assertIn("disabled", result["error"])

    def test_path_traversal_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(RuntimeError):
                recovery.safe_relative(Path(directory), "../outside")


if __name__ == "__main__":
    unittest.main()
