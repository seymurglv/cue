# Speech on this computer: Parakeet

On 02.11.2026 the owner decided CUE moves to local speech recognition with Parakeet ("однозначно переходим"). Spec:
`docs/specs/2026-11-02-local-speech.md`.

- ONNX Runtime from Rust, on the processor. CUE ships Parakeet Ultra directly: the community int8 ONNX
  `Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx` (onnx-asr layout, CC-BY-4.0). NVIDIA's v3
  (`istupakov/parakeet-tdt-0.6b-v3-onnx`) is only the bench comparison.
- Soniox is gone completely: its key, the cloud engine choice in settings, streaming. The Anthropic key stays for
  cleanup and "Ask".
- The model downloads on first run («Модель загружается», «Модель не скачалась» + «Повторить»); it is not in the
  installer.
- The Drop shows only the voice while recording; no live text for now. Streaming may come later: keep the engine
  behind a session-style interface (frames in, finish → text).
- The rules of [language-detection.md](language-detection.md) hold.
