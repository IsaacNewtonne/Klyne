# Voice responsiveness and concurrent conversation

Implemented on Windows, 2026-10-18. The laptop reports an NVIDIA RTX 4060 Laptop GPU with 8188 MiB VRAM.

## What changed

The composer and voice conversation can submit while a task is running. A separate durable conversation log answers questions without writing to the worker's checkpoint. Common status questions and the Progress update control read actual saved task state without calling a model. Other questions use a compact, tool-free conversational call. Explicit new tasks are logged and dispatched in order after successful completion of the current goal. Queued tasks retain the exact submitted request; they do not replace the current goal or its acceptance contract.

The queue waits on stopped/blocked work, pending approvals and unresolved effects. Dispatch is claimed durably before starting a task. After restart, an ambiguous dispatch claim is reported as interrupted rather than sent twice. Permission/workspace changes require the queued request to be reviewed and resubmitted. The last 100 conversational log entries are shown; model usage accounting reads the full log. Conversation deletion removes its auxiliary log. Changing the current action still requires Stop followed by a correction.

Speech recognition now retains its loaded model and warms during microphone startup. The default pause is 800 ms instead of 1200 ms, with 500/800/1200 ms choices. The first speech chunk is capped at 140 characters; later chunks are generated one ahead during playback. The installed-computer-voice option remains available. No new speech model download or cloud audio service was enabled.

## Measurements and tests

Synthetic English only, without microphone capture. Three direct worker runs used the sentence “Hello. The voice assistant is ready.” The saved before/after samples are included in `voice-responsive-evidence-2026-10-18.json`. Warm recognition samples changed from 1345/1406 ms to 964/794 ms. Synthesis remained variable (roughly 7.6–8.9 seconds in those warmed direct-worker samples). Cold-cache and concurrent-machine-load differences make these samples unsuitable as a universal speedup claim.

The real Studio speech smoke test separately measured 1250 ms synthesis and 828 ms recognition, reproduced the sentence, and passed cancellation/restart and HTTP intent checks. This confirms working local inference, not a microphone-to-reply latency guarantee. No live LLM conversation latency or acoustic interruption accuracy was established.

Validation commands:

The Studio run passed 120 unit tests (three ignored), 76 chat integration tests (three explicitly filtered), four performance tests and two supervisor/runtime tests (one ignored). The subsequently added cancellation regression passed in a focused three-test live-conversation run. Strict workspace Clippy, formatting and whitespace checks passed. The complete npm UI suite passed; the expanded voice test additionally verifies production-pane answers, speech prefetch during playback, and suppression of prefetched audio after Stop.

```powershell
cargo test --locked -p klyne-studio --bin klyne-studio --test chat --test perf --test runtime -- --test-threads=1 --skip production_view --skip chat_browser --skip recovery_screenshot
cargo clippy --locked --workspace --all-targets -- -D warnings
npm test
.voice/runtime/Scripts/python.exe scripts/test-voice-runtime.py
```

Windows browser/process fixtures need execution outside the restricted sandbox. The activity UI test exposed a composer-height change from showing Send and Stop together; Stop was moved into a persistent control slot and the same layout regression is rerun. Conversation replies are rendered in the production output pane as well as the normal chat view.

## Models suggested by the user

[NVIDIA Dory 4B](https://huggingface.co/nvidia/NVIDIA-Nemotron-Labs-Dory1.0-4B-BF16) is a text reasoning model with adjustable recurrent computation, not a speech recognizer or voice synthesizer. More loops spend more computation per token. Its BF16 weights alone are approximately 8 GB by parameter arithmetic, before runtime/cache memory. A supported quantized deployment could be evaluated separately; it was not installed or benchmarked here.

[Tyger-S2S's reference implementation](https://github.com/nineninesix-ai/tyger-s2s) combines Tyger turn control/recognition, Gemma 4 E4B and VoxCPM2, and documents approximately 28 GB GPU memory for its default four-conversation configuration. That configuration does not fit this 8 GB laptop. It is a relevant reference for interruption handling and sentence-at-a-time speech, but its advertised hardware results cannot be transferred to this machine. No hosted Tyger account or audio transfer was configured.

The next useful evaluation is real microphone turn latency and accuracy under concurrent model load. True streaming recognition, semantic endpointing, and token-to-audio streaming remain separate work; this release does not claim full-duplex speech.
