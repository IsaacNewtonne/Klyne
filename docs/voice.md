# Local voice in Klyne

Klyne can accept dictation, send spoken conversation turns, read replies aloud, and hand action requests to its existing task engine. The local speech stack is Moonshine Tiny Streaming (34M parameters) and quantized Kokoro (82M), with no PyTorch or GPU requirement. This is an initial voice implementation, not a completed always-listening Jarvis system.

## Use

With Python 3.12 installed, run `powershell -ExecutionPolicy Bypass -File scripts/setup-voice.ps1` once. Setup downloads public model assets and verifies a synthetic English speech round trip; it never opens the microphone. The runtime uses explicit local model paths and does not download models during conversation. Existing installations with an activated runtime need the update procedure below.

In **Settings > Voice**, choose **Check voice** to load and check the engines, then **Test speaking voice**. English recognition and Kokoro's George voice are the defaults. You can instead select a locally installed computer voice.

- **Talk**, then **Finish**: dictate into the composer; review/edit before pressing Send.
- **Conversation**: opt into repeated listening and automatic submission after a pause.
- **Stop voice** or Escape: stop listening and discard pending speech. This does not stop an app task.
- **Alt+Space**: toggle microphone capture.
- With automatic sending enabled, “stop the task” uses the existing task Stop control; “stop listening” ends the voice session; “task status” reads the observed task state.
- You can keep typing or speaking while a task runs. **Chat or add a task** answers questions or logs new tasks; **Ask only** cannot enqueue work; **Queue a task** logs the exact request without a model call. **Progress update** reports the current goal, step, latest observation and queue directly from saved state. Replies and queue states appear in both conversation and production views.
- Follow-up tasks start in order after the current goal completes. Stop, blocked work, unresolved effects and pending approvals keep the queue waiting. Cancel an unwanted queued task in its log entry. To change the current task, stop it and submit the correction; a conversational reply does not apply a mid-action correction.
- **Pause before sending** offers 0.5, 0.8 (default), and 1.2 seconds. The shorter setting may cut off thoughtful pauses. Recognition warms while the microphone opens and stays loaded between turns. Speech uses a shorter first chunk and prepares one subsequent chunk during playback.
- **Interrupt by speaking** is optional and off by default. Use headphones: the browser's echo cancellation plus the current energy threshold is not a tested full-duplex acoustic system.

Microphone access begins only after the user starts it and the browser permits it. Recordings and synthesized audio are held in memory. Transcripts become ordinary persisted chat messages. The configured AI connection still determines where conversation text is processed. Voice does not grant tools or bypass existing approvals.

## Languages and limitations

English and Vietnamese use separate recognition checkpoints; select the intended language. Vietnamese recognition is marked **preview** because we have not evaluated it against representative recorded Vietnamese speech. Automatic language detection and mixed-language recognition are not implemented.

The tested neural speaking voice is English. A trial of Moonshine's bundled Piper Vietnamese path did not reproduce the test sentence, so that voice is not exposed in Klyne. Select an installed Vietnamese computer voice for Vietnamese replies if one is available.

The model supports streaming, but Klyne currently submits a completed recording to it. Conversation endpointing uses a configurable silence threshold, not semantic turn detection. Replies are spoken in bounded, prefetched chunks after the model finishes its response; token-by-token speech, a wake phrase, and acoustic echo testing remain future work. A speech model alone does not provide agent memory, planning, or self-maintenance.

The Windows runtime and model assets occupy approximately 366 MiB with the tested package versions. Neural speech can take several seconds; low latency and accuracy across microphones and accents are not established. Installed computer voices provide another output option.

## Updating an existing runtime

The supervisor preserves the last activated binary under `workspace/studio/runtime/`. Building a new executable does not replace that saved version. After voice setup, run from the checkout:

```powershell
cargo build --locked -p klyne-studio --bins
$voicePython = (Resolve-Path .voice/runtime/Scripts/python.exe).Path
.\target\debug\klyne-studio.exe --stage-runtime workspace/studio target/debug/klyne-studio.exe . -- $voicePython scripts/test-voice-runtime.py
```

The command tests the candidate before staging it. A running supervisor activates it when idle and rolls back if startup fails. Check `workspace/studio/runtime/last-result.json` for `activated` with `rollback: false`; staging alone does not confirm activation. With Klyne open, press **Ctrl+R** after activation. **🎙 Talk** appears above the composer and **Voice** appears in Settings. If the supervisor is stopped, launch Klyne to process the staged update.

## Implementation and checks

- Rust owns one on-demand Python speech worker over bounded private JSON-lines stdio. There is no separate speech listener or cloud account.
- Requests have IDs, timeouts, recording/text size caps, and explicit cancellation. A Windows job owns the process lifetime. Restarting inference is safe after cancellation.
- Ordinary voice conversation uses one constrained model call with recent conversation context. Requests requiring tools hand off to the normal task engine. Task history and provider configuration remain shared.
- Concurrent conversation uses a separate SQLite log (`live-chat.sqlite3`), so a reply cannot overwrite the active worker checkpoint. Each submission has an idempotency key. Queue dispatch is recorded before admission; an interrupted dispatch claim is never blindly replayed. Ordinary questions use one bounded-context, tool-free call to the conversation's provider with a 30-second cancellation deadline. Status and explicit queuing need no inference. Model token/cost usage is logged and checked against the configured token/cost limits before a call; in-flight usage can overshoot, as with task calls. These replies can still wait behind a busy single-request model server.
- `node scripts/test-voice.cjs` checks dictation, opt-in submission, single reply playback, interruption, microphone release, and mobile controls using mocked microphone/audio APIs.
- Chat integration tests cover status during a delayed worker call, durable ordered follow-ups, duplicate submission, read-only questions and queued cancellation. See [the responsiveness report](voice-responsive-2026-10-18.md) for model research and measured limits.
- `.voice/runtime/Scripts/python.exe scripts/test-voice-runtime.py` exercises real Studio synthesis, recognition, cancellation, worker restart, and HTTP intent checks, without microphone access.
- `.voice/runtime/Scripts/python.exe scripts/benchmark-voice.py` repeats synthetic inference with Python network connections disabled. It writes only timings/transcripts to `.voice/benchmark.json`.

## Dependencies

[Moonshine model list](https://moonshine-voice.readthedocs.io/en/latest/models/available-models/) documents Tiny 34M and Small 123M streaming models; [accuracy notes](https://moonshine-voice.readthedocs.io/en/latest/models/accuracy/) distinguish reference and quantized results. [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) provides Apache-licensed 82M weights; [kokoro-onnx](https://github.com/thewh1teagle/kokoro-onnx) provides the Windows-compatible inference wrapper. Its bundled phonemizer/espeak dependencies have their own license notices.
